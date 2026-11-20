//! Canal local de Discord : trames `[op: i32 LE][longueur: i32 LE][JSON]`, sur un socket Unix ou un tube nommé.
//!
//! Le dialogue se fait « en lockstep » : on écrit une commande puis on lit sa réponse, sans thread de lecture.
//! Sous Windows, un tube ouvert en E/S synchrones sérialise lectures et écritures : une lecture en attente sur
//! un autre thread bloquerait chaque écriture.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;

pub const OP_HANDSHAKE: i32 = 0;
pub const OP_FRAME: i32 = 1;
pub const OP_CLOSE: i32 = 2;
pub const OP_PING: i32 = 3;
pub const OP_PONG: i32 = 4;

/// Taille maximale d'une trame acceptée (au-delà, le flux est jugé incohérent).
const MAX_FRAME: usize = 1 << 20;

/// Une trame : (op, contenu JSON ; `None` s'il est illisible).
pub type Frame = (i32, Option<Value>);

/// Emballe une trame.
pub fn encode(op: i32, data: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(data).unwrap_or_else(|_| b"{}".to_vec());
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(body.len() as i32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Découpe le flux en trames (un message peut arriver en plusieurs morceaux, ou plusieurs à la fois).
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Frame> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while self.buf.len() >= 8 {
            let op = i32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
            let len = i32::from_le_bytes([self.buf[4], self.buf[5], self.buf[6], self.buf[7]]);
            if len < 0 || len as usize > MAX_FRAME {
                self.buf.clear();
                break;
            }
            let len = len as usize;
            if self.buf.len() < 8 + len {
                break;
            }
            let data = serde_json::from_slice::<Value>(&self.buf[8..8 + len]).ok();
            self.buf.drain(..8 + len);
            out.push((op, data));
        }
        out
    }
}

/// Les emplacements possibles du canal de Discord (Discord, Canary, PTB, Snap, Flatpak, Vesktop…).
/// `env` lit une variable d'environnement ; `override_dir` remplace la recherche (comme `CONTROL_DISCORD_IPC_DIR`).
pub fn pipe_paths(windows: bool, env: &dyn Fn(&str) -> Option<String>, override_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if windows && override_dir.is_none() {
        for i in 0..10 {
            out.push(PathBuf::from(format!(r"\\.\pipe\discord-ipc-{i}")));
        }
        return out;
    }
    let forced = override_dir.map(Path::to_path_buf).or_else(|| env("CONTROL_DISCORD_IPC_DIR").map(PathBuf::from));
    let base = forced.clone().unwrap_or_else(|| {
        ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
            .iter()
            .find_map(|k| env(k).filter(|v| !v.is_empty()))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"))
    });
    let mut dirs = vec![base.clone()];
    if forced.is_none() {
        dirs.push(base.join("snap.discord"));
        for app in ["com.discordapp.Discord", "com.discordapp.DiscordCanary", "dev.vencord.Vesktop"] {
            dirs.push(base.join("app").join(app));
        }
    }
    for dir in dirs {
        for i in 0..10 {
            out.push(dir.join(format!("discord-ipc-{i}")));
        }
    }
    out
}

#[cfg(unix)]
type Stream = std::os::unix::net::UnixStream;
#[cfg(windows)]
type Stream = std::fs::File;

/// Connexion ouverte au client Discord.
pub struct Conn {
    stream: Stream,
    decoder: Decoder,
    frames: VecDeque<Frame>,
}

impl Conn {
    /// Ouvre le canal à cet emplacement (erreur s'il n'existe pas ou si personne n'écoute).
    pub fn open(path: &Path) -> io::Result<Conn> {
        #[cfg(unix)]
        let stream = {
            if !path.exists() {
                return Err(io::Error::from(io::ErrorKind::NotFound));
            }
            let s = Stream::connect(path)?;
            s.set_write_timeout(Some(Duration::from_secs(3)))?;
            s
        };
        #[cfg(windows)]
        let stream = std::fs::OpenOptions::new().read(true).write(true).open(path)?;
        Ok(Conn { stream, decoder: Decoder::default(), frames: VecDeque::new() })
    }

    pub fn write_frame(&mut self, op: i32, data: &Value) -> io::Result<()> {
        self.stream.write_all(&encode(op, data))?;
        self.stream.flush()
    }

    /// Lit la trame suivante. `Ok(None)` : rien avant `timeout` (sous Windows, la lecture attend sans limite).
    /// Une erreur veut dire que Discord a fermé le canal.
    pub fn read_frame(&mut self, timeout: Duration) -> io::Result<Option<Frame>> {
        let deadline = Instant::now() + timeout;
        let mut buf = [0u8; 8192];
        loop {
            if let Some(f) = self.frames.pop_front() {
                return Ok(Some(f));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            #[cfg(unix)]
            self.stream.set_read_timeout(Some(left.max(Duration::from_millis(1))))?;
            match self.stream.read(&mut buf) {
                Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
                Ok(n) => {
                    let frames = self.decoder.push(&buf[..n]);
                    self.frames.extend(frames);
                }
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => return Ok(None),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    /// Vérifie sans attendre que le canal est encore ouvert, et renvoie les trames arrivées entre-temps
    /// (un PING, par exemple). Sous Windows, rien à faire sans bloquer : la fermeture se voit à l'envoi suivant.
    pub fn poll(&mut self) -> io::Result<Vec<Frame>> {
        #[cfg(unix)]
        {
            self.stream.set_nonblocking(true)?;
            let mut buf = [0u8; 8192];
            let result = loop {
                match self.stream.read(&mut buf) {
                    Ok(0) => break Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
                    Ok(n) => {
                        let frames = self.decoder.push(&buf[..n]);
                        self.frames.extend(frames);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break Ok(()),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => break Err(e),
                }
            };
            self.stream.set_nonblocking(false)?;
            result?;
        }
        Ok(self.frames.drain(..).collect())
    }
}

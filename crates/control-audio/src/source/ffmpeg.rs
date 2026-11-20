//! Décodage par ffmpeg, pour ce que symphonia ne lit pas (Opus, audio des vidéos webm/mkv/mp4/avi…).
//!
//! ffmpeg écrit du PCM flottant stéréo, déjà à la fréquence de la sortie, sur sa sortie standard ; un thread le lit
//! et le passe par paquets au décodeur. Un déplacement relance ffmpeg avec `-ss`.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use crossbeam_channel::{Receiver, RecvTimeoutError, TryRecvError, bounded};
use parking_lot::Mutex;

use super::{Chunk, Decoder};

/// Trames par paquet lu sur la sortie de ffmpeg.
const PACKET_FRAMES: usize = 4096;
/// Paquets d'avance (≈ 3 s à 44,1 kHz) : ffmpeg attend ensuite que la lecture les consomme.
const QUEUE: usize = 32;
/// Attente maximale du premier paquet à l'ouverture.
const OPEN_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Default)]
struct Shared {
    duration: Option<f64>,
    /// Dernières lignes de ffmpeg, pour un message d'erreur utile.
    tail: Vec<String>,
}

pub(crate) struct FfmpegDecoder {
    exe: PathBuf,
    path: PathBuf,
    rate: u32,
    child: Option<Child>,
    rx: Receiver<Vec<f32>>,
    shared: Arc<Mutex<Shared>>,
    primed: Vec<f32>,
    received: u64,
}

impl FfmpegDecoder {
    pub fn open(exe: &Path, path: &Path, rate: u32) -> Result<FfmpegDecoder> {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (child, rx) = spawn(exe, path, rate, 0.0, &shared)?;
        let mut dec = FfmpegDecoder {
            exe: exe.to_path_buf(),
            path: path.to_path_buf(),
            rate,
            child: Some(child),
            rx,
            shared,
            primed: Vec::new(),
            received: 0,
        };
        // On attend le premier paquet : il confirme que ffmpeg sait lire le fichier (et la durée est alors connue).
        match dec.rx.recv_timeout(OPEN_TIMEOUT) {
            Ok(first) => {
                dec.received += first.len() as u64 / 2;
                dec.primed = first;
            }
            Err(RecvTimeoutError::Timeout) => return Err(anyhow!("ffmpeg ne répond pas")),
            Err(RecvTimeoutError::Disconnected) => return Err(dec.failure()),
        }
        for _ in 0..20 {
            if dec.shared.lock().duration.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(dec)
    }

    fn failure(&mut self) -> anyhow::Error {
        if let Some(child) = self.child.as_mut() {
            let _ = child.wait();
        }
        let tail = self.shared.lock().tail.clone();
        let line = tail.iter().rev().find(|l| !l.trim().is_empty()).cloned().unwrap_or_default();
        if line.is_empty() { anyhow!("ffmpeg n'a rien pu lire") } else { anyhow!("ffmpeg : {line}") }
    }

    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for FfmpegDecoder {
    fn drop(&mut self) {
        self.kill();
    }
}

fn spawn(
    exe: &Path,
    path: &Path,
    rate: u32,
    start: f64,
    shared: &Arc<Mutex<Shared>>,
) -> Result<(Child, Receiver<Vec<f32>>)> {
    let mut cmd = Command::new(exe);
    cmd.arg("-hide_banner").arg("-nostdin");
    if start > 0.0 {
        cmd.arg("-ss").arg(format!("{start:.6}"));
    }
    cmd.arg("-i")
        .arg(path)
        .args(["-map", "0:a:0", "-vn", "-sn", "-dn", "-f", "f32le", "-ac", "2", "-ar"])
        .arg(rate.to_string())
        .arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    control_core::process::hide_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| anyhow!("Impossible de lancer ffmpeg ({}) : {e}", exe.display()))?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow!("ffmpeg : pas de sortie"))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("ffmpeg : pas de sortie d'erreur"))?;
    let (tx, rx) = bounded::<Vec<f32>>(QUEUE);
    std::thread::Builder::new().name("control-audio-ffmpeg".into()).spawn(move || {
        let mut reader = BufReader::with_capacity(PACKET_FRAMES * 8, stdout);
        let mut bytes = vec![0u8; PACKET_FRAMES * 8];
        loop {
            // Lit un paquet entier (ou ce qui reste à la fin), en trames complètes.
            let mut filled = 0;
            while filled < bytes.len() {
                match reader.read(&mut bytes[filled..]) {
                    Ok(0) => break,
                    Ok(n) => filled += n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            let usable = filled - filled % 8;
            if usable > 0 {
                let samples: Vec<f32> =
                    bytes[..usable].as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
                if tx.send(samples).is_err() {
                    return;
                }
            }
            if filled < bytes.len() {
                return;
            }
        }
    })?;
    let shared = Arc::clone(shared);
    std::thread::Builder::new().name("control-audio-ffmpeg-log".into()).spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.split(b'\n').map_while(|l| l.ok()) {
            let line = String::from_utf8_lossy(&line).trim_end().to_string();
            let mut s = shared.lock();
            if s.duration.is_none()
                && let Some(d) = parse_duration(&line)
            {
                s.duration = Some(d);
            }
            if s.tail.len() >= 8 {
                s.tail.remove(0);
            }
            s.tail.push(line);
        }
    })?;
    Ok((child, rx))
}

/// Durée annoncée par ffmpeg (« Duration: 00:03:25.12, start: … »).
pub(crate) fn parse_duration(line: &str) -> Option<f64> {
    let rest = line.trim_start().strip_prefix("Duration:")?.trim_start();
    let stamp = rest.split(',').next()?.trim();
    let mut parts = stamp.split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    let d = h * 3600.0 + m * 60.0 + s;
    (d > 0.0).then_some(d)
}

impl Decoder for FfmpegDecoder {
    fn next_chunk(&mut self, out: &mut Vec<f32>) -> Result<Chunk> {
        if !self.primed.is_empty() {
            out.append(&mut self.primed);
            return Ok(Chunk::Data);
        }
        match self.rx.try_recv() {
            Ok(mut data) => {
                self.received += data.len() as u64 / 2;
                out.append(&mut data);
                Ok(Chunk::Data)
            }
            Err(TryRecvError::Empty) => Ok(Chunk::Pending),
            Err(TryRecvError::Disconnected) => {
                // Fin normale, ou échec avant le moindre échantillon (après un déplacement, par exemple).
                let failed =
                    self.child.as_mut().and_then(|c| c.try_wait().ok().flatten()).is_some_and(|s| !s.success());
                if failed && self.received == 0 { Err(self.failure()) } else { Ok(Chunk::End) }
            }
        }
    }

    fn seek(&mut self, secs: f64) -> Result<f64> {
        self.kill();
        self.primed.clear();
        let secs = secs.max(0.0);
        let (child, rx) = spawn(&self.exe, &self.path, self.rate, secs, &self.shared)?;
        self.child = Some(child);
        self.rx = rx;
        self.received = 0;
        Ok(secs)
    }

    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn duration(&self) -> Option<f64> {
        self.shared.lock().duration
    }
}

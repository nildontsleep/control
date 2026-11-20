//! Lanceur (`control-server --launcher`, et l'image Docker) : il fait tourner le serveur comme un processus fils.
//! C'est ce qui permet la mise à jour automatique : quand le serveur a téléchargé une nouvelle version, il demande au
//! lanceur de le redémarrer, et le lanceur relance le serveur sur cette version. Si elle ne démarre pas, le lanceur
//! revient tout seul à la précédente et se souvient de ne plus la réessayer.
//!
//! Le lanceur et le serveur se parlent par une connexion locale (127.0.0.1, port choisi par le lanceur, jeton
//! aléatoire) : des lignes JSON `{"type":"ready"}`, `{"type":"restart"}` (serveur → lanceur) et `{"type":"stop"}`
//! (lanceur → serveur), comme les messages IPC de l'original.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::appstate;
use crate::util::now_ms;

const READY_TIMEOUT: Duration = Duration::from_secs(90);
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
const CRASH_WINDOW_MS: i64 = 5 * 60 * 1000;

/// La version à lancer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    pub version: String,
    /// Binaire à lancer.
    pub binary: PathBuf,
    /// Mise à jour installée (sinon la version d'origine).
    pub staged: bool,
}

/// Celle que la mise à jour a installée, si elle est complète, plus récente et pas déjà en échec ; sinon celle d'origine.
pub fn choose(data_dir: &Path, bundled: &str, base_binary: &Path) -> Pick {
    let st = appstate::read_state(data_dir);
    if let Some(target) = appstate::str_of(&st, "target")
        && !appstate::bad_versions(&st).contains(&target)
        && appstate::compare_versions(&target, bundled).is_gt()
        && let Some(dir) = appstate::version_dir(data_dir, &target)
    {
        return Pick { version: target, binary: dir.join(appstate::binary_name()), staged: true };
    }
    Pick { version: bundled.to_string(), binary: base_binary.to_path_buf(), staged: false }
}

/// Une version qui ne démarre pas : on ne la réessaie plus et on revient à la précédente (ou à celle d'origine).
/// Renvoie la version vers laquelle on revient (`None` : celle d'origine).
pub fn abandon(data_dir: &Path, version: &str, reason: &str) -> Option<String> {
    let st = appstate::read_state(data_dir);
    let mut bad = appstate::bad_versions(&st);
    let previous = appstate::str_of(&st, "previous")
        .filter(|p| p != version && !bad.contains(p) && appstate::version_dir(data_dir, p).is_some());
    if !bad.iter().any(|v| v == version) {
        bad.push(version.to_string());
    }
    let _ = appstate::write_state(
        data_dir,
        json!({ "bad": bad, "target": previous, "previous": null, "failure": { "version": version, "reason": reason, "at": now_ms() } }),
    );
    previous
}

/// Attente avant un nouvel essai après `crashes` plantages récents.
pub fn backoff(crashes: usize) -> Duration {
    Duration::from_millis((1000u64 << crashes.saturating_sub(1).min(5)).min(30_000))
}

enum Msg {
    Ready,
    Restart,
}

/// Accepte la connexion du serveur fils (avec le bon jeton) et relaie ses messages.
async fn link(
    listener: &TcpListener,
    token: &str,
    tx: mpsc::UnboundedSender<Msg>,
) -> Option<tokio::net::tcp::OwnedWriteHalf> {
    loop {
        let (stream, _) = listener.accept().await.ok()?;
        let (read, write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        if lines.next_line().await.ok().flatten().as_deref() != Some(token) {
            continue;
        }
        let tx = tx.clone();
        tokio::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                let msg: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                let _ = match msg["type"].as_str() {
                    Some("ready") => tx.send(Msg::Ready),
                    Some("restart") => tx.send(Msg::Restart),
                    _ => Ok(()),
                };
            }
        });
        return Some(write);
    }
}

/// Arrête le serveur proprement (message), puis de force s'il ne répond pas.
async fn terminate(child: &mut Child, writer: &mut Option<tokio::net::tcp::OwnedWriteHalf>) {
    if let Some(w) = writer.as_mut() {
        let _ = w.write_all(b"{\"type\":\"stop\"}\n").await;
    }
    if tokio::time::timeout(STOP_TIMEOUT, child.wait()).await.is_err() {
        let _ = child.kill().await;
    }
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => {
                let _ = ctrl_c.await;
                return;
            }
        };
        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
}

/// Fait tourner le serveur, le relance, revient en arrière. Ne rend la main qu'à l'arrêt (code de sortie).
pub async fn run(args: Vec<String>) -> i32 {
    let data_dir = crate::config::Config::from_env().data_dir;
    let bundled = env!("CARGO_PKG_VERSION");
    let Ok(base) = std::env::current_exe() else {
        eprintln!("control : binaire introuvable.");
        return 1;
    };
    let mut crashes: Vec<i64> = Vec::new();
    let stop = shutdown_signal();
    tokio::pin!(stop);
    loop {
        let pick = choose(&data_dir, bundled, &base);
        let Ok(listener) = TcpListener::bind("127.0.0.1:0").await else {
            eprintln!("control : canal du lanceur impossible à ouvrir.");
            return 1;
        };
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        let token = hex::encode(rand::random::<[u8; 16]>());
        let mut cmd = Command::new(&pick.binary);
        cmd.args(&args)
            .env("CONTROL_SUPERVISED", "1")
            .env("CONTROL_LAUNCHER", format!("127.0.0.1:{port}"))
            .env("CONTROL_LAUNCHER_TOKEN", &token)
            .env_remove("CONTROL_STAGED")
            .kill_on_drop(true);
        if pick.staged {
            cmd.env("CONTROL_STAGED", &pick.version);
            println!("control : lancement de la version {} (mise à jour installée).", pick.version);
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(err) => {
                eprintln!("control : lancement impossible : {err}");
                if pick.staged {
                    abandon(&data_dir, &pick.version, &format!("lancement impossible, {err}"));
                    continue;
                }
                return 1;
            }
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut writer = None;
        let mut ready = false;
        let mut requested = false;
        let deadline = Instant::now() + READY_TIMEOUT;
        let status = loop {
            tokio::select! {
                w = link(&listener, &token, tx.clone()), if writer.is_none() => { writer = w; }
                msg = rx.recv() => match msg {
                    Some(Msg::Ready) => ready = true,
                    Some(Msg::Restart) => {
                        requested = true;
                        terminate(&mut child, &mut writer).await;
                    }
                    None => {}
                },
                st = child.wait() => break st.ok(),
                _ = tokio::time::sleep_until(deadline.into()), if !ready => {
                    eprintln!("control : la version {} n'a pas démarré en {} s.", pick.version, READY_TIMEOUT.as_secs());
                    let _ = child.kill().await;
                }
                _ = &mut stop => {
                    terminate(&mut child, &mut writer).await;
                    return 0;
                }
            }
        };
        if requested {
            continue;
        }
        let code = status.and_then(|s| s.code());
        if ready && code == Some(0) {
            return 0; // arrêt voulu
        }
        let reason = code.map(|c| format!("code {c}")).unwrap_or_else(|| "signal".into());
        if pick.staged && !ready {
            let back = abandon(&data_dir, &pick.version, &format!("a quitté avant d'être prêt, {reason}"));
            eprintln!(
                "control : la version {} ne démarre pas ; retour à la version {}.",
                pick.version,
                back.as_deref().unwrap_or(bundled)
            );
            continue;
        }
        let now = now_ms();
        crashes.retain(|t| now - t < CRASH_WINDOW_MS);
        crashes.push(now);
        if pick.staged && crashes.len() >= 3 {
            abandon(&data_dir, &pick.version, &format!("plantages répétés, {reason}"));
            crashes.clear();
            continue;
        }
        if crashes.len() >= 10 {
            eprintln!("control : le serveur plante en boucle, abandon.");
            return 1;
        }
        let wait = backoff(crashes.len());
        eprintln!("control : le serveur s'est arrêté ({reason}) ; nouvel essai dans {} s.", wait.as_secs());
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = &mut stop => return 0,
        }
    }
}

/// Côté serveur : la connexion au lanceur, s'il y en a un (`CONTROL_LAUNCHER`).
pub struct Supervisor {
    writer: parking_lot::Mutex<std::net::TcpStream>,
}

impl Supervisor {
    /// Se connecte au lanceur ; `on_stop` est appelé quand il demande l'arrêt.
    pub fn connect(on_stop: impl Fn() + Send + 'static) -> Option<std::sync::Arc<Supervisor>> {
        if std::env::var("CONTROL_SUPERVISED").ok().as_deref() != Some("1") {
            return None;
        }
        let addr = std::env::var("CONTROL_LAUNCHER").ok()?;
        let token = std::env::var("CONTROL_LAUNCHER_TOKEN").ok()?;
        let mut stream = std::net::TcpStream::connect(addr).ok()?;
        use std::io::{BufRead as _, Write as _};
        stream.write_all(format!("{token}\n").as_bytes()).ok()?;
        let reader = stream.try_clone().ok()?;
        let _ = std::thread::Builder::new().name("control-launcher".into()).spawn(move || {
            for line in std::io::BufReader::new(reader).lines() {
                let Ok(line) = line else { break };
                if serde_json::from_str::<Value>(&line).is_ok_and(|m| m["type"] == "stop") {
                    on_stop();
                }
            }
        });
        Some(std::sync::Arc::new(Supervisor { writer: parking_lot::Mutex::new(stream) }))
    }

    fn send(&self, msg: Value) {
        use std::io::Write as _;
        let _ = self.writer.lock().write_all(format!("{msg}\n").as_bytes());
    }

    pub fn ready(&self, version: &str) {
        self.send(json!({ "type": "ready", "version": version }));
    }

    pub fn restart(&self) {
        self.send(json!({ "type": "restart" }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(data: &Path, version: &str) {
        let dir = appstate::paths(data).versions.join(version);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".ok"), "").unwrap();
        std::fs::write(dir.join(appstate::binary_name()), "").unwrap();
    }

    #[test]
    fn picks_the_installed_update_and_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        let base = Path::new("/opt/control-server");
        assert_eq!(choose(data, "1.0.0", base), Pick { version: "1.0.0".into(), binary: base.into(), staged: false });

        // Cible annoncée mais pas installée : la version d'origine.
        appstate::write_state(data, json!({ "target": "1.1.0" })).unwrap();
        assert!(!choose(data, "1.0.0", base).staged);
        install(data, "1.1.0");
        let pick = choose(data, "1.0.0", base);
        assert!(pick.staged && pick.version == "1.1.0" && pick.binary.starts_with(data));
        // Plus ancienne que celle d'origine : ignorée.
        assert!(!choose(data, "1.2.0", base).staged);

        // 1.2.0 ne démarre pas : retour à 1.1.0 (la précédente), et elle n'est plus réessayée.
        install(data, "1.2.0");
        appstate::write_state(data, json!({ "target": "1.2.0", "previous": "1.1.0" })).unwrap();
        assert_eq!(choose(data, "1.0.0", base).version, "1.2.0");
        assert_eq!(abandon(data, "1.2.0", "code 1").as_deref(), Some("1.1.0"));
        let st = appstate::read_state(data);
        assert_eq!(appstate::bad_versions(&st), ["1.2.0"]);
        assert_eq!(st["failure"]["version"], "1.2.0");
        assert_eq!(choose(data, "1.0.0", base).version, "1.1.0");
        // 1.1.0 échoue à son tour : retour à la version d'origine.
        assert_eq!(abandon(data, "1.1.0", "code 1"), None);
        assert!(!choose(data, "1.0.0", base).staged);
        assert_eq!(appstate::bad_versions(&appstate::read_state(data)), ["1.2.0", "1.1.0"]);
        appstate::write_state(data, json!({ "target": "1.2.0" })).unwrap();
        assert!(!choose(data, "1.0.0", base).staged, "une version en échec n'est plus lancée");
    }

    #[test]
    fn backoff_grows_then_caps() {
        assert_eq!(backoff(1), Duration::from_secs(1));
        assert_eq!(backoff(2), Duration::from_secs(2));
        assert_eq!(backoff(4), Duration::from_secs(8));
        assert_eq!(backoff(9), Duration::from_secs(30));
    }
}

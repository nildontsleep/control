//! Lancement de yt-dlp et de ses compagnons : sortie capturée, délai maximal, arrêt de tout l'arbre de processus.
//!
//! yt-dlp lance lui-même ffmpeg, et sa version autonome pour Linux et macOS (PyInstaller) se dédouble au
//! démarrage : tuer le seul processus lancé laisserait tourner le reste. Sous Unix, chaque programme est donc
//! lancé dans son propre groupe de processus, arrêté d'un coup ; sous Windows, `taskkill /T` suit l'arbre.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::process::hide_window;

/// Prépare une commande lancée en arrière-plan : pas de console sous Windows, groupe de processus à part sous Unix.
pub fn configure(cmd: &mut Command) {
    hide_window(cmd);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
}

/// Arrête un programme lancé avec [`configure`] et tous ceux qu'il a lancés. `force` : sans lui laisser le temps
/// de finir proprement (sous Windows, l'arrêt est toujours immédiat).
pub fn kill_tree(child: &mut Child, force: bool) {
    #[cfg(unix)]
    {
        use rustix::process::{Pid, Signal, kill_process_group};
        let sig = if force { Signal::KILL } else { Signal::TERM };
        if kill_process_group(Pid::from_child(child), sig).is_ok() {
            return;
        }
    }
    #[cfg(windows)]
    {
        let _ = force;
        let mut cmd = Command::new("taskkill");
        cmd.args(["/pid", &child.id().to_string(), "/T", "/F"]).stdin(Stdio::null()).stdout(Stdio::null());
        cmd.stderr(Stdio::null());
        hide_window(&mut cmd);
        if cmd.status().is_ok_and(|s| s.success()) {
            return;
        }
    }
    #[cfg(not(any(unix, windows)))]
    let _ = force;
    let _ = child.kill();
}

/// Attend la fin d'un programme, au plus jusqu'à `deadline`.
pub fn wait_until(child: &mut Child, deadline: Instant) -> std::io::Result<Option<ExitStatus>> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Message d'échec d'un programme : dernière ligne de sa sortie d'erreur, sinon de sa sortie, sinon son code.
pub fn failure_message(stderr: &str, stdout: &str, code: Option<i32>) -> String {
    let code_text = match code {
        Some(c) => format!("code {c}"),
        None => "code null".to_string(),
    };
    let text = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        &code_text
    };
    text.trim().rsplit('\n').next().unwrap_or_default().trim().to_string()
}

fn read_all(r: Option<impl Read + Send + 'static>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut r) = r {
            let _ = r.read_to_end(&mut buf);
        }
        buf
    })
}

/// Lance un programme et renvoie sa sortie (sans espaces autour) s'il réussit. Arrêté au bout de `timeout`.
pub fn run(
    program: &Path,
    args: &[impl AsRef<OsStr>],
    timeout: Duration,
    envs: &[(OsString, OsString)],
) -> anyhow::Result<String> {
    let mut cmd = Command::new(program);
    cmd.args(args).envs(envs.iter().map(|(k, v)| (k, v)));
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    configure(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| anyhow::anyhow!("{} : {e}", program.display()))?;
    let out = read_all(child.stdout.take());
    let err = read_all(child.stderr.take());
    let status = match wait_until(&mut child, Instant::now() + timeout) {
        Ok(Some(status)) => Some(status),
        Ok(None) | Err(_) => {
            kill_tree(&mut child, true);
            let _ = child.wait();
            None
        }
    };
    let out = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    let err = String::from_utf8_lossy(&err.join().unwrap_or_default()).into_owned();
    match status {
        Some(s) if s.success() => Ok(out.trim().to_string()),
        Some(s) => Err(anyhow::anyhow!(failure_message(&err, &out, s.code()))),
        None => {
            let msg = failure_message(&err, &out, None);
            if msg == "code null" {
                Err(anyhow::anyhow!("Délai dépassé ({} s)", timeout.as_secs()))
            } else {
                Err(anyhow::anyhow!(msg))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_message_takes_the_last_useful_line() {
        assert_eq!(failure_message("WARNING: x\nERROR: boom\n", "out", Some(1)), "ERROR: boom");
        assert_eq!(failure_message("", "a\nb\r\n", Some(2)), "b");
        assert_eq!(failure_message("", "", Some(3)), "code 3");
        assert_eq!(failure_message("", "", None), "code null");
    }

    #[cfg(unix)]
    #[test]
    fn run_captures_and_times_out() {
        let sh = Path::new("/bin/sh");
        assert_eq!(run(sh, &["-c", "echo ' bonjour '"], Duration::from_secs(10), &[]).unwrap(), "bonjour");
        let err = run(sh, &["-c", "echo raté >&2; exit 4"], Duration::from_secs(10), &[]).unwrap_err();
        assert_eq!(err.to_string(), "raté");
        let env = [(OsString::from("CTRL_X"), OsString::from("42"))];
        assert_eq!(run(sh, &["-c", "echo $CTRL_X"], Duration::from_secs(10), &env).unwrap(), "42");
        // Le petit-enfant (sleep) est arrêté avec son parent : sinon la lecture de la sortie ne finirait pas.
        let start = Instant::now();
        let err = run(sh, &["-c", "sleep 30 & sleep 30"], Duration::from_millis(300), &[]).unwrap_err();
        assert!(err.to_string().starts_with("Délai dépassé"), "{err}");
        assert!(start.elapsed() < Duration::from_secs(10));
        assert!(run(Path::new("/nexiste/pas"), &["x"], Duration::from_secs(1), &[]).is_err());
    }
}

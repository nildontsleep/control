//! Le lanceur de bout en bout : un vrai processus (`control-server --launcher`) qui lance le serveur, le met à jour
//! sur une version téléchargée, et revient à la version précédente quand la nouvelle ne démarre pas
//! (port de `server/test/launcher.test.js`). Les « versions » téléchargées sont de petits scripts qui relancent ce
//! même binaire (ou échouent) : Unix seulement.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Client, PASSWORD};
use control_server::appstate;
use control_server::tar::{TarFile, pack};
use control_server::updater::{asset_name, target};
use parking_lot::Mutex;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

struct Release {
    version: String,
    data: Vec<u8>,
}

/// Un GitHub de poche : `releases/latest` et l'archive.
fn github() -> (String, Arc<Mutex<Option<Release>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let release: Arc<Mutex<Option<Release>>> = Arc::default();
    let (rel, b) = (release.clone(), base.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap_or(0);
            loop {
                let mut l = String::new();
                if reader.read_line(&mut l).unwrap_or(0) == 0 || l == "\r\n" {
                    break;
                }
            }
            let path = line.split(' ').nth(1).unwrap_or("").to_string();
            let r = rel.lock();
            let (status, body): (u16, Vec<u8>) = match r.as_ref() {
                Some(r) if path == "/releases/latest" => {
                    let digest = hex::encode(Sha256::digest(&r.data));
                    let body = json!({
                        "tag_name": format!("v{}", r.version), "name": r.version, "body": "", "html_url": format!("{b}/release"),
                        "draft": false, "prerelease": false,
                        "assets": [{ "name": asset_name(&r.version), "size": r.data.len(), "browser_download_url": format!("{b}/asset"), "digest": format!("sha256:{digest}") }],
                    });
                    (200, body.to_string().into_bytes())
                }
                Some(r) if path == "/asset" => (200, r.data.clone()),
                _ => (404, Vec::new()),
            };
            let _ =
                write!(stream, "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
            let _ = stream.write_all(&body);
        }
    });
    (format!("{base}/releases/latest"), release)
}

/// L'archive d'une version : un script qui relance ce binaire (version qui marche) ou qui échoue aussitôt.
fn bundle(version: &str, broken: bool) -> Release {
    let real = env!("CARGO_BIN_EXE_control-server");
    let script = if broken {
        "#!/bin/sh\necho 'version cassée (essai)' >&2\nexit 1\n".to_string()
    } else {
        format!("#!/bin/sh\nexec '{real}' \"$@\"\n")
    };
    let files = [
        TarFile::new(
            "manifest.json",
            json!({ "name": "control-server", "version": version, "target": target() }).to_string(),
        ),
        TarFile { name: appstate::binary_name(), data: script.into_bytes(), mode: 0o755 },
    ];
    Release { version: version.into(), data: pack(&files).unwrap() }
}

fn health(base: &str) -> Option<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(2)))
        .build()
        .into();
    let mut res = agent.get(format!("{base}/api/health")).call().ok()?;
    let v: Value = res.body_mut().read_json().ok()?;
    v["version"].as_str().map(str::to_string)
}

fn wait<T>(secs: u64, mut f: impl FnMut() -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < end, "délai dépassé");
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn launcher_updates_then_rolls_back_a_broken_version() {
    let data = tempfile::tempdir().unwrap();
    let (url, release) = github();
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let mut launcher = Command::new(env!("CARGO_BIN_EXE_control-server"))
        .arg("--launcher")
        .env("CONTROL_DATA", data.path())
        .env("PORT", port.to_string())
        .env("CONTROL_HOST", "127.0.0.1")
        .env("CONTROL_RESCAN_MINUTES", "1000")
        .env("CONTROL_UPDATE_URL", &url)
        .env("CONTROL_UPDATE", "")
        .env("CONTROL_LIBRARY", "")
        .env_remove("CONTROL_STAGED")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let bundled = env!("CARGO_PKG_VERSION");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_eq!(wait(30, || health(&base)), bundled, "le serveur d'origine démarre");
        let admin = Client::new(&base);
        assert_eq!(admin.setup(json!({ "username": "admin", "password": PASSWORD })).status, 200);

        // ─── Mise à jour vers 99.0.0 ────────────────────────────────────────
        *release.lock() = Some(bundle("99.0.0", false));
        let found = admin.rpc("srv:update:check", vec![]).unwrap();
        assert_eq!(
            (&found["phase"], &found["version"], &found["supervised"]),
            (&json!("available"), &json!("99.0.0"), &json!(true))
        );
        assert_eq!(admin.rpc("srv:update:download", vec![]).unwrap()["phase"], "ready");
        assert_eq!(health(&base).as_deref(), Some(bundled), "rien ne change avant l'installation");
        assert_eq!(admin.rpc("srv:update:install", vec![]).unwrap()["phase"], "installing");
        wait(40, || (health(&base).as_deref() == Some("99.0.0")).then_some(()));
        assert!(launcher.try_wait().unwrap().is_none(), "le lanceur, lui, n'a pas bougé");
        assert_eq!(appstate::str_of(&appstate::read_state(data.path()), "target").as_deref(), Some("99.0.0"));
        // La session et les données survivent au redémarrage.
        assert_eq!(admin.rpc("srv:update:status", vec![]).unwrap()["current"], "99.0.0");
        let backups: Vec<String> = std::fs::read_dir(data.path().join("backups"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            backups.iter().any(|n| n.starts_with(&format!("server-{bundled}-"))),
            "base sauvegardée avant le changement"
        );

        // ─── Une version cassée : retour à 99.0.0, et on ne la réessaie plus ──
        *release.lock() = Some(bundle("99.1.0", true));
        assert_eq!(admin.rpc("srv:update:check", vec![]).unwrap()["version"], "99.1.0");
        assert_eq!(admin.rpc("srv:update:download", vec![]).unwrap()["phase"], "ready");
        assert_eq!(admin.rpc("srv:update:install", vec![]).unwrap()["phase"], "installing");
        wait(40, || {
            appstate::bad_versions(&appstate::read_state(data.path())).contains(&"99.1.0".to_string()).then_some(())
        });
        wait(40, || (health(&base).as_deref() == Some("99.0.0")).then_some(()));
        let st = appstate::read_state(data.path());
        assert_eq!(
            (appstate::str_of(&st, "target").as_deref(), &st["failure"]["version"]),
            (Some("99.0.0"), &json!("99.1.0"))
        );
        let status = admin.rpc("srv:update:status", vec![]).unwrap();
        assert_eq!(status["failure"]["version"], "99.1.0", "l'administrateur voit pourquoi");
        assert_eq!(
            admin.rpc("srv:update:check", vec![]).unwrap()["phase"],
            "available",
            "la version peut être redemandée à la main"
        );
    }));

    // ─── Arrêt propre ───────────────────────────────────────────────────────
    let _ = Command::new("kill").arg("-TERM").arg(launcher.id().to_string()).status();
    let code = wait(15, || launcher.try_wait().ok().flatten());
    if result.is_err() {
        let _ = launcher.kill();
    }
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
    assert_eq!(code.code(), Some(0));
}

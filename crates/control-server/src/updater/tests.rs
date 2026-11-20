use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;
use std::sync::atomic::AtomicUsize;

use super::*;
use crate::tar::{TarFile, pack};

/// Une release publiée sur le faux GitHub.
#[derive(Clone, Default)]
struct Published {
    version: String,
    data: Vec<u8>,
    sha256: String,
    no_asset: bool,
    no_digest: bool,
    digest_override: Option<String>,
    signature: Option<String>,
}

/// Un GitHub de poche : `releases/latest` et le téléchargement de l'archive.
struct Github {
    url: String,
    base: String,
    release: Arc<Mutex<Option<Published>>>,
    hits: Arc<Mutex<Vec<String>>>,
}

fn respond(stream: &mut std::net::TcpStream, status: u16, kind: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: {kind}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn github() -> Github {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let release: Arc<Mutex<Option<Published>>> = Arc::default();
    let hits: Arc<Mutex<Vec<String>>> = Arc::default();
    let (rel, h, b) = (release.clone(), hits.clone(), base.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut line = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            reader.read_line(&mut line).unwrap_or(0);
            loop {
                let mut l = String::new();
                if reader.read_line(&mut l).unwrap_or(0) == 0 || l == "\r\n" {
                    break;
                }
            }
            let path = line.split(' ').nth(1).unwrap_or("").to_string();
            h.lock().push(path.clone());
            let r = rel.lock().clone();
            match r {
                Some(r) if path == "/releases/latest" => {
                    let mut assets = Vec::new();
                    if !r.no_asset {
                        let digest = if r.no_digest {
                            Value::Null
                        } else {
                            json!(format!("sha256:{}", r.digest_override.clone().unwrap_or(r.sha256.clone())))
                        };
                        assets.push(json!({ "name": asset_name(&r.version), "size": r.data.len(), "browser_download_url": format!("{b}/asset/{}.tgz", r.version), "digest": digest }));
                    }
                    if let Some(sig) = &r.signature {
                        assets.push(json!({ "name": format!("{}.sig", asset_name(&r.version)), "size": sig.len(), "browser_download_url": format!("{b}/asset/{}.tgz.sig", r.version), "digest": null }));
                    }
                    let body = json!({
                        "tag_name": format!("v{}", r.version), "name": format!("control {}", r.version), "body": "Nouveautés de la version.",
                        "html_url": format!("{b}/release"), "draft": false, "prerelease": false, "assets": assets,
                    });
                    respond(&mut stream, 200, "application/json", body.to_string().as_bytes());
                }
                Some(r) if path == format!("/asset/{}.tgz.sig", r.version) && r.signature.is_some() => {
                    respond(&mut stream, 200, "text/plain", r.signature.unwrap_or_default().as_bytes());
                }
                Some(r) if path == format!("/asset/{}.tgz", r.version) => {
                    respond(&mut stream, 200, "application/gzip", &r.data)
                }
                _ => respond(&mut stream, 404, "text/plain", b""),
            }
        }
    });
    Github { url: format!("{base}/releases/latest"), base, release, hits }
}

impl Github {
    fn publish(&self, p: Published) {
        *self.release.lock() = Some(p);
    }
}

/// L'archive d'une version fictive (et au besoin une archive pour une autre plateforme, ou sans binaire).
fn bundle(version: &str, manifest_version: &str, target_name: &str, extra: Vec<TarFile>) -> Published {
    let mut files = vec![
        TarFile::new(
            "manifest.json",
            json!({ "name": "control-server", "version": manifest_version, "target": target_name }).to_string(),
        ),
        TarFile { name: appstate::binary_name(), data: b"#!/bin/sh\nexit 0\n".to_vec(), mode: 0o755 },
        TarFile::new("src/web/login.html", "<html></html>"),
    ];
    files.extend(extra);
    let data = pack(&files).unwrap();
    Published { version: version.into(), sha256: sha256_hex(&data), data, ..Published::default() }
}

fn fake(version: &str) -> Published {
    bundle(version, version, &target(), Vec::new())
}

struct Setup {
    dir: tempfile::TempDir,
    github: Github,
    updater: Arc<Updater>,
    restarts: Arc<AtomicUsize>,
    backups: Arc<AtomicUsize>,
    config: Arc<Mutex<UpdateConfig>>,
}

fn updater_for(
    dir: &Path,
    url: &str,
    config: Arc<Mutex<UpdateConfig>>,
    supervised: bool,
    idle: bool,
    keys: Vec<String>,
) -> (Arc<Updater>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let restarts = Arc::new(AtomicUsize::new(0));
    let backups = Arc::new(AtomicUsize::new(0));
    let (r, b) = (restarts.clone(), backups.clone());
    let restart: Option<Callback> = supervised.then(|| {
        Arc::new(move || {
            r.fetch_add(1, Ordering::SeqCst);
        }) as Callback
    });
    let updater = Arc::new(Updater::new(UpdaterOptions {
        version: "0.9.0".into(),
        data_dir: dir.to_path_buf(),
        api_url: Some(url.to_string()),
        public_keys: Some(keys),
        settings: Arc::new(move || config.lock().clone()),
        restart,
        backup: Arc::new(move || {
            b.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }),
        idle: Arc::new(move || idle),
        on_state: Arc::new(|_| {}),
    }));
    (updater, restarts, backups)
}

fn setup(mode: &str, hour: i64, supervised: bool, idle: bool) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let github = github();
    let config = Arc::new(Mutex::new(UpdateConfig { mode: mode.into(), hour, ..UpdateConfig::default() }));
    let (updater, restarts, backups) =
        updater_for(dir.path(), &github.url, config.clone(), supervised, idle, Vec::new());
    Setup { dir, github, updater, restarts, backups, config }
}

fn phase(v: &Value) -> &str {
    v["phase"].as_str().unwrap_or("")
}

#[test]
fn check_download_install_and_restart() {
    let t = setup("notify", 4, true, false);
    t.github.publish(fake("0.9.0"));
    assert_eq!(phase(&t.updater.check(false)), "none", "même version : rien à faire");
    t.github.publish(fake("0.9.1"));
    let found = t.updater.check(true);
    assert_eq!((phase(&found), found["version"].as_str()), ("available", Some("0.9.1")));
    assert!(found.get("manual").is_none());
    assert!(found["notes"].as_str().unwrap().contains("Nouveautés"));

    let ready = t.updater.download();
    assert_eq!((phase(&ready), &ready["verified"]), ("ready", &json!(true)));
    let dir = appstate::version_dir(t.dir.path(), "0.9.1").expect("version installée à côté de celle en cours");
    assert!(dir.join("src/web/login.html").exists());
    assert_eq!(
        appstate::str_of(&appstate::read_state(t.dir.path()), "target"),
        None,
        "rien n'est activé avant l'installation"
    );

    let installing = t.updater.install();
    assert_eq!(phase(&installing), "installing");
    assert_eq!(t.backups.load(Ordering::SeqCst), 1);
    assert_eq!(appstate::str_of(&appstate::read_state(t.dir.path()), "target").as_deref(), Some("0.9.1"));
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(t.restarts.load(Ordering::SeqCst), 1, "le lanceur est prié de redémarrer le serveur");
}

#[test]
fn refuses_corrupt_lying_escaping_and_foreign_archives() {
    let t = setup("notify", 4, true, false);
    let mut corrupt = fake("0.9.1");
    corrupt.digest_override = Some("0".repeat(64));
    t.github.publish(corrupt);
    t.updater.check(true);
    let st = t.updater.download();
    assert_eq!((phase(&st), st["code"].as_str()), ("error", Some("corrupt")));
    assert_eq!(appstate::version_dir(t.dir.path(), "0.9.1"), None);

    // Annoncée 0.9.2, mais contient 0.9.3.
    t.github.publish(bundle("0.9.2", "0.9.3", &target(), Vec::new()));
    t.updater.check(true);
    assert_eq!(t.updater.download()["code"], "stage");

    // Chemin qui sortirait du dossier d'extraction.
    let mut raw = Vec::new();
    std::io::Read::read_to_end(
        &mut flate2::read::GzDecoder::new(&pack(&[TarFile::new("manifest.json", "{}")]).unwrap()[..]),
        &mut raw,
    )
    .unwrap();
    raw[..12].copy_from_slice(b"../../evil\0\0");
    raw[148..156].fill(32);
    let sum: u64 = raw[..512].iter().map(|&b| b as u64).sum();
    raw[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(&raw).unwrap();
    let evil = enc.finish().unwrap();
    t.github.publish(Published {
        version: "0.9.4".into(),
        sha256: sha256_hex(&evil),
        data: evil,
        ..Published::default()
    });
    t.updater.check(true);
    assert_eq!(t.updater.download()["code"], "stage");
    assert!(!t.dir.path().join("..").join("evil").exists());

    // Pas de manifeste.
    let bare = pack(&[TarFile::new(appstate::binary_name(), "x")]).unwrap();
    t.github.publish(Published {
        version: "0.9.6".into(),
        sha256: sha256_hex(&bare),
        data: bare,
        ..Published::default()
    });
    t.updater.check(true);
    assert_eq!(t.updater.download()["code"], "stage");

    // Archive d'une autre plateforme.
    t.github.publish(bundle("0.9.5", "0.9.5", "plan9-mips", Vec::new()));
    t.updater.check(true);
    assert_eq!(t.updater.download()["code"], "engine");
    let versions = appstate::paths(t.dir.path()).versions;
    let left: Vec<_> = std::fs::read_dir(&versions).map(|r| r.flatten().collect()).unwrap_or_default();
    assert!(left.is_empty(), "aucune version à moitié installée");
}

#[test]
fn warns_without_launcher_asset_or_digest() {
    let bare = setup("notify", 4, false, false);
    bare.github.publish(fake("0.9.1"));
    let st = bare.updater.check(true);
    assert_eq!((phase(&st), &st["manual"], st["reason"].as_str()), ("available", &json!(true), Some("unsupervised")));
    assert_eq!(phase(&bare.updater.download()), "available", "rien n'est téléchargé");

    let t = setup("notify", 4, true, false);
    t.github.publish(Published { no_asset: true, ..fake("0.9.1") });
    assert_eq!(t.updater.check(true)["reason"], "asset");
    t.github.publish(Published { no_digest: true, ..fake("0.9.1") });
    assert_eq!(t.updater.check(true)["reason"], "nodigest");
}

#[test]
fn modes_and_abandoned_versions() {
    let t = setup("off", 4, true, false);
    t.github.publish(fake("0.9.1"));
    t.updater.check(false);
    assert!(t.github.hits.lock().is_empty(), "rien n'est demandé à GitHub");
    assert_eq!(phase(&t.updater.check(true)), "available", "mais l'administrateur peut toujours le demander");

    t.config.lock().mode = "notify".into();
    t.updater.run();
    assert_eq!(phase(&t.updater.status()), "available", "on prévient, sans rien télécharger");

    let now_hour = i64::from(control_core::util::local_datetime(now_ms()).hour());
    t.config.lock().mode = "auto".into();
    t.config.lock().hour = (now_hour + 12) % 24;
    t.updater.run();
    t.updater.check(false);
    assert_eq!(phase(&t.updater.status()), "ready", "téléchargée, mais pas installée : des comptes sont connectés");
    t.config.lock().hour = now_hour;
    t.updater.auto();
    assert_eq!(phase(&t.updater.status()), "installing", "installée à l'heure choisie");

    // Une version abandonnée par le lanceur n'est plus proposée automatiquement.
    appstate::write_state(t.dir.path(), json!({ "bad": ["0.9.2"] })).unwrap();
    t.github.publish(fake("0.9.2"));
    let (fresh, _, _) = updater_for(t.dir.path(), &t.github.url, t.config.clone(), true, false, Vec::new());
    let st = fresh.check(false);
    assert_eq!((phase(&st), st["skipped"].as_str()), ("none", Some("0.9.2")));
    assert_eq!(phase(&fresh.check(true)), "available", "sauf si l'administrateur la redemande");
}

#[test]
fn auto_installs_when_nobody_is_connected() {
    let hour = (i64::from(control_core::util::local_datetime(now_ms()).hour()) + 12) % 24;
    let t = setup("auto", hour, true, true);
    t.github.publish(fake("0.9.1"));
    t.updater.run();
    assert_eq!(phase(&t.updater.status()), "installing");
    assert_eq!(appstate::str_of(&appstate::read_state(t.dir.path()), "target").as_deref(), Some("0.9.1"));
}

#[test]
fn cleanup_and_database_backup() {
    let t = setup("notify", 4, true, false);
    let versions = appstate::paths(t.dir.path()).versions;
    for v in ["0.9.1", "0.9.2", "0.9.3"] {
        std::fs::create_dir_all(versions.join(v)).unwrap();
        std::fs::write(versions.join(v).join(".ok"), "").unwrap();
        std::fs::write(versions.join(v).join(appstate::binary_name()), "").unwrap();
    }
    std::fs::create_dir_all(versions.join("0.9.4.tmp-123")).unwrap();
    appstate::write_state(t.dir.path(), json!({ "target": "0.9.3", "previous": "0.9.2" })).unwrap();
    t.updater.cleanup();
    let mut left: Vec<String> =
        std::fs::read_dir(&versions).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    assert_eq!(left, ["0.9.2", "0.9.3"], "la version visée et la précédente sont gardées");

    let db = crate::serverdb::open(&t.dir.path().join("server.db")).unwrap();
    crate::serverdb::kv_set(&db, "x", "y");
    for _ in 0..4 {
        backup_db(&db, t.dir.path(), "0.9.0").unwrap();
        std::thread::sleep(Duration::from_millis(2));
    }
    let copies = std::fs::read_dir(t.dir.path().join("backups")).unwrap().count();
    assert_eq!(copies, KEEP_BACKUPS);
    let _ = &t.github.base;
}

#[test]
fn signed_updates_need_a_trusted_key() {
    let (trusted, trusted_private) = crate::sign::keygen();
    let (_, stranger_private) = crate::sign::keygen();
    let t = setup("notify", 4, true, false);
    let keyed = || updater_for(t.dir.path(), &t.github.url, t.config.clone(), true, false, vec![trusted.clone()]).0;
    let b = fake("0.9.1");
    let signed_by = |key: &str, version: &str| crate::sign::sign(version, &b.data, key).unwrap();
    let updater = keyed();
    assert_eq!(updater.status()["signed"], true);

    t.github.publish(Published { signature: Some(signed_by(&trusted_private, "0.9.1")), ..b.clone() });
    assert_eq!(phase(&updater.check(true)), "available");
    let ready = updater.download();
    assert_eq!((phase(&ready), &ready["signed"]), ("ready", &json!(true)));
    std::fs::remove_dir_all(t.dir.path().join("app")).unwrap();

    for bad in [signed_by(&stranger_private, "0.9.1"), signed_by(&trusted_private, "0.9.2"), "AAAA".into()] {
        t.github.publish(Published { signature: Some(bad), ..b.clone() });
        let fresh = keyed();
        fresh.check(true);
        let st = fresh.download();
        assert_eq!((phase(&st), st["code"].as_str()), ("error", Some("signature")));
        assert_eq!(appstate::version_dir(t.dir.path(), "0.9.1"), None);
    }

    // Archive modifiée après la signature.
    let forged = bundle("0.9.1", "0.9.1", &target(), vec![TarFile::new("evil.sh", "exit 0")]);
    t.github.publish(Published { signature: Some(signed_by(&trusted_private, "0.9.1")), ..forged });
    let f = keyed();
    f.check(true);
    assert_eq!(f.download()["code"], "signature");

    // Pas de signature alors que des clés sont configurées : on prévient, sans installer.
    t.github.publish(b.clone());
    let st = keyed().check(true);
    assert_eq!((phase(&st), &st["manual"], st["reason"].as_str()), ("available", &json!(true), Some("nosignature")));
}

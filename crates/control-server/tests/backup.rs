//! Sauvegardes : archive sans secrets, chiffrement, découpe, envoi au webhook, planification
//! (port de `server/test/backup.test.js`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::sync::Arc;

use common::{Boot, PASSWORD, boot};
use control_server::backup::decrypt;
use control_server::tar::{Limits, unpack};
use parking_lot::Mutex;
use serde_json::{Value, json};

struct Message {
    payload: Value,
    name: Option<String>,
    data: Option<Vec<u8>>,
}

struct Discord {
    url: String,
    messages: Arc<Mutex<Vec<Message>>>,
    limited: Arc<Mutex<usize>>,
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Les parties d'un formulaire multipart : (nom, nom de fichier, contenu).
fn parts(body: &[u8], boundary: &str) -> Vec<(String, Option<String>, Vec<u8>)> {
    let sep = format!("--{boundary}");
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(i) = find(rest, sep.as_bytes()) {
        rest = &rest[i + sep.len()..];
        if rest.starts_with(b"--") {
            break;
        }
        let Some(h) = find(rest, b"\r\n\r\n") else { break };
        let head = String::from_utf8_lossy(&rest[..h]).into_owned();
        let data_start = h + 4;
        let end = find(&rest[data_start..], format!("\r\n{sep}").as_bytes()).unwrap_or(rest.len() - data_start);
        let field =
            |key: &str| head.split(&format!("{key}=\"")).nth(1).and_then(|v| v.split('"').next()).map(str::to_string);
        out.push((field("name").unwrap_or_default(), field("filename"), rest[data_start..data_start + end].to_vec()));
        rest = &rest[data_start + end..];
    }
    out
}

/// Un Discord de poche : reçoit les messages d'un webhook (JSON + fichier joint).
fn discord() -> Discord {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/api/webhooks/1", listener.local_addr().unwrap());
    let messages: Arc<Mutex<Vec<Message>>> = Arc::default();
    let limited: Arc<Mutex<usize>> = Arc::default();
    let (m, l) = (messages.clone(), limited.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let (mut len, mut boundary) = (0usize, String::new());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                if h == "\r\n" || h.is_empty() {
                    break;
                }
                let lower = h.to_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                if lower.starts_with("content-type:") {
                    boundary = h.split("boundary=").nth(1).unwrap_or("").trim().to_string();
                }
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body).unwrap();
            let reply = |s: &mut std::net::TcpStream, status: &str, body: &str| {
                write!(s, "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).unwrap();
            };
            {
                let mut left = l.lock();
                if *left > 0 {
                    *left -= 1;
                    reply(&mut stream, "429 Too Many", "{\"retry_after\":0.01}");
                    continue;
                }
            }
            let form = parts(&body, &boundary);
            let payload = form
                .iter()
                .find(|p| p.0 == "payload_json")
                .map(|p| serde_json::from_slice(&p.2).unwrap())
                .unwrap_or(Value::Null);
            let file = form.iter().find(|p| p.0 == "files[0]");
            m.lock().push(Message { payload, name: file.and_then(|f| f.1.clone()), data: file.map(|f| f.2.clone()) });
            reply(&mut stream, "200 OK", "{\"id\":\"1\"}");
        }
    });
    Discord { url, messages, limited }
}

fn gunzip(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(data).read_to_end(&mut out).unwrap();
    out
}

#[test]
fn encrypted_backup_reaches_the_webhook_without_secrets() {
    let d = discord();
    let s = boot(Boot { no_common: true, backup_any_url: true, ..Boot::default() });
    let admin = s.client();
    admin.setup(json!({ "username": "nil", "password": PASSWORD, "libraryFolders": [s.music] }));
    let ami =
        admin.rpc("srv:users:create", vec![json!({ "username": "ami", "password": PASSWORD })]).unwrap()["id"].clone();
    let ami_client = s.client();
    ami_client.login("ami", PASSWORD);
    let tracks = admin.wait_tracks(3);
    admin.rpc("lib:play-start", vec![tracks[0]["id"].clone(), json!(1)]).unwrap();
    admin.rpc("pl:create", vec![json!("À garder"), json!("manual"), Value::Null, json!([tracks[1]["id"]])]).unwrap();
    admin
        .rpc(
            "dl:set-cookies",
            vec![json!([".youtube.com", "TRUE", "/", "TRUE", "1893456000", "SID", "secretcookie"].join("\t"))],
        )
        .unwrap();
    admin.rpc("srv:settings", vec![json!({ "updateToken": "ghp_secret", "tmdbKey": "tmdb_secret" })]).unwrap();

    // Réglages : l'adresse invalide est refusée, les secrets ne reviennent jamais au navigateur.
    assert_eq!(
        admin.rpc("srv:settings", vec![json!({ "backupWebhook": "ftp://exemple.org/x" })]).unwrap_err().status,
        400
    );
    assert_eq!(ami_client.rpc("srv:backup:run", vec![]).unwrap_err().status, 403);
    admin.rpc("srv:settings", vec![json!({ "backupWebhook": d.url, "backupPassphrase": "phrase de test", "backupEvery": "daily", "backupHour": 3 })]).unwrap();
    let shown = admin.rpc("srv:settings:get", vec![]).unwrap();
    assert_eq!((&shown["hasBackupWebhook"], &shown["hasBackupPassphrase"]), (&json!(true), &json!(true)));
    assert!(shown.get("backupWebhook").is_none() && shown.get("backupPassphrase").is_none());
    assert!(!shown.to_string().contains("phrase de test"));

    // Discord limite d'abord (429) : l'envoi est retenté.
    *d.limited.lock() = 1;
    let status = admin.rpc("srv:backup:run", vec![]).unwrap();
    assert_eq!(
        (&status["last"]["ok"], &status["last"]["accounts"], &status["last"]["encrypted"]),
        (&json!(true), &json!(2), &json!(true))
    );
    let messages = d.messages.lock();
    assert_eq!(messages.len(), 1);
    let content = messages[0].payload["content"].as_str().unwrap();
    assert!(content.contains("Sauvegarde") && content.contains("chiffrée"));
    assert_eq!(messages[0].payload["allowed_mentions"], json!({ "parse": [] }));
    let name = messages[0].name.clone().unwrap();
    assert!(
        name.starts_with("control-backup-")
            && name.ends_with(".tgz.enc")
            && name.len() == "control-backup-".len() + 12 + 8,
        "{name}"
    );

    // L'archive : se déchiffre avec la phrase, contient les bases, pas les secrets.
    let archive =
        unpack(&decrypt(messages[0].data.as_ref().unwrap(), "phrase de test").unwrap(), Limits::default()).unwrap();
    drop(messages);
    let names: Vec<&str> = archive.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"MANIFEST.json") && names.contains(&"server.db"));
    assert_eq!(names.iter().filter(|n| n.ends_with("/library.db")).count(), 2, "une base par compte");
    assert!(!names.iter().any(|n| n.contains("cookies")), "pas de cookies YouTube");
    assert!(!archive.iter().any(|f| String::from_utf8_lossy(&f.data).contains("secretcookie")));
    let tmp = tempfile::tempdir().unwrap();
    let open = |name: &str| {
        let file = tmp.path().join(name.replace(['/', '.'], "_"));
        std::fs::write(&file, &archive.iter().find(|f| f.name == name).unwrap().data).unwrap();
        rusqlite::Connection::open(file).unwrap()
    };
    let server_db = open("server.db");
    let count = |db: &rusqlite::Connection, sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(count(&server_db, "SELECT COUNT(*) FROM sessions"), 0, "sans les sessions");
    assert_eq!(count(&server_db, "SELECT COUNT(*) FROM users"), 2);
    let kept: Value = serde_json::from_str(
        &server_db.query_row("SELECT value FROM kv WHERE key = 'settings'", [], |r| r.get::<_, String>(0)).unwrap(),
    )
    .unwrap();
    for secret in ["updateToken", "tmdbKey", "backupWebhook", "backupPassphrase"] {
        assert!(kept.get(secret).is_none(), "{secret}");
    }
    assert_eq!(kept["backupEvery"], "daily");
    let admin_id = admin.me()["id"].as_str().unwrap().to_string();
    let library = open(&format!("users/{admin_id}/library.db"));
    assert_eq!(count(&library, "SELECT COUNT(*) FROM plays"), 1, "les écoutes sont sauvegardées");
    assert_eq!(library.query_row("SELECT name FROM playlists", [], |r| r.get::<_, String>(0)).unwrap(), "À garder");
    let manifest: Value =
        serde_json::from_slice(&archive.iter().find(|f| f.name == "MANIFEST.json").unwrap().data).unwrap();
    let mut users: Vec<&str> =
        manifest["accounts"].as_array().unwrap().iter().map(|a| a["username"].as_str().unwrap()).collect();
    users.sort();
    assert_eq!(users, ["ami", "nil"]);
    assert!(manifest["accounts"].as_array().unwrap().iter().any(|a| a["id"] == ami));

    // Une copie reste aussi sur le disque du serveur.
    let local = std::fs::read_dir(s.root.join("data").join("backups"))
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().ends_with(".tgz.enc"));
    assert!(local);

    // Test du webhook.
    assert_eq!(admin.rpc("srv:backup:test", vec![]).unwrap(), true);
    assert!(d.messages.lock()[1].payload["content"].as_str().unwrap().contains("fonctionne"));
}

#[test]
fn backup_parts_rejoin_and_stay_plain_without_passphrase() {
    let s = boot(Boot { no_common: true, backup_any_url: true, ..Boot::default() });
    let admin = s.client();
    admin.setup(json!({ "username": "nil", "password": PASSWORD, "libraryFolders": [s.music] }));
    admin.wait_tracks(3);
    let hub = s.hub();
    let built = hub.backups.build(hub, 4096).unwrap();
    assert!(built.parts.len() > 1, "plusieurs parties");
    for (i, (name, data)) in built.parts.iter().enumerate() {
        assert!(data.len() <= 4096);
        assert_eq!(name, &format!("{}.part{:02}", built.name, i + 1));
    }
    assert!(!built.encrypted);
    let joined: Vec<u8> = built.parts.iter().flat_map(|(_, d)| d.clone()).collect();
    assert_eq!(joined.len(), built.size);
    let raw = gunzip(&joined);
    assert!(!raw.is_empty());
    assert!(unpack(&joined, Limits::default()).unwrap().iter().any(|f| f.name == "server.db"));
    assert_eq!(hub.backups.run(hub, "manuelle").unwrap_err().status, 400, "sans webhook, rien n'est envoyé");
}

/// Un instant en heure locale, en millisecondes.
fn at(date: &str, hour: u8, minute: u8) -> i64 {
    let n: Vec<i32> = date.split('-').map(|x| x.parse().unwrap()).collect();
    let d = time::Date::from_calendar_date(n[0], time::Month::try_from(n[1] as u8).unwrap(), n[2] as u8).unwrap();
    let local = d.with_hms(hour, minute, 0).unwrap();
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    local.assume_offset(offset).unix_timestamp() * 1000
}

#[test]
fn scheduled_backup_due_at_hour_daily_weekly_and_retry() {
    let s = boot(Boot { no_common: true, backup_any_url: true, ..Boot::default() });
    let hub = s.hub();
    let b = &hub.backups;
    assert!(!b.due(hub, at("2026-11-03", 5, 0)), "rien sans webhook");
    hub.patch_settings(
        &json!({ "backupWebhook": "http://127.0.0.1:1/api/webhooks/1", "backupEvery": "daily", "backupHour": 3 }),
    )
    .unwrap();
    assert!(!b.due(hub, at("2026-11-03", 2, 30)), "avant l'heure choisie");
    assert!(b.due(hub, at("2026-11-03", 3, 5)));

    let save = |state: Value| control_server::serverdb::kv_set(&hub.db.lock(), "backup_state", &state.to_string());
    save(json!({ "at": at("2026-11-03", 3, 5), "ok": true }));
    assert!(!b.due(hub, at("2026-11-03", 15, 0)), "déjà faite aujourd'hui");
    assert!(!b.due(hub, at("2026-11-04", 2, 0)));
    assert!(b.due(hub, at("2026-11-04", 3, 10)), "le lendemain à l'heure");

    hub.patch_settings(&json!({ "backupEvery": "weekly" })).unwrap();
    assert!(!b.due(hub, at("2026-11-04", 3, 10)));
    assert!(b.due(hub, at("2026-11-10", 3, 10)), "une semaine plus tard");

    hub.patch_settings(&json!({ "backupEvery": "daily" })).unwrap();
    save(json!({ "at": at("2026-11-04", 3, 10), "ok": false, "error": "x" }));
    assert!(!b.due(hub, at("2026-11-04", 3, 40)), "pas de nouvel essai avant une heure");
    assert!(b.due(hub, at("2026-11-04", 4, 20)));
    hub.patch_settings(&json!({ "backupEvery": "off" })).unwrap();
    assert!(!b.due(hub, at("2026-11-20", 4, 20)));
}

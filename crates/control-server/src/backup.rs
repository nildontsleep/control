//! Sauvegardes du serveur : la base des comptes (`server.db`) et, pour chaque compte, sa base (écoutes, playlists,
//! tags, progression…) et ses réglages, rangées dans une archive .tgz (chiffrée avec une phrase secrète si
//! l'administrateur en a choisi une), découpée en parties assez petites pour un webhook Discord, et envoyée à ce
//! webhook. Ne sont jamais copiés : les sessions ouvertes, les jetons et clés (GitHub, TMDB, Last.fm, webhook), les
//! cookies YouTube. Même format que `server/backup.js` (chiffrement : `CTRLBK1\n` | sel | iv | étiquette | données).

use std::path::Path;
use std::time::Duration;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use parking_lot::Mutex;
use regex::Regex;
use serde_json::{Value, json};

use crate::hub::Hub;
use crate::serverdb::{self, kv_get, kv_set};
use crate::tar::{TarFile, pack_raw};
use crate::util::{HttpResult, http_error, now_ms};

const MAGIC: &[u8] = b"CTRLBK1\n";
const SECRET_KEYS: &[&str] =
    &["updateToken", "tmdbKey", "backupWebhook", "backupPassphrase", "lastfmKey", "lastfmSecret"];
const KEEP_LOCAL: usize = 2;
const MAX_DB_BYTES: u64 = 200 * 1024 * 1024;
pub const PART_BYTES: usize = 9 * 1024 * 1024;

pub static WEBHOOK: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"^https://(?:(?:ptb|canary)\.)?discord(?:app)?\.com/api/webhooks/\d{15,25}/[\w-]{20,120}$")
        .expect("regex constante")
});

/// Chiffre une archive avec une phrase secrète (scrypt + AES-256-GCM).
pub fn encrypt(data: &[u8], passphrase: &str) -> Vec<u8> {
    let salt: [u8; 16] = rand::random();
    let iv: [u8; 12] = rand::random();
    let key = crate::auth::scrypt_key(passphrase.as_bytes(), &salt, 32);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let out = cipher.encrypt(Nonce::from_slice(&iv), data).unwrap_or_default();
    let (body, tag) = out.split_at(out.len().saturating_sub(16));
    [MAGIC, &salt, &iv, tag, body].concat()
}

pub fn decrypt(blob: &[u8], passphrase: &str) -> anyhow::Result<Vec<u8>> {
    if !blob.starts_with(MAGIC) || blob.len() < MAGIC.len() + 44 {
        anyhow::bail!("Ce fichier n’est pas une sauvegarde chiffrée de control.");
    }
    let m = MAGIC.len();
    let (salt, iv, tag, body) = (&blob[m..m + 16], &blob[m + 16..m + 28], &blob[m + 28..m + 44], &blob[m + 44..]);
    let key = crate::auth::scrypt_key(passphrase.as_bytes(), salt, 32);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    cipher
        .decrypt(Nonce::from_slice(iv), [body, tag].concat().as_slice())
        .map_err(|_| anyhow::anyhow!("Phrase secrète incorrecte, ou fichier abîmé."))
}

/// Les réglages d'un compte sans ce qui est secret (clés d'API, jetons de scrobbling…).
pub fn scrub_settings(text: &str) -> String {
    let Ok(mut data) = serde_json::from_str::<Value>(text) else { return text.to_string() };
    let root = if data.get("settings").is_some_and(|s| !s.is_null()) { &mut data["settings"] } else { &mut data };
    if let Some(media) = root.get_mut("media").and_then(Value::as_object_mut) {
        media.remove("tmdbKey");
    }
    if let Some(update) = root.get_mut("update").and_then(Value::as_object_mut) {
        update.remove("token");
    }
    if let Some(obj) = root.as_object_mut() {
        obj.remove("scrobble");
    }
    data.to_string()
}

fn iso(ms: i64) -> String {
    let t = time::OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        t.year(),
        t.month() as u8,
        t.day(),
        t.hour(),
        t.minute(),
        t.second(),
        t.millisecond()
    )
}

/// Une sauvegarde prête à envoyer.
pub struct Built {
    pub name: String,
    pub parts: Vec<(String, Vec<u8>)>,
    pub size: usize,
    pub manifest: Value,
    pub encrypted: bool,
}

pub struct Backups {
    allow_any_url: bool,
    running: Mutex<()>,
}

struct Settings {
    webhook: String,
    every: String,
    hour: i64,
    passphrase: String,
}

impl Backups {
    pub fn new(allow_any_url: bool) -> Backups {
        Backups { allow_any_url, running: Mutex::new(()) }
    }

    fn settings(hub: &Hub) -> Settings {
        let every = hub.setting_str("backupEvery");
        Settings {
            webhook: hub.setting_str("backupWebhook"),
            every: if every.is_empty() { "off".into() } else { every },
            hour: hub.setting("backupHour").as_i64().unwrap_or(3),
            passphrase: hub.setting_str("backupPassphrase"),
        }
    }

    fn state(hub: &Hub) -> Value {
        kv_get(&hub.db.lock(), "backup_state").and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(json!({}))
    }

    fn save(hub: &Hub, state: &Value) {
        kv_set(&hub.db.lock(), "backup_state", &state.to_string());
    }

    pub fn status(&self, hub: &Hub) -> Value {
        let s = Backups::settings(hub);
        json!({
            "configured": !s.webhook.is_empty(), "every": s.every, "hour": s.hour, "encrypted": !s.passphrase.is_empty(),
            "running": self.running.try_lock().is_none(), "last": Backups::state(hub),
        })
    }

    pub fn valid_url(&self, url: &str) -> bool {
        if self.allow_any_url {
            url.starts_with("http://") || url.starts_with("https://")
        } else {
            WEBHOOK.is_match(url)
        }
    }

    // ─── Contenu ───────────────────────────────────────────────────────────

    /// L'archive (tar non compressé) : server.db, bibliothèques et réglages des comptes, manifeste.
    pub fn collect(&self, hub: &Hub) -> anyhow::Result<(Vec<u8>, Value)> {
        let tmp = tempdir_in(&hub.config.data_dir)?;
        let res = (|| -> anyhow::Result<(Vec<u8>, Value)> {
            let mut files = Vec::new();
            let mut skipped = Vec::new();
            // Base des comptes, sans les sessions ni les secrets.
            let server_copy = tmp.join("server.db");
            serverdb::vacuum_into(&hub.db.lock(), &server_copy)?;
            {
                let copy = rusqlite::Connection::open(&server_copy)?;
                copy.execute_batch("DELETE FROM sessions")?;
                if let Some(text) = kv_get(&copy, "settings")
                    && let Ok(Value::Object(mut s)) = serde_json::from_str::<Value>(&text)
                {
                    for k in SECRET_KEYS {
                        s.remove(*k);
                    }
                    copy.execute("UPDATE kv SET value = ? WHERE key = 'settings'", [Value::Object(s).to_string()])?;
                }
                copy.execute_batch("DELETE FROM kv WHERE key = 'backup_state'; VACUUM")?;
            }
            files.push(TarFile::new("server.db", std::fs::read(&server_copy)?));

            let accounts = hub.accounts.list();
            for user in &accounts {
                let id = user["id"].as_str().unwrap_or_default().to_string();
                let username = user["username"].clone();
                let dir = hub.users_dir.join(&id);
                let db_file = dir.join("library.db");
                if db_file.exists() {
                    let out = tmp.join(format!("{id}.db"));
                    let copied = match hub.loaded(&id) {
                        Some(entry) => serverdb::vacuum_into(&entry.core.db.lock(), &out),
                        None => {
                            rusqlite::Connection::open_with_flags(&db_file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                                .and_then(|db| serverdb::vacuum_into(&db, &out))
                        }
                    };
                    match copied.map_err(anyhow::Error::from).and_then(|_| Ok(std::fs::metadata(&out)?.len())) {
                        Ok(size) if size > MAX_DB_BYTES => skipped.push(json!({
                            "id": id, "username": username,
                            "reason": format!("base trop volumineuse ({} Mo)", size / 1048576),
                        })),
                        Ok(_) => files.push(TarFile::new(format!("users/{id}/library.db"), std::fs::read(&out)?)),
                        Err(err) => {
                            skipped.push(json!({ "id": id, "username": username, "reason": format!("{err:#}") }))
                        }
                    }
                    let _ = std::fs::remove_file(&out);
                }
                let settings = dir.join("settings.json");
                if let Ok(text) = std::fs::read_to_string(&settings) {
                    files.push(TarFile::new(format!("users/{id}/settings.json"), scrub_settings(&text)));
                }
            }
            let manifest = json!({
                "app": "control",
                "version": crate::hub::version(),
                "createdAt": iso(now_ms()),
                "accounts": accounts.iter().map(|u| json!({ "id": u["id"], "username": u["username"] })).collect::<Vec<_>>(),
                "skipped": skipped,
                "note": "Sans sessions, jetons, clés d’API ni cookies. Restauration : voir le README (section Sauvegardes).",
            });
            files.insert(0, TarFile::new("MANIFEST.json", serde_json::to_vec_pretty(&manifest)?));
            Ok((pack_raw(&files)?, manifest))
        })();
        let _ = std::fs::remove_dir_all(&tmp);
        res
    }

    /// Les parties à envoyer (une seule si l'archive tient dans une partie).
    pub fn build(&self, hub: &Hub, part_bytes: usize) -> anyhow::Result<Built> {
        let (tar, manifest) = self.collect(hub)?;
        let mut blob = {
            use std::io::Write as _;
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
            enc.write_all(&tar)?;
            enc.finish()?
        };
        let pass = Backups::settings(hub).passphrase;
        let stamp: String =
            manifest["createdAt"].as_str().unwrap_or("").chars().take(16).filter(|c| c.is_ascii_digit()).collect();
        let mut name = format!("control-backup-{stamp}.tgz");
        if !pass.is_empty() {
            blob = encrypt(&blob, &pass);
            name.push_str(".enc");
        }
        let part_bytes = part_bytes.max(1);
        let total = blob.len().div_ceil(part_bytes).max(1);
        let parts = (0..total)
            .map(|i| {
                let chunk = blob[(i * part_bytes).min(blob.len())..((i + 1) * part_bytes).min(blob.len())].to_vec();
                (if total == 1 { name.clone() } else { format!("{name}.part{:02}", i + 1) }, chunk)
            })
            .collect();
        Ok(Built { name, parts, size: blob.len(), manifest, encrypted: !pass.is_empty() })
    }

    // ─── Envoi ─────────────────────────────────────────────────────────────

    fn post(hub: &Hub, url: &str, content: &str, file: Option<(&str, &[u8])>) -> HttpResult<()> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(120)))
            .build()
            .into();
        let boundary = format!("----control{}", hex::encode(rand::random::<[u8; 12]>()));
        let payload = json!({ "content": content, "username": hub.server_name(), "allowed_mentions": { "parse": [] } });
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"payload_json\"\r\nContent-Type: application/json\r\n\r\n{payload}\r\n"
        )
        .into_bytes();
        if let Some((name, data)) = file {
            let safe = name.replace(['"', '\r', '\n'], "_");
            body.extend_from_slice(
                format!("--{boundary}\r\nContent-Disposition: form-data; name=\"files[0]\"; filename=\"{safe}\"\r\nContent-Type: application/octet-stream\r\n\r\n")
                    .as_bytes(),
            );
            body.extend_from_slice(data);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        for _ in 0..3 {
            let res = agent
                .post(&format!("{url}?wait=true"))
                .header("content-type", &format!("multipart/form-data; boundary={boundary}"))
                .send(&body[..]);
            let mut res = res.map_err(|e| http_error(502, "webhook_failed", format!("Discord injoignable : {e}")))?;
            let status = res.status().as_u16();
            if status == 429 {
                let wait =
                    res.body_mut().read_json::<Value>().ok().and_then(|v| v["retry_after"].as_f64()).unwrap_or(2.0);
                std::thread::sleep(Duration::from_millis((wait.min(30.0) * 1000.0) as u64 + 200));
                continue;
            }
            if status == 404 || status == 401 {
                return Err(http_error(
                    400,
                    "bad_webhook",
                    "Webhook refusé ou supprimé (Discord répond « introuvable »).",
                ));
            }
            if !(200..300).contains(&status) {
                return Err(http_error(502, "webhook_failed", format!("Discord a répondu {status}.")));
            }
            return Ok(());
        }
        Err(http_error(502, "webhook_failed", "Discord limite les envois : réessayez plus tard."))
    }

    /// Un message court pour vérifier que le webhook marche.
    pub fn test(&self, hub: &Hub) -> HttpResult<bool> {
        let webhook = Backups::settings(hub).webhook;
        if webhook.is_empty() {
            return Err(http_error(400, "no_webhook", "Aucun webhook configuré."));
        }
        Backups::post(hub, &webhook, "✅ control : le webhook de sauvegarde fonctionne.", None)?;
        Ok(true)
    }

    /// Fait une sauvegarde maintenant et l'envoie. Une seule à la fois (une demande pendant une sauvegarde attend
    /// celle-ci et rend son résultat).
    pub fn run(&self, hub: &Hub, reason: &str) -> HttpResult<Value> {
        let webhook = Backups::settings(hub).webhook;
        if webhook.is_empty() {
            return Err(http_error(400, "no_webhook", "Aucun webhook configuré."));
        }
        let Some(_guard) = self.running.try_lock() else {
            drop(self.running.lock());
            return Ok(Backups::state(hub));
        };
        let started = now_ms();
        let res = (|| -> HttpResult<Value> {
            let built = self.build(hub, PART_BYTES).map_err(|e| http_error(500, "error", format!("{e:#}")))?;
            let accounts = built.manifest["accounts"].as_array().map(Vec::len).unwrap_or(0);
            let head = format!(
                "💾 **Sauvegarde de {}** ({reason}) · {accounts} compte(s) · {:.1} Mo{}{}",
                hub.server_name(),
                built.size as f64 / 1048576.0,
                if built.encrypted {
                    " · chiffrée"
                } else {
                    " · **non chiffrée** (elle contient les empreintes des mots de passe)"
                },
                if built.parts.len() > 1 { format!(" · {} parties", built.parts.len()) } else { String::new() },
            );
            let n = built.parts.len();
            for (i, (name, data)) in built.parts.iter().enumerate() {
                let content = if i == 0 { head.clone() } else { format!("Partie {}/{n}", i + 1) };
                Backups::post(hub, &webhook, &content, Some((name, data)))?;
            }
            keep_local(&hub.config.data_dir, &built);
            Ok(json!({
                "at": now_ms(), "ok": true, "size": built.size, "parts": n, "accounts": accounts,
                "encrypted": built.encrypted, "tookMs": now_ms() - started,
                "skipped": built.manifest["skipped"].as_array().map(Vec::len).unwrap_or(0),
            }))
        })();
        match res {
            Ok(last) => {
                Backups::save(hub, &last);
                Ok(last)
            }
            Err(err) => {
                Backups::save(hub, &json!({ "at": now_ms(), "ok": false, "error": err.message }));
                Err(err)
            }
        }
    }

    // ─── Planification ─────────────────────────────────────────────────────

    /// Une sauvegarde est-elle due à `now` ? (À l'heure choisie, une fois par jour ou par semaine ; nouvel essai une
    /// heure après un échec.)
    pub fn due(&self, hub: &Hub, now: i64) -> bool {
        let s = Backups::settings(hub);
        let period = match s.every.as_str() {
            "daily" => 24 * 3600 * 1000,
            "weekly" => 7 * 24 * 3600 * 1000,
            _ => return false,
        };
        if s.webhook.is_empty() {
            return false;
        }
        let last = Backups::state(hub);
        let at = last["at"].as_i64().unwrap_or(0);
        if at > 0 && last["ok"] == false && now - at < 3600 * 1000 {
            return false;
        }
        let local = control_core::util::local_datetime(now);
        let Ok(time) = time::Time::from_hms(s.hour.clamp(0, 23) as u8, 0, 0) else { return false };
        let slot = local.replace_time(time).unix_timestamp() * 1000;
        if now < slot {
            return false;
        }
        last["ok"] != true || now - at >= period - 3600 * 1000
    }

    /// Vérification périodique (toutes les 5 minutes) : lance la sauvegarde planifiée si elle est due.
    pub fn tick(&self, hub: &std::sync::Arc<Hub>) {
        if self.running.try_lock().is_none() || !self.due(hub, now_ms()) {
            return;
        }
        let hub = hub.clone();
        let _ = std::thread::Builder::new().name("control-backup".into()).spawn(move || {
            if let Err(err) = hub.backups.run(&hub, "planifiée") {
                eprintln!("Sauvegarde : {}", err.message);
            }
        });
    }
}

/// Garde les deux dernières sauvegardes sur le disque du serveur (en plus de Discord).
fn keep_local(data_dir: &Path, built: &Built) {
    let dir = data_dir.join("backups");
    let data: Vec<u8> = built.parts.iter().flat_map(|(_, d)| d.iter().copied()).collect();
    if crate::util::write_private(&dir.join(&built.name), &data).is_err() {
        return;
    }
    let mut all: Vec<String> = std::fs::read_dir(&dir)
        .map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    all.retain(|n| n.starts_with("control-backup-") && (n.ends_with(".tgz") || n.ends_with(".tgz.enc")));
    all.sort();
    let extra = all.len().saturating_sub(KEEP_LOCAL);
    for old in &all[..extra] {
        let _ = std::fs::remove_file(dir.join(old));
    }
}

/// Dossier temporaire dans le dossier de données (même disque, effacé après usage).
fn tempdir_in(data_dir: &Path) -> std::io::Result<std::path::PathBuf> {
    let dir = data_dir.join("tmp").join(format!("control-backup-{}", hex::encode(rand::random::<[u8; 6]>())));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encryption_roundtrip_and_refusals() {
        let data = "contenu de la sauvegarde ".repeat(1000).into_bytes();
        let blob = encrypt(&data, "phrase secrète");
        assert!(!blob.windows(7).any(|w| w == b"contenu"));
        assert_eq!(decrypt(&blob, "phrase secrète").unwrap(), data);
        assert!(decrypt(&blob, "autre phrase").unwrap_err().to_string().contains("incorrecte"));
        let mut torn = blob.clone();
        let i = torn.len() - 3;
        torn[i] ^= 1;
        assert!(decrypt(&torn, "phrase secrète").is_err());
        assert!(decrypt(b"autre chose", "x").unwrap_err().to_string().contains("pas une sauvegarde"));
    }

    #[test]
    fn decrypts_a_node_backup() {
        // Produit par server/backup.js : encrypt(Buffer.from('bonjour'), 'phrase').toString('base64').
        use base64::Engine as _;
        let blob = base64::engine::general_purpose::STANDARD.decode(NODE_BLOB).unwrap();
        assert_eq!(decrypt(&blob, "phrase").unwrap(), b"bonjour");
    }

    const NODE_BLOB: &str = include_str!("../tests/data/node-backup.b64");

    #[test]
    fn settings_lose_their_secrets() {
        let input = json!({ "settings": { "theme": "light", "media": { "tmdbKey": "K", "other": 1 }, "update": { "token": "T", "auto": true }, "scrobble": { "listenbrainz": { "token": "LB" } } } });
        let out: Value = serde_json::from_str(&scrub_settings(&input.to_string())).unwrap();
        assert_eq!(out["settings"], json!({ "theme": "light", "media": { "other": 1 }, "update": { "auto": true } }));
        assert_eq!(scrub_settings("pas du json"), "pas du json");
    }

    #[test]
    fn only_discord_webhooks() {
        for ok in [
            "https://discord.com/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyzABCDEF-_12",
            "https://ptb.discord.com/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyz",
        ] {
            assert!(WEBHOOK.is_match(ok), "{ok}");
        }
        for bad in [
            "http://discord.com/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyz",
            "https://evil.example/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyz",
            "https://discord.com.evil.example/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyz",
            "https://discord.com/api/webhooks/12/short",
        ] {
            assert!(!WEBHOOK.is_match(bad), "{bad}");
        }
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
    }
}

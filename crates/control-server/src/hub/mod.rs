//! Le cœur du serveur : comptes, réglages généraux, et un noyau (réglages, base, stats, playlists…) par compte,
//! chargé à la demande et déchargé quand il n'est plus utilisé. Chaque compte a sa propre bibliothèque : ses dossiers
//! de musique (attribués par l'administrateur), plus les éventuels dossiers communs à tous.

mod entry;
mod folders;
mod tasks;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use parking_lot::{Mutex, RwLock};
use serde_json::{Map, Value, json};

pub use entry::{Clients, Entry, ServerHost, event_json, frame};

use crate::auth::{Accounts, Throttle, User};
use crate::backup::Backups;
use crate::config::Config;
use crate::presence::Presence;
use crate::serverdb::{self, ServerDb};
use crate::social::Social;
use crate::subsonic::Subsonic;
use crate::updater::{UpdateConfig, Updater, UpdaterOptions};
use crate::util::{HttpResult, http_error, now_ms};
use control_core::core::{Core, CoreOptions, text, truthy};
use control_core::scrobble::LastfmApp;
use control_core::tools::Tools;

/// Version en cours : celle qu'a choisie le lanceur (mise à jour installée), sinon celle du binaire.
pub fn version() -> String {
    std::env::var("CONTROL_STAGED")
        .ok()
        .filter(|v| crate::appstate::is_version(v))
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

const IDLE_MS: i64 = 30 * 60 * 1000;

/// Ce qui change d'un lancement à l'autre (essais).
#[derive(Default)]
pub struct HubOptions {
    /// Adresse de `releases/latest` (sinon `CONTROL_UPDATE_URL`, sinon GitHub).
    pub update_url: Option<String>,
    /// Clés publiques de confiance des mises à jour (sinon `update-key.json`).
    pub update_keys: Option<Vec<String>>,
    /// Lancé par le lanceur : la mise à jour peut redémarrer le serveur.
    pub supervisor: Option<Arc<dyn Fn() + Send + Sync>>,
}

pub struct Hub {
    pub config: Config,
    pub db: ServerDb,
    pub accounts: Arc<Accounts>,
    pub login_throttle: Throttle,
    pub ip_throttle: Throttle,
    pub subsonic_throttle: Throttle,
    pub covers_dir: PathBuf,
    pub avatars_dir: PathBuf,
    pub users_dir: PathBuf,
    pub social: Social,
    pub tools: Arc<Tools>,
    pub updater: Arc<Updater>,
    pub backups: Backups,
    pub presence: Presence,
    pub subsonic: Subsonic,
    pub cache_hits: AtomicU64,
    pub cache_misses: AtomicU64,
    /// Création du premier compte en cours (deux requêtes simultanées ne doivent pas créer deux administrateurs).
    pub initializing: AtomicBool,
    settings: RwLock<Map<String, Value>>,
    entries: Mutex<HashMap<String, Arc<Entry>>>,
    watchers: Mutex<Vec<notify::RecommendedWatcher>>,
    rescan_at: AtomicI64,
    closed: AtomicBool,
    me: Weak<Hub>,
}

fn defaults(config: &Config) -> Map<String, Value> {
    let v = json!({
        "serverName": "control",
        "registrationEnabled": false,
        "allowUserDownloads": false,
        "allowUserUploads": false,
        "uploadMaxMb": 500,
        "subsonicEnabled": true,
        "libraryFolders": [],
        "usersRoot": "",
        "downloadDir": config.data_dir.join("downloads").to_string_lossy(),
        "tmdbKey": config.tmdb_key,
        "updateMode": "notify",
        "updateHour": 4,
        "updateToken": "",
        "backupWebhook": "",
        "backupEvery": "off",
        "backupHour": 3,
        "backupPassphrase": "",
        "lastfmKey": "",
        "lastfmSecret": "",
    });
    v.as_object().cloned().unwrap_or_default()
}

fn clamp_int(v: &Value, lo: i64, hi: i64, fallback: i64) -> i64 {
    let n = match v {
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => s.trim().parse().unwrap_or(f64::NAN),
        Value::Bool(b) => *b as i64 as f64,
        _ => f64::NAN,
    };
    let n = n.floor();
    let n = if n.is_finite() && n != 0.0 { n as i64 } else { fallback };
    n.clamp(lo, hi)
}

impl Hub {
    pub fn new(config: Config, options: HubOptions) -> anyhow::Result<Arc<Hub>> {
        std::fs::create_dir_all(&config.data_dir)?;
        let db: ServerDb = Arc::new(Mutex::new(serverdb::open(&config.data_dir.join("server.db"))?));
        let accounts = Arc::new(Accounts::new(db.clone(), config.session_days));
        let data = config.data_dir.clone();
        let mut settings = defaults(&config);
        if let Some(saved) = serverdb::kv_get(&db.lock(), "settings")
            && let Ok(Value::Object(saved)) = serde_json::from_str::<Value>(&saved)
        {
            settings.extend(saved);
        }
        if !settings.get("libraryFolders").is_some_and(Value::is_array) {
            settings.insert("libraryFolders".into(), json!([]));
        }
        let hub = Arc::new_cyclic(|me: &Weak<Hub>| {
            let tools_hub = me.clone();
            let tools = Arc::new(Tools::new(
                data.join("bin"),
                data.join("tmp"),
                Arc::new(move |event| {
                    if let Some(hub) = tools_hub.upgrade() {
                        let (channel, payload) = event_json(event);
                        hub.broadcast(channel, &payload);
                    }
                }),
            ));
            let cfg_hub = me.clone();
            let state_hub = me.clone();
            let idle_hub = me.clone();
            let backup_db = db.clone();
            let backup_dir = data.clone();
            let updater = Arc::new(Updater::new(UpdaterOptions {
                version: version(),
                data_dir: data.clone(),
                api_url: options.update_url.clone(),
                public_keys: options.update_keys.clone(),
                settings: Arc::new(move || cfg_hub.upgrade().map(|h| h.update_config()).unwrap_or_default()),
                restart: options.supervisor.clone(),
                backup: Arc::new(move || crate::updater::backup_db(&backup_db.lock(), &backup_dir, &version())),
                idle: Arc::new(move || idle_hub.upgrade().is_some_and(|h| h.connected_clients() == 0)),
                on_state: Arc::new(move |state: &Value| {
                    if let Some(h) = state_hub.upgrade() {
                        h.to_admins("update:state", state);
                    }
                }),
            }));
            Hub {
                social: Social::new(db.clone(), accounts.clone(), data.join("avatars")),
                accounts,
                login_throttle: Throttle::new(8, 10 * 60 * 1000),
                ip_throttle: Throttle::new(40, 10 * 60 * 1000),
                subsonic_throttle: Throttle::new(30, 10 * 60 * 1000),
                covers_dir: data.join("covers"),
                avatars_dir: data.join("avatars"),
                users_dir: data.join("users"),
                tools,
                updater,
                backups: Backups::new(config.backup_any_url),
                presence: Presence::default(),
                subsonic: Subsonic::new(&data),
                cache_hits: AtomicU64::new(0),
                cache_misses: AtomicU64::new(0),
                initializing: AtomicBool::new(false),
                settings: RwLock::new(settings),
                entries: Mutex::new(HashMap::new()),
                watchers: Mutex::new(Vec::new()),
                rescan_at: AtomicI64::new(0),
                closed: AtomicBool::new(false),
                me: me.clone(),
                db,
                config,
            }
        });
        Ok(hub)
    }

    // ─── Réglages du serveur ───────────────────────────────────────────────

    pub fn settings(&self) -> Map<String, Value> {
        self.settings.read().clone()
    }

    pub fn setting(&self, key: &str) -> Value {
        self.settings.read().get(key).cloned().unwrap_or(Value::Null)
    }

    pub fn setting_str(&self, key: &str) -> String {
        text(&self.setting(key))
    }

    pub fn setting_bool(&self, key: &str) -> bool {
        truthy(&self.setting(key))
    }

    pub fn server_name(&self) -> String {
        let name = self.setting_str("serverName");
        if name.is_empty() { "control".into() } else { name }
    }

    pub fn upload_max_mb(&self) -> i64 {
        self.setting("uploadMaxMb").as_f64().filter(|n| *n > 0.0).map(|n| n as i64).unwrap_or(500).min(20480)
    }

    pub fn patch_settings(&self, patch: &Value) -> HttpResult<()> {
        let Some(patch) = patch.as_object() else { return Ok(()) };
        if let Some(v) = patch.get("backupWebhook") {
            let url = text(v).trim().to_string();
            if !url.is_empty() && !self.backups.valid_url(&url) {
                return Err(http_error(
                    400,
                    "bad_webhook",
                    "Adresse de webhook invalide (https://discord.com/api/webhooks/…).",
                ));
            }
        }
        const ALLOWED: &[&str] = &[
            "serverName",
            "registrationEnabled",
            "allowUserDownloads",
            "allowUserUploads",
            "uploadMaxMb",
            "subsonicEnabled",
            "libraryFolders",
            "downloadDir",
            "tmdbKey",
            "usersRoot",
            "updateMode",
            "updateHour",
            "updateToken",
            "backupWebhook",
            "backupEvery",
            "backupHour",
            "backupPassphrase",
            "lastfmKey",
            "lastfmSecret",
        ];
        let saved = {
            let mut s = self.settings.write();
            for key in ALLOWED {
                let Some(v) = patch.get(*key) else { continue };
                let value = match *key {
                    "libraryFolders" => Value::Array(
                        v.as_array()
                            .map(|a| a.iter().filter(|x| x.as_str().is_some_and(|s| !s.is_empty())).cloned().collect())
                            .unwrap_or_default(),
                    ),
                    "registrationEnabled" | "allowUserDownloads" | "allowUserUploads" | "subsonicEnabled" => {
                        Value::Bool(truthy(v))
                    }
                    "uploadMaxMb" => json!(clamp_int(v, 1, 20480, 500)),
                    "serverName" => {
                        let name: String = text(v).trim().chars().take(60).collect();
                        json!(if name.trim().is_empty() { "control".to_string() } else { name.trim().to_string() })
                    }
                    "updateMode" => json!(if ["off", "notify", "auto"].contains(&text(v).as_str()) {
                        text(v)
                    } else {
                        "notify".into()
                    }),
                    "updateHour" | "backupHour" => json!(clamp_int(v, 0, 23, 0)),
                    "backupEvery" => json!(if ["off", "daily", "weekly"].contains(&text(v).as_str()) {
                        text(v)
                    } else {
                        "off".into()
                    }),
                    _ => json!(text(v).trim()),
                };
                s.insert(key.to_string(), value);
            }
            Value::Object(s.clone()).to_string()
        };
        serverdb::kv_set(&self.db.lock(), "settings", &saved);
        if patch.contains_key("libraryFolders") || patch.contains_key("downloadDir") {
            self.sync_folders();
        }
        if ["updateMode", "updateHour", "updateToken"].iter().any(|k| patch.contains_key(*k)) {
            self.updater.refresh();
        }
        Ok(())
    }

    /// Les réglages tels que la page d'administration les voit (les secrets ne quittent jamais le serveur).
    pub fn public_settings(&self) -> Value {
        let mut s = self.settings();
        let has = |s: &Map<String, Value>, k: &str| s.get(k).is_some_and(truthy);
        let has_secret = has(&s, "lastfmSecret");
        let has_webhook = has(&s, "backupWebhook");
        let has_pass = has(&s, "backupPassphrase");
        let has_token = has(&s, "updateToken") || !self.config.update_token.is_empty();
        for k in ["updateToken", "backupWebhook", "backupPassphrase", "lastfmSecret"] {
            s.remove(k);
        }
        s.insert("hasLastfmSecret".into(), json!(has_secret));
        s.insert("hasBackupWebhook".into(), json!(has_webhook));
        s.insert("hasBackupPassphrase".into(), json!(has_pass));
        s.insert("hasUpdateToken".into(), json!(has_token));
        if !self.config.update.is_empty() {
            s.insert("updateMode".into(), json!(self.config.update));
        }
        s.insert("updateLocked".into(), json!(!self.config.update.is_empty()));
        Value::Object(s)
    }

    pub fn update_config(&self) -> UpdateConfig {
        let mode =
            if self.config.update.is_empty() { self.setting_str("updateMode") } else { self.config.update.clone() };
        let token = if self.config.update_token.is_empty() {
            self.setting_str("updateToken")
        } else {
            self.config.update_token.clone()
        };
        UpdateConfig {
            mode,
            hour: self.setting("updateHour").as_i64().unwrap_or(4),
            token,
            locked: !self.config.update.is_empty(),
        }
    }

    pub fn lastfm_app(&self) -> Option<LastfmApp> {
        let (key, secret) = (self.setting_str("lastfmKey"), self.setting_str("lastfmSecret"));
        (!key.is_empty() && !secret.is_empty()).then_some(LastfmApp { key, secret })
    }

    // ─── Noyaux par compte ─────────────────────────────────────────────────

    /// Le compte déjà chargé, sans le charger.
    pub fn loaded(&self, id: &str) -> Option<Arc<Entry>> {
        self.entries.lock().get(id).cloned()
    }

    pub fn loaded_entries(&self) -> Vec<Arc<Entry>> {
        self.entries.lock().values().cloned().collect()
    }

    /// Le noyau d'un compte (créé au besoin). Bloquant.
    pub fn entry(&self, user: &User) -> HttpResult<Arc<Entry>> {
        let entry = {
            let mut entries = self.entries.lock();
            if let Some(e) = entries.get(&user.id) {
                e.touch();
                return Ok(e.clone());
            }
            let clients = Arc::new(Clients::default());
            let emit_clients = clients.clone();
            let key_hub = self.me.clone();
            let lastfm_hub = self.me.clone();
            let dir = self.users_dir.join(&user.id);
            let core = Core::new(CoreOptions {
                dir: dir.clone(),
                covers_dir: Some(self.covers_dir.clone()),
                tools: Some(self.tools.clone()),
                emit: Arc::new(move |event| {
                    let (channel, payload) = event_json(event);
                    emit_clients.send(channel, &payload, None);
                }),
                default_download_dir: self.download_dir_for(user),
                tmdb_fallback: Some(Arc::new(move || {
                    key_hub.upgrade().map(|h| h.setting_str("tmdbKey")).unwrap_or_default()
                })),
                cookies_file: Some(dir.join("cookies.txt")),
                lastfm_keys: Some(Arc::new(move || lastfm_hub.upgrade().and_then(|h| h.lastfm_app()))),
                guard_url: Some(Arc::new(|url: &str| Ok(control_core::urlguard::check_url(url)?))),
                host: Arc::new(ServerHost { hub: self.me.clone(), user_id: user.id.clone(), clients: clients.clone() }),
            })
            .map_err(|e| http_error(500, "error", format!("Bibliothèque du compte illisible : {e:#}")))?;
            let entry = Arc::new(Entry {
                id: user.id.clone(),
                clients,
                core,
                last_used: AtomicI64::new(now_ms()),
                tracks_cache: Mutex::new(None),
                sub_model: Mutex::new(None),
            });
            entries.insert(user.id.clone(), entry.clone());
            entry
        };
        self.mirror(&entry);
        self.refresh(&entry, self.roots_for(&user.id));
        Ok(entry)
    }

    /// Envoie un événement à tous les comptes connectés (outils installés, par exemple).
    pub fn broadcast(&self, channel: &str, payload: &Value) {
        for e in self.loaded_entries() {
            e.emit(channel, payload);
        }
    }

    /// … ou aux seuls administrateurs (mises à jour du serveur).
    pub fn to_admins(&self, channel: &str, payload: &Value) {
        for e in self.loaded_entries() {
            if self.accounts.by_id(&e.id).is_some_and(|u| u.is_admin()) {
                e.emit(channel, payload);
            }
        }
    }

    /// Nombre de pages ouvertes (flux d'événements) : zéro, c'est le bon moment pour redémarrer.
    pub fn connected_clients(&self) -> usize {
        self.loaded_entries().iter().map(|e| e.clients.len()).sum()
    }

    /// Coupe ce qui est ouvert pour un compte (désactivé, supprimé) : connexions en direct et noyau.
    pub fn drop_user(&self, id: &str) {
        let entry = self.entries.lock().remove(id);
        if let Some(e) = entry {
            e.clients.close_all();
            e.core.close();
        }
    }

    /// Efface le dossier d'un compte supprimé.
    pub fn forget_user(&self, id: &str) {
        self.drop_user(id);
        let _ = std::fs::remove_dir_all(self.users_dir.join(id));
        self.social.forget(id);
    }

    /// La liste des morceaux d'un compte, en JSON compressé `{ ok: true, result: [...] }`, gardée tant que rien n'a
    /// changé dans sa base.
    pub fn tracks_payload(&self, entry: &Entry) -> Arc<Vec<u8>> {
        let version = entry.db_version();
        if let Some((v, gz)) = entry.tracks_cache.lock().as_ref()
            && *v == version
        {
            self.cache_hits.fetch_add(1, Ordering::SeqCst);
            return gz.clone();
        }
        self.cache_misses.fetch_add(1, Ordering::SeqCst);
        let body = json!({ "ok": true, "result": entry.core.library.tracks() });
        let gz = Arc::new(crate::http::gzip(&serde_json::to_vec(&body).unwrap_or_default()));
        *entry.tracks_cache.lock() = Some((version, gz.clone()));
        gz
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Arrête tout : minuteries, surveillance, noyaux chargés.
    pub fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.updater.stop();
        self.watchers.lock().clear();
        let all: Vec<Arc<Entry>> = self.entries.lock().drain().map(|(_, e)| e).collect();
        for e in all {
            e.clients.close_all();
            e.core.close();
        }
    }
}

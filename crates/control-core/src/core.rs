//! Le noyau d'un utilisateur : réglages, base, bibliothèque, tags, paroles, statistiques, téléchargements, films et
//! séries, scrobbling, normalisation du volume. L'application de bureau en crée un seul ; le serveur web, un par compte.
//!
//! [`Core::dispatch`] reprend les canaux IPC d'origine (`Core.register()` de `src/main/core.js`, plus ceux du service
//! films/séries) : mêmes noms, mêmes arguments JSON, mêmes réponses. Ce qui dépend de l'hôte (dossiers analysés, écho
//! aux autres fenêtres ou onglets, garde des adresses) passe par le trait [`Host`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Value, json};

use crate::covers::{self, Covers};
use crate::downloads::job::AddOptions;
use crate::downloads::{Downloads, DownloadsConfig};
use crate::library::{Library, SharedDb};
use crate::loudness::Loudness;
use crate::lyrics::Lyrics;
use crate::media::{KeyProvider, MediaOptions, MediaService};
use crate::model::{Emit, Event, PlaylistKind, SmartSpec, TrackId};
use crate::scrobble::{GuardFn, KeysFn, Scrobbler, ScrobblerOptions};
use crate::store::Store;
use crate::tags::{CoverSource, TagEdit, Tags};
use crate::tools::{Tool, Tools};
use crate::util::{now_ms, path_key};

/// Erreur d'un appel : statut HTTP (lu par le serveur web), code court et message pour l'utilisateur.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct RpcError {
    pub status: u16,
    pub code: String,
    pub message: String,
}

impl RpcError {
    pub fn new(status: u16, code: &str, message: impl Into<String>) -> RpcError {
        RpcError { status, code: code.to_string(), message: message.into() }
    }

    pub fn unknown(channel: &str) -> RpcError {
        RpcError::new(404, "unknown_channel", format!("Commande inconnue : {channel}"))
    }

    pub fn bad_request(message: impl Into<String>) -> RpcError {
        RpcError::new(400, "bad_request", message)
    }

    /// Erreur inattendue : le serveur n'en montre pas le détail.
    pub fn internal(message: impl Into<String>) -> RpcError {
        RpcError::new(500, "error", message)
    }
}

/// Ce que le noyau attend de son hôte (application de bureau ou serveur web).
pub trait Host: Send + Sync {
    /// Dossiers à analyser pour `lib:scan`.
    fn scan_roots(&self) -> Vec<String>;
    /// Effets d'un changement de réglages propres à l'hôte (`scan` : relancer l'analyse si les dossiers changent).
    fn apply_setting_effects(&self, _patch: &Value, _scan: bool) {}
    /// Envoie un événement à toutes les fenêtres ou onglets sauf `sender` (celui qui a fait le changement).
    fn emit_others(&self, sender: &str, channel: &str, payload: &Value);
    /// Réglages remis à zéro.
    fn on_reset(&self) {}
    /// Garde des liens de téléchargement (serveur : publics seulement). Par défaut, tout lien est permis.
    fn guard_url(&self, url: &str) -> Result<String, RpcError> {
        Ok(url.to_string())
    }
}

/// Hôte minimal : pas de dossiers, pas d'écho (tests, outils).
pub struct NoHost;

impl Host for NoHost {
    fn scan_roots(&self) -> Vec<String> {
        Vec::new()
    }

    fn emit_others(&self, _sender: &str, _channel: &str, _payload: &Value) {}
}

/// Paramètres de [`Core::new`].
pub struct CoreOptions {
    /// Dossier de données de cet utilisateur (réglages, base, vignettes, téléchargements en cours).
    pub dir: PathBuf,
    /// Pochettes (identifiées par leur contenu : elles peuvent être partagées entre utilisateurs). Sinon `dir/covers`.
    pub covers_dir: Option<PathBuf>,
    /// Outils déjà installés (partagés) ; sinon créés dans `dir/bin`.
    pub tools: Option<Arc<Tools>>,
    pub emit: Emit,
    /// Dossier des téléchargements quand le réglage est vide.
    pub default_download_dir: PathBuf,
    /// Clé TMDB fournie d'office (serveur web) quand le compte n'a pas la sienne.
    pub tmdb_fallback: Option<KeyProvider>,
    /// cookies.txt du compte pour yt-dlp (serveur : envoyé par l'utilisateur ; bureau : aucun).
    pub cookies_file: Option<PathBuf>,
    /// Application Last.fm du serveur.
    pub lastfm_keys: Option<KeysFn>,
    /// Garde des adresses ListenBrainz personnalisées (serveur : publiques seulement).
    pub guard_url: Option<GuardFn>,
    pub host: Arc<dyn Host>,
}

pub struct Core {
    pub dir: PathBuf,
    pub store: Store,
    pub db: SharedDb,
    pub library: Arc<Library>,
    pub tags: Tags,
    pub lyrics: Lyrics,
    pub tools: Arc<Tools>,
    pub downloads: Downloads,
    pub media: MediaService,
    pub scrobbler: Scrobbler,
    pub loudness: Loudness,
    emit: Emit,
    host: Arc<dyn Host>,
    closed: Mutex<bool>,
}

/// Canaux « invoke » (une réponse est attendue).
pub const HANDLE_CHANNELS: &[&str] = &[
    "lib:tracks",
    "lib:playlists",
    "lib:scan",
    "lib:add-paths",
    "lib:remove-folder",
    "lib:remove-tracks",
    "lib:play-start",
    "lyrics:get",
    "lyrics:set",
    "lyrics:save-file",
    "stats:get",
    "tags:read",
    "tags:write",
    "tags:undo",
    "tags:stage-cover",
    "tags:search-covers",
    "pl:create",
    "pl:rename",
    "pl:update",
    "pl:set-cover",
    "pl:rules",
    "pl:delete",
    "pl:add",
    "pl:remove",
    "pl:reorder",
    "tools:status",
    "tools:install",
    "tools:update",
    "dl:list",
    "dl:info",
    "dl:add",
    "dl:search",
    "loudness:progress",
    "loudness:levels",
    "scrobble:status",
    "scrobble:connect-listenbrainz",
    "scrobble:lastfm-start",
    "scrobble:lastfm-finish",
    "scrobble:set-lastfm-keys",
    "scrobble:disconnect",
    "media:home",
    "media:list",
    "media:details",
    "media:season",
    "media:search",
    "media:test-key",
    "media:clear-cache",
    "watch:save",
    "watch:show",
    "watch:continue",
    "watch:history",
    "watch:remove",
    "watch:clear",
    "watchlist:list",
    "watchlist:toggle",
];

/// Canaux « send » (sans réponse).
pub const SEND_CHANNELS: &[&str] =
    &["settings:set", "settings:reset", "lib:play-end", "dl:cancel", "dl:retry", "dl:remove", "dl:clear"];

// ─── Lecture des arguments JSON ──────────────────────────────────────────────

const NULL: Value = Value::Null;

fn arg(args: &[Value], i: usize) -> &Value {
    args.get(i).unwrap_or(&NULL)
}

/// Un entier (`Number.isInteger`).
pub fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => {
            n.as_i64().or_else(|| n.as_f64().filter(|f| f.fract() == 0.0 && f.is_finite()).map(|f| f as i64))
        }
        _ => None,
    }
}

fn number(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s.trim().parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// `String(v || '')` : le texte d'une valeur (vide pour null, undefined ou false).
pub fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(true) => "true".into(),
        _ => String::new(),
    }
}

/// Valeur JavaScript « vraie ».
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// Liste d'identifiants entiers (les autres valeurs sont ignorées).
pub fn ids(v: &Value) -> Vec<i64> {
    v.as_array().map(|a| a.iter().filter_map(int).collect()).unwrap_or_default()
}

fn to_value<T: Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// `{ ok: true, data }` ou `{ ok: false, message }`.
fn wrap<T: Serialize, E: std::fmt::Display>(r: Result<T, E>) -> Value {
    match r {
        Ok(data) => json!({ "ok": true, "data": to_value(data) }),
        Err(err) => json!({ "ok": false, "message": err.to_string() }),
    }
}

/// Décode une image envoyée par le navigateur (`data:image/…;base64,…`), 12 Mo au plus.
pub fn image_from_data_url(src: &Value) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let s = src.as_str()?;
    let rest =
        ["jpeg", "jpg", "png", "webp"].iter().find_map(|t| s.strip_prefix(&format!("data:image/{t};base64,")))?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=') {
        return None;
    }
    let data = base64::engine::general_purpose::STANDARD.decode(rest).ok()?;
    (!data.is_empty() && data.len() <= covers::MAX_IMAGE_BYTES).then_some(data)
}

/// L'image se décode-t-elle (ou est-ce du WebP, que le navigateur sait afficher) ?
pub fn decodable_image(data: &[u8]) -> bool {
    let webp = data.len() > 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP";
    webp || image::load_from_memory(data).is_ok()
}

impl Core {
    pub fn new(o: CoreOptions) -> anyhow::Result<Core> {
        std::fs::create_dir_all(&o.dir)?;
        let dir = o.dir;
        let store = Store::open(dir.join("settings.json"));
        store.spawn_writer();
        let db: SharedDb = Arc::new(Mutex::new(crate::db::open(&dir.join("library.db"))?));
        let covers = Covers::new(o.covers_dir.unwrap_or_else(|| dir.join("covers")));
        let library = Arc::new(Library::new(db.clone(), covers, dir.join("thumbs"), o.emit.clone()));
        let tags = Tags::new(library.clone(), o.emit.clone());
        let lyrics = Lyrics::new(db.clone());
        let tools = o.tools.unwrap_or_else(|| Arc::new(Tools::new(dir.join("bin"), dir.join("tmp"), o.emit.clone())));
        let downloads = Downloads::new(DownloadsConfig {
            tools: tools.clone(),
            library: library.clone(),
            store: store.clone(),
            file: dir.join("downloads.json"),
            emit: o.emit.clone(),
            cookies_file: o.cookies_file,
            default_dir: o.default_download_dir,
        });
        let media = MediaService::with_options(
            db.clone(),
            store.clone(),
            o.emit.clone(),
            MediaOptions { fallback_key: o.tmdb_fallback, ..MediaOptions::default() },
        );
        let scrobbler = Scrobbler::new(ScrobblerOptions {
            dir: dir.clone(),
            db: db.clone(),
            emit: o.emit.clone(),
            keys: o.lastfm_keys,
            guard: o.guard_url,
        });
        scrobbler.resume();
        let ffmpeg_tools = tools.clone();
        let loudness =
            Loudness::new(db.clone(), store.clone(), Arc::new(move || ffmpeg_tools.path(Tool::Ffmpeg)), o.emit.clone());
        Ok(Core {
            dir,
            store,
            db,
            library,
            tags,
            lyrics,
            tools,
            downloads,
            media,
            scrobbler,
            loudness,
            emit: o.emit,
            host: o.host,
            closed: Mutex::new(false),
        })
    }

    /// Ferme ce que le noyau garde ouvert (serveur : utilisateur inactif).
    pub fn close(&self) {
        let mut closed = self.closed.lock();
        if *closed {
            return;
        }
        *closed = true;
        self.loudness.close();
        self.scrobbler.close();
        self.library.close();
        self.downloads.kill_all();
        self.downloads.flush();
        self.store.flush();
    }

    fn emit_json(&self, channel: &'static str, payload: Value) {
        (self.emit)(Event::Json(channel, payload));
    }

    /// Le canal existe-t-il ? (`send` : canal sans réponse.)
    pub fn has_channel(channel: &str, send: bool) -> bool {
        if send { SEND_CHANNELS.contains(&channel) } else { HANDLE_CHANNELS.contains(&channel) }
    }

    /// Appelle un canal d'origine, depuis une fenêtre anonyme.
    pub fn dispatch(&self, channel: &str, args: &[Value]) -> Result<Value, RpcError> {
        self.dispatch_from("", channel, args)
    }

    /// Appelle un canal d'origine ; `sender` identifie la fenêtre ou l'onglet qui demande (pour l'écho des réglages).
    /// Les canaux « send » rendent `null`.
    pub fn dispatch_from(&self, sender: &str, channel: &str, args: &[Value]) -> Result<Value, RpcError> {
        let a = |i: usize| arg(args, i);
        let lib = &self.library;
        Ok(match channel {
            // ─── Réglages ───────────────────────────────────────────────────
            "settings:set" => {
                let patch = a(0);
                if patch.is_object() {
                    self.store.patch(patch);
                    if patch.get("normalize").is_some() {
                        self.loudness.kick();
                    }
                    self.host.apply_setting_effects(patch, true);
                    self.host.emit_others(sender, "settings:changed", patch);
                }
                Value::Null
            }
            "settings:reset" => {
                self.store.reset();
                self.emit_json("settings:reset", Value::Null);
                self.host.on_reset();
                Value::Null
            }

            // ─── Bibliothèque ───────────────────────────────────────────────
            "lib:tracks" => to_value(lib.tracks()),
            "lib:playlists" => self.playlists_value(),
            "lib:scan" => to_value(lib.scan(&self.host.scan_roots())),
            "lib:add-paths" => {
                let paths: Vec<String> = a(0)
                    .as_array()
                    .map(|v| v.iter().filter_map(|p| p.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                to_value(lib.add_paths(&paths, None))
            }
            "lib:remove-folder" => {
                let folder = text(a(0));
                let folders: Vec<Value> = self
                    .store
                    .get("libraryFolders")
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|f| f.as_str() != Some(folder.as_str()))
                    .collect();
                let patch = json!({ "libraryFolders": folders });
                self.store.patch(&patch);
                self.emit_json("settings:changed", patch.clone());
                self.host.apply_setting_effects(&patch, false);
                // Retire de la bibliothèque les morceaux de ce dossier (les fichiers restent intacts).
                let abs = std::path::absolute(&folder).unwrap_or_else(|_| PathBuf::from(&folder));
                let prefix =
                    path_key(&format!("{}{}", abs.to_string_lossy(), std::path::MAIN_SEPARATOR)).to_lowercase();
                let ids: Vec<TrackId> = lib
                    .tracks()
                    .into_iter()
                    .filter(|t| path_key(&t.path).to_lowercase().starts_with(&prefix))
                    .map(|t| t.id)
                    .collect();
                if !ids.is_empty() {
                    lib.remove_tracks(&ids, false);
                }
                Value::Array(folders)
            }
            "lib:remove-tracks" => {
                lib.remove_tracks(&ids(a(0)), truthy(a(1)));
                Value::Null
            }
            "lib:play-start" => {
                let Some(id) = int(a(0)) else { return Ok(Value::Null) };
                let play = lib.play_start(id, number(a(1)));
                if let Some(play) = play {
                    self.scrobbler.started(play, id);
                }
                to_value(play)
            }
            "lib:play-end" => {
                if let Some(play) = int(a(0)) {
                    let seconds = number(a(1));
                    lib.play_end(play, seconds, truthy(a(2)));
                    self.scrobbler.ended(play, seconds);
                }
                Value::Null
            }

            // ─── Paroles ────────────────────────────────────────────────────
            "lyrics:get" | "lyrics:set" | "lyrics:save-file" => {
                let Some(id) = int(a(0)) else { return Ok(json!({ "ok": false, "message": "Morceau invalide." })) };
                let res = match channel {
                    "lyrics:get" => self.lyrics.get(id, truthy(a(1))).map(to_value),
                    "lyrics:set" => {
                        let t: String = text(a(1)).chars().take(200_000).collect();
                        self.lyrics.set(id, &t).map(to_value)
                    }
                    _ => self.lyrics.save_sidecar(id).map(|p| Value::String(p.to_string_lossy().into_owned())),
                };
                match res {
                    Ok(data) => json!({ "ok": true, "data": data }),
                    Err(err) => json!({ "ok": false, "message": err.to_string(), "code": err.code() }),
                }
            }

            // ─── Statistiques ───────────────────────────────────────────────
            "stats:get" => {
                let range = a(0);
                let db = self.db.lock();
                wrap(crate::stats::get(&db, &text(&range["from"]), &text(&range["to"]), 10, now_ms()))
            }

            // ─── Éditeur de tags ────────────────────────────────────────────
            "tags:read" => {
                let list: Vec<i64> = ids(a(0)).into_iter().take(5000).collect();
                json!({ "ok": true, "data": to_value(self.tags.read(&list)) })
            }
            "tags:write" => {
                let edits: Vec<TagEdit> = a(0)
                    .as_array()
                    .map(|v| {
                        v.iter()
                            .filter(|x| int(&x["id"]).is_some())
                            .filter_map(|x| serde_json::from_value(x.clone()).ok())
                            .collect()
                    })
                    .unwrap_or_default();
                json!({ "ok": true, "data": to_value(self.tags.write(&edits)) })
            }
            "tags:undo" => json!({ "ok": true, "data": to_value(self.tags.undo()) }),
            "tags:stage-cover" => {
                let src: CoverSource = if a(0).is_object() {
                    serde_json::from_value(a(0).clone()).unwrap_or_default()
                } else {
                    CoverSource::default()
                };
                wrap(self.tags.stage_cover(&src).map_err(|e| format!("{e:#}")))
            }
            "tags:search-covers" => {
                let q = a(0);
                let artist: String = text(&q["artist"]).chars().take(200).collect();
                let album: String = text(&q["album"]).chars().take(200).collect();
                wrap(self.tags.search_covers(&artist, &album).map_err(|e| format!("{e:#}")))
            }

            // ─── Playlists ──────────────────────────────────────────────────
            "pl:create" => {
                let kind = PlaylistKind::parse(&text(a(1)));
                let rules: Option<SmartSpec> =
                    if a(2).is_object() { serde_json::from_value(a(2).clone()).ok() } else { None };
                let id = lib.create_playlist(&text(a(0)), kind, rules.as_ref());
                let tracks = ids(a(3));
                if let Some(id) = id
                    && !tracks.is_empty()
                {
                    lib.add_to_playlist(id, &tracks);
                }
                to_value(id)
            }
            "pl:rename" => {
                if let Some(id) = int(a(0)) {
                    lib.rename_playlist(id, &text(a(1)));
                }
                Value::Null
            }
            "pl:update" => {
                let (Some(id), Some(patch)) = (int(a(0)), a(1).as_object()) else { return Ok(Value::Bool(false)) };
                let name = patch.get("name").filter(|v| !v.is_null()).map(text);
                let description = patch.get("description").filter(|v| !v.is_null()).map(text);
                let shared = patch.get("shared").filter(|v| !v.is_null()).map(truthy);
                Value::Bool(lib.update_playlist(id, name.as_deref(), description.as_deref(), shared))
            }
            "pl:set-cover" => self.set_playlist_cover(a(0), a(1)),
            "pl:rules" => {
                if let (Some(id), Ok(rules)) = (int(a(0)), serde_json::from_value::<SmartSpec>(a(1).clone())) {
                    lib.set_playlist_rules(id, &rules);
                }
                Value::Null
            }
            "pl:delete" => {
                if let Some(id) = int(a(0)) {
                    lib.delete_playlist(id);
                }
                Value::Null
            }
            "pl:add" => {
                if let Some(id) = int(a(0)) {
                    lib.add_to_playlist(id, &ids(a(1)));
                }
                Value::Null
            }
            "pl:remove" => {
                lib.remove_from_playlist(&ids(a(0)));
                Value::Null
            }
            "pl:reorder" => {
                if let Some(id) = int(a(0)) {
                    lib.reorder_playlist(id, &ids(a(1)));
                }
                Value::Null
            }

            // ─── Outils et téléchargements ──────────────────────────────────
            "tools:status" => to_value(self.tools.status()),
            "tools:install" => match self.tools.install(&[Tool::Ytdlp, Tool::Ffmpeg, Tool::Deno]) {
                Ok(()) => json!({ "ok": true }),
                Err(err) => json!({ "ok": false, "error": format!("{err:#}") }),
            },
            "tools:update" => to_value(self.tools.update()),
            "dl:list" => to_value(self.downloads.list()),
            "dl:info" => {
                let res = self
                    .host
                    .guard_url(&text(a(0)))
                    .map_err(|e| e.message)
                    .and_then(|url| self.downloads.info(&url).map_err(|e| format!("{e:#}")));
                match res {
                    Ok(info) => json!({ "ok": true, "info": to_value(info) }),
                    Err(error) => json!({ "ok": false, "error": error }),
                }
            }
            "dl:add" => {
                let opts = a(0);
                if !opts.is_object() {
                    return Err(RpcError::new(400, "bad_request", "Demande invalide."));
                }
                let url = self.host.guard_url(&text(&opts["url"]))?;
                let mut add = AddOptions::from_value(opts);
                add.url = url;
                to_value(self.downloads.add(add))
            }
            "dl:search" => match self.downloads.search(&text(a(0)), &text(a(1)), 12) {
                Ok(results) => json!({ "ok": true, "results": to_value(results) }),
                Err(err) => json!({ "ok": false, "error": format!("{err:#}") }),
            },
            "dl:cancel" | "dl:retry" | "dl:remove" => {
                if let Some(id) = int(a(0)) {
                    match channel {
                        "dl:cancel" => self.downloads.cancel(id),
                        "dl:retry" => self.downloads.retry(id),
                        _ => self.downloads.remove(id),
                    }
                }
                Value::Null
            }
            "dl:clear" => {
                self.downloads.clear_finished();
                Value::Null
            }

            // ─── Normalisation du volume ────────────────────────────────────
            "loudness:progress" => to_value(self.loudness.progress()),
            "loudness:levels" => to_value(self.loudness.levels(&ids(a(0)))),

            // ─── Scrobbling ─────────────────────────────────────────────────
            "scrobble:status" => to_value(self.scrobbler.status()),
            "scrobble:connect-listenbrainz" => {
                let url = a(1).as_str().filter(|s| !s.trim().is_empty());
                wrap(self.scrobbler.connect_listenbrainz(&text(a(0)), url).map_err(|e| format!("{e:#}")))
            }
            "scrobble:lastfm-start" => wrap(self.scrobbler.lastfm_start().map_err(|e| format!("{e:#}"))),
            "scrobble:lastfm-finish" => wrap(self.scrobbler.lastfm_finish(&text(a(0))).map_err(|e| format!("{e:#}"))),
            "scrobble:set-lastfm-keys" => to_value(self.scrobbler.set_lastfm_keys(&text(a(0)), &text(a(1)))),
            "scrobble:disconnect" => to_value(self.scrobbler.disconnect(&text(a(0)))),

            // ─── Films, séries et animés ────────────────────────────────────
            other => match self.media.handle(other, args) {
                Some(v) if other != "app:env" => v,
                _ => return Err(RpcError::unknown(channel)),
            },
        })
    }

    /// `{ playlists, items }`.
    pub fn playlists_value(&self) -> Value {
        let (playlists, items) = self.library.playlists();
        json!({ "playlists": to_value(playlists), "items": to_value(items) })
    }

    /// Pochette d'une playlist : une image envoyée (`{ data }`), celle d'un morceau (`{ trackId }`), ou null (mosaïque).
    fn set_playlist_cover(&self, id: &Value, src: &Value) -> Value {
        let Some(id) = int(id) else { return json!({ "ok": false, "message": "Playlist invalide." }) };
        let lib = &self.library;
        if !truthy(src) {
            lib.set_playlist_cover(id, None);
            return json!({ "ok": true, "cover": null });
        }
        if let Some(track) = int(&src["trackId"]) {
            let Some(cover) = lib.track_cover(track) else {
                return json!({ "ok": false, "message": "Ce morceau n’a pas de pochette." });
            };
            lib.set_playlist_cover(id, Some(&cover));
            return json!({ "ok": true, "cover": cover });
        }
        let Some(data) = image_from_data_url(&src["data"]).filter(|d| covers::is_image(d) && decodable_image(d)) else {
            return json!({ "ok": false, "message": "Image refusée (JPEG, PNG ou WebP, 12 Mo au plus)." });
        };
        match lib.save_cover_image(&data) {
            Ok(cover) => {
                lib.set_playlist_cover(id, Some(&cover));
                json!({ "ok": true, "cover": cover })
            }
            Err(err) => json!({ "ok": false, "message": err.to_string() }),
        }
    }

    /// Dossier des pochettes.
    pub fn covers_dir(&self) -> &Path {
        self.library.covers().dir()
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests;

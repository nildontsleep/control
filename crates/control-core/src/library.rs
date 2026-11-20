//! Bibliothèque : scan incrémental des dossiers, lecture des tags, pochettes, playlists, écoutes.
//!
//! Toutes les méthodes sont bloquantes : l'interface les appelle depuis un thread de travail, jamais depuis le sien.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};

use parking_lot::{Condvar, Mutex};
use rusqlite::{OptionalExtension, params};

use crate::covers::Covers;
use crate::db::{Db, TRACK_COLUMNS};
use crate::model::{
    Emit, Event, Kind, Playlist, PlaylistId, PlaylistItem, PlaylistKind, ScanProgress, SmartSpec, Track, TrackId,
};
use crate::probe::{self, is_media_file};
use crate::util::{clip, now_ms, path_key};

const COVER_NAMES: &[&str] = &["cover", "folder", "front", "album", "albumart", "albumartsmall"];
const COVER_EXT: &[&str] = &["jpg", "jpeg", "png", "webp"];
const READ_CONCURRENCY: usize = 6;

/// Base partagée entre les modules du noyau.
pub type SharedDb = Arc<Mutex<Db>>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct ScanResult {
    pub added: usize,
    pub removed: usize,
    pub total: usize,
}

/// Résultat d'un import M3U.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct M3uImport {
    pub id: PlaylistId,
    pub matched: usize,
    pub total: usize,
}

#[derive(Default)]
struct ScanCtl {
    running: bool,
    pending: Vec<String>,
    holds: usize,
    deferred: Vec<String>,
}

/// Un fichier trouvé sur le disque.
#[derive(Debug, Clone)]
struct FileStat {
    path: String,
    size: i64,
    mtime: i64,
}

/// Ligne prête à écrire dans `tracks`.
#[derive(Debug, Clone)]
struct Row {
    path: String,
    kind: Kind,
    title: String,
    artist: String,
    album: String,
    album_artist: String,
    genre: String,
    year: Option<i64>,
    track_no: Option<i64>,
    disc_no: Option<i64>,
    duration: f64,
    bitrate: Option<i64>,
    codec: Option<String>,
    size: i64,
    mtime: i64,
    cover: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    source_url: Option<String>,
}

pub struct Library {
    db: SharedDb,
    covers: Covers,
    thumbs_dir: PathBuf,
    emit: Emit,
    ctl: Mutex<ScanCtl>,
    ctl_cv: Condvar,
    folder_covers: Mutex<HashMap<PathBuf, Option<String>>>,
    closed: AtomicBool,
    rev: AtomicU64,
    last_change: Mutex<Option<Instant>>,
}

fn stat(path: &Path) -> Option<FileStat> {
    let md = std::fs::metadata(path).ok()?;
    let mtime = md.modified().ok()?.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    Some(FileStat { path: path.to_string_lossy().into_owned(), size: md.len() as i64, mtime })
}

/// Parcourt un dossier (sans les dossiers et fichiers cachés `.x` ni système `$x`), en appelant `f` pour chaque média.
fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name.starts_with('$') {
            continue;
        }
        let Ok(ft) = entry.file_type() else { continue };
        let full = entry.path();
        if ft.is_dir() {
            walk(&full, f);
        } else if ft.is_file() && is_media_file(&full) {
            f(&full);
        }
    }
}

/// Préfixe de comparaison d'un dossier racine (chemin absolu suivi du séparateur).
fn root_prefix(root: &str) -> String {
    let abs = std::path::absolute(root).unwrap_or_else(|_| PathBuf::from(root));
    let mut s = abs.to_string_lossy().into_owned();
    if !s.ends_with(std::path::MAIN_SEPARATOR) {
        s.push(std::path::MAIN_SEPARATOR);
    }
    path_key(&s)
}

impl Library {
    pub fn new(db: SharedDb, covers: Covers, thumbs_dir: PathBuf, emit: Emit) -> Library {
        let _ = std::fs::create_dir_all(&thumbs_dir);
        Library {
            db,
            covers,
            thumbs_dir,
            emit,
            ctl: Mutex::new(ScanCtl::default()),
            ctl_cv: Condvar::new(),
            folder_covers: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            rev: AtomicU64::new(0),
            last_change: Mutex::new(None),
        }
    }

    pub fn db(&self) -> &SharedDb {
        &self.db
    }

    pub fn covers(&self) -> &Covers {
        &self.covers
    }

    /// Une analyse en cours s'arrête sans bruit.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// Augmente à chaque changement des morceaux : de quoi garder des index en cache.
    pub fn rev(&self) -> u64 {
        self.rev.load(Ordering::SeqCst)
    }

    /// Prévient l'interface : tout de suite, ou au plus une fois toutes les 400 ms pendant un scan.
    fn changed(&self, what: &'static str, now: bool) {
        if what == "tracks" {
            self.rev.fetch_add(1, Ordering::SeqCst);
        }
        if !now {
            let mut last = self.last_change.lock();
            if last.is_some_and(|t| t.elapsed() < Duration::from_millis(400)) {
                return;
            }
            *last = Some(Instant::now());
        }
        (self.emit)(Event::LibraryChanged(what));
    }

    fn progress(&self, p: ScanProgress) {
        (self.emit)(Event::ScanProgress(p));
    }

    // ─── Lecture ─────────────────────────────────────────────────────────────

    pub fn tracks(&self) -> Vec<Track> {
        let db = self.db.lock();
        let Ok(mut stmt) = db.prepare_cached(&format!("SELECT {TRACK_COLUMNS} FROM tracks")) else { return Vec::new() };
        stmt.query_map([], Track::from_row).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    pub fn track(&self, id: TrackId) -> Option<Track> {
        let db = self.db.lock();
        db.query_row(&format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE id = ?"), [id], Track::from_row)
            .optional()
            .ok()?
    }

    pub fn track_path(&self, id: TrackId) -> Option<String> {
        let db = self.db.lock();
        db.query_row("SELECT path FROM tracks WHERE id = ?", [id], |r| r.get(0)).optional().ok()?
    }

    pub fn track_cover(&self, id: TrackId) -> Option<String> {
        let db = self.db.lock();
        db.query_row("SELECT cover FROM tracks WHERE id = ?", [id], |r| r.get::<_, Option<String>>(0))
            .optional()
            .ok()
            .flatten()
            .flatten()
    }

    pub fn playlists(&self) -> (Vec<Playlist>, Vec<PlaylistItem>) {
        let db = self.db.lock();
        let playlists = db
            .prepare_cached(
                "SELECT id, name, kind, rules, description, cover, shared, created_at, updated_at
                 FROM playlists ORDER BY name COLLATE NOCASE",
            )
            .and_then(|mut stmt| {
                stmt.query_map([], |r| {
                    let rules: Option<String> = r.get(3)?;
                    Ok(Playlist {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        kind: PlaylistKind::parse(&r.get::<_, String>(2)?),
                        rules: rules.and_then(|s| serde_json::from_str::<SmartSpec>(&s).ok()),
                        description: r.get(4)?,
                        cover: r.get(5)?,
                        shared: r.get::<_, i64>(6)? != 0,
                        created_at: r.get(7)?,
                        updated_at: r.get(8)?,
                    })
                })
                .map(|rows| rows.flatten().collect::<Vec<_>>())
            })
            .unwrap_or_default();
        let items = db
            .prepare_cached(
                "SELECT id, playlist_id, track_id, position FROM playlist_tracks ORDER BY playlist_id, position",
            )
            .and_then(|mut stmt| {
                stmt.query_map([], |r| {
                    Ok(PlaylistItem { id: r.get(0)?, playlist_id: r.get(1)?, track_id: r.get(2)?, position: r.get(3)? })
                })
                .map(|rows| rows.flatten().collect::<Vec<_>>())
            })
            .unwrap_or_default();
        (playlists, items)
    }

    // ─── Scan ────────────────────────────────────────────────────────────────

    /// Exécute `f` pendant qu'aucun scan ne tourne (renommage de fichiers, écriture de tags). Un scan demandé
    /// pendant ce temps est fait juste après.
    pub fn exclusive<T>(&self, f: impl FnOnce() -> T) -> T {
        {
            let mut ctl = self.ctl.lock();
            while ctl.running {
                self.ctl_cv.wait(&mut ctl);
            }
            ctl.holds += 1;
        }
        let out = f();
        let deferred = {
            let mut ctl = self.ctl.lock();
            ctl.holds -= 1;
            if ctl.holds == 0 { std::mem::take(&mut ctl.deferred) } else { Vec::new() }
        };
        if !deferred.is_empty() {
            self.scan(&deferred);
        }
        out
    }

    /// Analyse les dossiers. Si un scan tourne déjà, ces dossiers seront analysés juste après (et le résultat est
    /// vide ici).
    pub fn scan(&self, roots: &[String]) -> ScanResult {
        let mut roots: Vec<String> = dedupe(roots.iter().filter(|r| !r.is_empty()).cloned());
        {
            let mut ctl = self.ctl.lock();
            if ctl.holds > 0 {
                ctl.deferred = dedupe(ctl.deferred.drain(..).chain(roots));
                return ScanResult::default();
            }
            if ctl.running {
                ctl.pending = dedupe(ctl.pending.drain(..).chain(roots));
                return ScanResult::default();
            }
            ctl.running = true;
        }
        let mut result;
        loop {
            result = self.scan_once(&roots);
            let mut ctl = self.ctl.lock();
            if !ctl.pending.is_empty() && !self.closed.load(Ordering::SeqCst) {
                roots = std::mem::take(&mut ctl.pending);
                continue;
            }
            ctl.running = false;
            self.ctl_cv.notify_all();
            break;
        }
        result
    }

    pub fn is_scanning(&self) -> bool {
        self.ctl.lock().running
    }

    fn scan_once(&self, roots: &[String]) -> ScanResult {
        self.progress(ScanProgress { phase: "discover", found: 0, done: 0, total: 0, added: 0, removed: 0 });
        let mut found: HashMap<String, FileStat> = HashMap::new();
        let mut per_root: HashMap<&str, usize> = HashMap::new();
        let mut reachable = Vec::new();
        for root in roots {
            if !Path::new(root).exists() {
                continue; // disque débranché : on ne touche à rien
            }
            reachable.push(root.as_str());
            let mut n = 0usize;
            walk(Path::new(root), &mut |file| {
                if let Some(st) = stat(file) {
                    found.insert(path_key(&st.path), st);
                    n += 1;
                    if found.len().is_multiple_of(250) {
                        self.progress(ScanProgress {
                            phase: "discover",
                            found: found.len(),
                            done: 0,
                            total: 0,
                            added: 0,
                            removed: 0,
                        });
                    }
                }
            });
            per_root.insert(root.as_str(), n);
            if self.closed.load(Ordering::SeqCst) {
                return ScanResult::default();
            }
        }

        let existing: HashMap<String, (i64, i64, i64)> = {
            let db = self.db.lock();
            let mut map = HashMap::new();
            if let Ok(mut stmt) = db.prepare("SELECT id, path, size, mtime FROM tracks")
                && let Ok(rows) = stmt.query_map([], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?))
                })
            {
                for (id, path, size, mtime) in rows.flatten() {
                    map.insert(path_key(&path), (id, size, mtime));
                }
            }
            map
        };

        let mut todo: Vec<FileStat> = found
            .iter()
            .filter(|(key, f)| match existing.get(*key) {
                Some((_, size, mtime)) => *size != f.size || *mtime != f.mtime,
                None => true,
            })
            .map(|(_, f)| f.clone())
            .collect();
        todo.sort_by(|a, b| a.path.cmp(&b.path));

        // Fichiers supprimés : seulement dans les dossiers lisibles et non vides.
        let mut remove_ids = Vec::new();
        for root in &reachable {
            let prefix = root_prefix(root);
            let rows: Vec<(&String, &(i64, i64, i64))> =
                existing.iter().filter(|(k, _)| k.starts_with(&prefix)).collect();
            if per_root.get(root).copied().unwrap_or(0) == 0 && !rows.is_empty() {
                continue;
            }
            for (key, (id, _, _)) in rows {
                if !found.contains_key(key) {
                    remove_ids.push(*id);
                }
            }
        }
        if !remove_ids.is_empty() {
            self.delete_rows(&remove_ids);
        }

        let done = AtomicUsize::new(0);
        let added = AtomicUsize::new(0);
        let index = AtomicUsize::new(0);
        let batch: Mutex<Vec<Row>> = Mutex::new(Vec::new());
        let flush = |force: bool| {
            let rows = {
                let mut b = batch.lock();
                if b.is_empty() || (!force && b.len() < 40) {
                    return;
                }
                std::mem::take(&mut *b)
            };
            self.upsert_rows(&rows);
            self.changed("tracks", false);
        };
        let total = todo.len();
        std::thread::scope(|scope| {
            for _ in 0..READ_CONCURRENCY.min(total) {
                scope.spawn(|| {
                    loop {
                        if self.closed.load(Ordering::SeqCst) {
                            return;
                        }
                        let i = index.fetch_add(1, Ordering::SeqCst);
                        let Some(file) = todo.get(i) else { return };
                        if let Some(row) = self.read(file, None) {
                            batch.lock().push(row);
                            added.fetch_add(1, Ordering::SeqCst);
                        }
                        let d = done.fetch_add(1, Ordering::SeqCst) + 1;
                        flush(false);
                        if d.is_multiple_of(20) {
                            self.progress(ScanProgress {
                                phase: "read",
                                done: d,
                                total,
                                found: found.len(),
                                added: 0,
                                removed: 0,
                            });
                        }
                    }
                });
            }
        });
        flush(true);
        self.folder_covers.lock().clear();

        let result = ScanResult { added: added.into_inner(), removed: remove_ids.len(), total: found.len() };
        self.progress(ScanProgress {
            phase: "done",
            found: result.total,
            done: result.added,
            total: result.total,
            added: result.added,
            removed: result.removed,
        });
        if result.added > 0 || result.removed > 0 {
            self.changed("tracks", true);
        }
        result
    }

    /// Ajoute des fichiers ou dossiers précis (glisser-déposer, téléchargements). Renvoie les identifiants.
    pub fn add_paths(&self, paths: &[String], source_url: Option<&str>) -> Vec<TrackId> {
        let mut files: Vec<FileStat> = Vec::new();
        let mut seen = HashSet::new();
        for p in paths {
            let path = Path::new(p);
            let Ok(md) = std::fs::metadata(path) else { continue };
            if md.is_dir() {
                walk(path, &mut |file| {
                    if let Some(st) = stat(file)
                        && seen.insert(path_key(&st.path))
                    {
                        files.push(st);
                    }
                });
            } else if md.is_file()
                && is_media_file(path)
                && let Some(st) = stat(path)
                && seen.insert(path_key(&st.path))
            {
                files.push(st);
            }
        }
        let rows: Vec<Row> = files.iter().filter_map(|f| self.read(f, source_url)).collect();
        let ids = self.upsert_rows(&rows);
        self.folder_covers.lock().clear();
        if !ids.is_empty() {
            self.changed("tracks", true);
        }
        ids
    }

    fn read(&self, file: &FileStat, source_url: Option<&str>) -> Option<Row> {
        let path = Path::new(&file.path);
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let p = probe::probe(path);
        let mut cover = p.picture.as_ref().and_then(|data| self.covers.save(data).ok());
        if cover.is_none() && p.kind == Kind::Audio {
            cover = path.parent().and_then(|dir| self.folder_cover(dir));
        }
        Some(Row {
            path: file.path.clone(),
            kind: p.kind,
            title: p.title.unwrap_or(stem),
            artist: p.artist,
            album: p.album,
            album_artist: p.album_artist,
            genre: p.genre,
            year: p.year,
            track_no: p.track_no,
            disc_no: p.disc_no,
            duration: p.duration,
            bitrate: p.bitrate,
            codec: p.codec,
            size: file.size,
            mtime: file.mtime,
            cover,
            width: p.width,
            height: p.height,
            source_url: source_url.map(str::to_string),
        })
    }

    fn folder_cover(&self, dir: &Path) -> Option<String> {
        if let Some(hit) = self.folder_covers.lock().get(dir) {
            return hit.clone();
        }
        let mut hash = None;
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut names: Vec<String> =
                entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            // Ordre de préférence : celui des noms, puis des extensions.
            'search: for n in COVER_NAMES {
                for e in COVER_EXT {
                    let wanted = format!("{n}.{e}");
                    if let Some(hit) = names.iter().find(|name| name.to_lowercase() == wanted)
                        && let Ok(data) = std::fs::read(dir.join(hit))
                    {
                        hash = self.covers.save(&data).ok();
                        break 'search;
                    }
                }
            }
        }
        self.folder_covers.lock().insert(dir.to_path_buf(), hash.clone());
        hash
    }

    fn upsert_rows(&self, rows: &[Row]) -> Vec<TrackId> {
        if rows.is_empty() {
            return Vec::new();
        }
        let mut db = self.db.lock();
        let now = now_ms();
        let res = crate::db::transaction(&mut db, |tx| {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO tracks (path, kind, title, artist, album, album_artist, genre, year, track_no, disc_no,
                   duration, bitrate, codec, size, mtime, cover, width, height, added_at, source_url)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
                 ON CONFLICT(path) DO UPDATE SET
                   kind = excluded.kind, title = excluded.title, artist = excluded.artist, album = excluded.album,
                   album_artist = excluded.album_artist, genre = excluded.genre, year = excluded.year,
                   track_no = excluded.track_no, disc_no = excluded.disc_no, duration = excluded.duration,
                   bitrate = excluded.bitrate, codec = excluded.codec, size = excluded.size, mtime = excluded.mtime,
                   cover = excluded.cover, width = excluded.width, height = excluded.height,
                   source_url = COALESCE(excluded.source_url, tracks.source_url)
                 RETURNING id",
            )?;
            let mut ids = Vec::with_capacity(rows.len());
            for r in rows {
                let id: i64 = stmt.query_row(
                    params![
                        r.path,
                        r.kind.as_str(),
                        r.title,
                        r.artist,
                        r.album,
                        r.album_artist,
                        r.genre,
                        r.year,
                        r.track_no,
                        r.disc_no,
                        r.duration,
                        r.bitrate,
                        r.codec,
                        r.size,
                        r.mtime,
                        r.cover,
                        r.width,
                        r.height,
                        now,
                        r.source_url
                    ],
                    |row| row.get(0),
                )?;
                ids.push(id);
            }
            Ok(ids)
        });
        res.unwrap_or_else(|err| {
            log::error!("Écriture de la bibliothèque impossible : {err}");
            Vec::new()
        })
    }

    fn delete_rows(&self, ids: &[TrackId]) {
        let mut db = self.db.lock();
        let res = crate::db::transaction(&mut db, |tx| {
            let mut stmt = tx.prepare_cached("DELETE FROM tracks WHERE id = ?")?;
            for id in ids {
                stmt.execute([id])?;
            }
            Ok(())
        });
        if let Err(err) = res {
            log::error!("Suppression impossible : {err}");
        }
    }

    /// Relit un fichier modifié (tags, nom) en gardant le même identifiant : lectures et playlists suivent.
    pub fn refresh_track(&self, id: TrackId, new_path: Option<&str>) -> Option<Track> {
        let current = self.track_path(id)?;
        let file = new_path.unwrap_or(&current).to_string();
        let st = stat(Path::new(&file))?;
        let row = self.read(&st, None)?;
        if file != current {
            let db = self.db.lock();
            if let Err(err) = db.execute("UPDATE tracks SET path = ? WHERE id = ?", params![file, id]) {
                log::error!("Renommage dans la base impossible : {err}");
                return None;
            }
        }
        self.upsert_rows(std::slice::from_ref(&row));
        self.folder_covers.lock().clear();
        // Le morceau a changé : les index gardés en cache doivent le savoir (l'appelant prévient l'interface).
        self.rev.fetch_add(1, Ordering::SeqCst);
        self.track(id)
    }

    /// Retire des morceaux de la bibliothèque, et met leurs fichiers à la corbeille si demandé.
    pub fn remove_tracks(&self, ids: &[TrackId], delete_files: bool) {
        if delete_files {
            for id in ids {
                if let Some(file) = self.track_path(*id)
                    && let Err(err) = trash::delete(&file)
                {
                    log::warn!("Corbeille impossible pour {file} : {err}");
                }
            }
        }
        self.delete_rows(ids);
        self.changed("tracks", true);
        self.changed("playlists", true);
    }

    /// Retire de la bibliothèque les morceaux d'un dossier (les fichiers restent intacts).
    pub fn remove_folder(&self, folder: &str) {
        let prefix = root_prefix(folder);
        let ids: Vec<TrackId> =
            self.tracks().into_iter().filter(|t| path_key(&t.path).starts_with(&prefix)).map(|t| t.id).collect();
        if !ids.is_empty() {
            self.remove_tracks(&ids, false);
        }
    }

    // ─── Pochettes ───────────────────────────────────────────────────────────

    /// Enregistre une image comme pochette ; renvoie son empreinte.
    pub fn save_cover_image(&self, data: &[u8]) -> std::io::Result<String> {
        self.covers.save(data)
    }

    /// Vignette d'une vidéo, extraite par ffmpeg (et gardée).
    pub fn video_thumb(&self, id: TrackId, ffmpeg: Option<&Path>) -> Option<PathBuf> {
        let t = self.track(id)?;
        if t.kind != Kind::Video {
            return None;
        }
        let out = self.thumbs_dir.join(format!("{id}-{}.jpg", t.mtime));
        if out.exists() {
            return Some(out);
        }
        let ffmpeg = ffmpeg?;
        let at = if t.duration > 6.0 { "3" } else { "0" };
        let mut cmd = std::process::Command::new(ffmpeg);
        cmd.args(["-y", "-v", "error", "-ss", at, "-i"])
            .arg(&t.path)
            .args(["-frames:v", "1", "-vf", "scale=480:-2", "-q:v", "4"])
            .arg(&out)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        crate::process::hide_window(&mut cmd);
        let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
        (ok && out.exists()).then_some(out)
    }

    // ─── Playlists ───────────────────────────────────────────────────────────

    fn exec(&self, sql: &str, p: impl rusqlite::Params) -> bool {
        match self.db.lock().execute(sql, p) {
            Ok(n) => n > 0,
            Err(err) => {
                log::error!("Base : {err}");
                false
            }
        }
    }

    pub fn create_playlist(&self, name: &str, kind: PlaylistKind, rules: Option<&SmartSpec>) -> Option<PlaylistId> {
        let now = now_ms();
        let name = name.trim();
        let name = if name.is_empty() { "Sans titre" } else { name };
        let rules = rules.and_then(|r| serde_json::to_string(r).ok());
        let id = self
            .db
            .lock()
            .query_row(
                "INSERT INTO playlists (name, kind, rules, created_at, updated_at) VALUES (?, ?, ?, ?, ?) RETURNING id",
                params![clip(name, 120), kind.as_str(), rules, now, now],
                |r| r.get(0),
            )
            .ok();
        self.changed("playlists", true);
        id
    }

    pub fn rename_playlist(&self, id: PlaylistId, name: &str) {
        let name = name.trim();
        let name = if name.is_empty() { "Sans titre" } else { name };
        self.exec("UPDATE playlists SET name = ?, updated_at = ? WHERE id = ?", params![clip(name, 120), now_ms(), id]);
        self.changed("playlists", true);
    }

    /// Nom, description et visibilité (seuls les champs fournis changent).
    pub fn update_playlist(
        &self,
        id: PlaylistId,
        name: Option<&str>,
        description: Option<&str>,
        shared: Option<bool>,
    ) -> bool {
        let row: Option<(String, String, i64)> = self
            .db
            .lock()
            .query_row("SELECT name, description, shared FROM playlists WHERE id = ?", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()
            .ok()
            .flatten();
        let Some((old_name, old_desc, old_shared)) = row else { return false };
        let name = match name {
            Some(n) => {
                let n = clip(n.trim(), 120);
                if n.is_empty() { "Sans titre".to_string() } else { n }
            }
            None => old_name,
        };
        let description = description.map(|d| clip(d.trim(), 500)).unwrap_or(old_desc);
        let shared = shared.map(i64::from).unwrap_or(old_shared);
        self.exec(
            "UPDATE playlists SET name = ?, description = ?, shared = ?, updated_at = ? WHERE id = ?",
            params![name, description, shared, now_ms(), id],
        );
        self.changed("playlists", true);
        true
    }

    /// Pochette d'une playlist : une empreinte, ou `None` pour revenir à la mosaïque des morceaux.
    pub fn set_playlist_cover(&self, id: PlaylistId, hash: Option<&str>) {
        self.exec("UPDATE playlists SET cover = ?, updated_at = ? WHERE id = ?", params![hash, now_ms(), id]);
        self.changed("playlists", true);
    }

    pub fn set_playlist_rules(&self, id: PlaylistId, rules: &SmartSpec) {
        let json = serde_json::to_string(rules).unwrap_or_else(|_| "null".into());
        self.exec("UPDATE playlists SET rules = ?, updated_at = ? WHERE id = ?", params![json, now_ms(), id]);
        self.changed("playlists", true);
    }

    pub fn delete_playlist(&self, id: PlaylistId) {
        self.exec("DELETE FROM playlists WHERE id = ?", [id]);
        self.changed("playlists", true);
    }

    pub fn add_to_playlist(&self, playlist: PlaylistId, tracks: &[TrackId]) {
        let now = now_ms();
        let mut db = self.db.lock();
        let res = crate::db::transaction(&mut db, |tx| {
            let next: i64 = tx.query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_tracks WHERE playlist_id = ?",
                [playlist],
                |r| r.get(0),
            )?;
            let mut stmt = tx.prepare_cached(
                "INSERT INTO playlist_tracks (playlist_id, track_id, position, added_at) VALUES (?, ?, ?, ?)",
            )?;
            for (offset, t) in tracks.iter().enumerate() {
                stmt.execute(params![playlist, t, next + offset as i64, now])?;
            }
            tx.execute("UPDATE playlists SET updated_at = ? WHERE id = ?", params![now, playlist])?;
            Ok(())
        });
        drop(db);
        if let Err(err) = res {
            log::error!("Ajout à la playlist impossible : {err}");
        }
        self.changed("playlists", true);
    }

    pub fn remove_from_playlist(&self, entries: &[i64]) {
        let mut db = self.db.lock();
        let res = crate::db::transaction(&mut db, |tx| {
            let mut stmt = tx.prepare_cached("DELETE FROM playlist_tracks WHERE id = ?")?;
            for e in entries {
                stmt.execute([e])?;
            }
            Ok(())
        });
        drop(db);
        if let Err(err) = res {
            log::error!("Retrait de la playlist impossible : {err}");
        }
        self.changed("playlists", true);
    }

    pub fn reorder_playlist(&self, playlist: PlaylistId, entries: &[i64]) {
        let mut db = self.db.lock();
        let res = crate::db::transaction(&mut db, |tx| {
            let mut stmt =
                tx.prepare_cached("UPDATE playlist_tracks SET position = ? WHERE id = ? AND playlist_id = ?")?;
            for (pos, e) in entries.iter().enumerate() {
                stmt.execute(params![pos as i64, e, playlist])?;
            }
            tx.execute("UPDATE playlists SET updated_at = ? WHERE id = ?", params![now_ms(), playlist])?;
            Ok(())
        });
        drop(db);
        if let Err(err) = res {
            log::error!("Réordonnancement impossible : {err}");
        }
        self.changed("playlists", true);
    }

    /// Importe une playlist M3U / M3U8 (les chemins relatifs le sont au dossier du fichier).
    pub fn import_m3u(&self, file: &Path) -> std::io::Result<M3uImport> {
        let bytes = std::fs::read(file)?;
        let text = String::from_utf8_lossy(&bytes);
        let text = text.trim_start_matches('\u{feff}');
        let dir = file.parent().unwrap_or(Path::new(""));
        let by_path: HashMap<String, TrackId> = {
            let db = self.db.lock();
            let mut map = HashMap::new();
            if let Ok(mut stmt) = db.prepare("SELECT id, path FROM tracks")
                && let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            {
                for (id, p) in rows.flatten() {
                    map.insert(path_key(&normalize(Path::new(&p))), id);
                }
            }
            map
        };
        let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        let mut ids = Vec::new();
        for line in &lines {
            let p = Path::new(line);
            let full = if p.is_absolute() { p.to_path_buf() } else { dir.join(p) };
            if let Some(id) = by_path.get(&path_key(&normalize(&full))) {
                ids.push(*id);
            }
        }
        let name = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let id = self
            .create_playlist(&name, PlaylistKind::Manual, None)
            .ok_or_else(|| std::io::Error::other("création de la playlist impossible"))?;
        if !ids.is_empty() {
            self.add_to_playlist(id, &ids);
        }
        Ok(M3uImport { id, matched: ids.len(), total: lines.len() })
    }

    /// Exporte des morceaux en M3U étendu (UTF-8, fins de ligne CRLF). Renvoie le nombre de morceaux écrits.
    pub fn export_m3u(&self, file: &Path, tracks: &[TrackId]) -> std::io::Result<usize> {
        let mut lines = vec!["#EXTM3U".to_string()];
        let mut n = 0;
        for id in tracks {
            let Some(t) = self.track(*id) else { continue };
            let who = if t.artist.is_empty() { String::new() } else { format!("{} - ", t.artist) };
            lines.push(format!("#EXTINF:{},{who}{}", t.duration.round() as i64, t.title));
            lines.push(t.path);
            n += 1;
        }
        crate::util::write_atomic(file, format!("{}\r\n", lines.join("\r\n")).as_bytes())?;
        Ok(n)
    }

    // ─── Écoutes ─────────────────────────────────────────────────────────────

    /// Une lecture commence : enregistrée, et comptée pour le morceau. Renvoie l'identifiant de l'écoute.
    pub fn play_start(&self, track: TrackId, duration: f64) -> Option<i64> {
        let now = now_ms();
        let db = self.db.lock();
        let id = db
            .query_row(
                "INSERT INTO plays (track_id, played_at, duration) VALUES (?, ?, ?) RETURNING id",
                params![track, now, if duration.is_finite() { duration } else { 0.0 }],
                |r| r.get(0),
            )
            .ok()?;
        let _ = db.execute(
            "UPDATE tracks SET play_count = play_count + 1, last_played = ? WHERE id = ?",
            params![now, track],
        );
        Some(id)
    }

    pub fn play_end(&self, play: i64, seconds: f64, completed: bool) {
        let seconds = if seconds.is_finite() { seconds.max(0.0) } else { 0.0 };
        self.exec("UPDATE plays SET seconds = ?, completed = ? WHERE id = ?", params![seconds, completed as i64, play]);
    }
}

fn dedupe(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items.into_iter().filter(|s| seen.insert(s.clone())).collect()
}

/// Normalise un chemin sans toucher au disque (`a/./b/../c` → `a/c`).
pub fn normalize(p: &Path) -> String {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests;

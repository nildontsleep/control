//! Paroles : fichier `.lrc` (ou `.txt`) voisin, paroles intégrées au fichier, puis LRCLIB.
//!
//! Le résultat est gardé dans la table `lyrics` : un morceau n'est cherché en ligne qu'une fois. Une recherche
//! sans résultat est gardée aussi (`source = 'none'`), puis retentée au bout de trois jours.
//!
//! Méthodes bloquantes, à appeler depuis un thread de travail. La base n'est verrouillée que le temps d'une
//! requête, jamais pendant la lecture des fichiers ni les appels à LRCLIB.

mod embedded;
mod lrclib;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{OptionalExtension, params};
use serde::Serialize;

pub use embedded::{embedded_lyrics, sylt_to_lrc};
pub use lrclib::{
    DEFAULT_API, Http, Query, Record, USER_AGENT, UreqHttp, exact_url, parse_get, parse_search, pick, search_url,
};

use crate::library::SharedDb;
use crate::lrc::{Lrc, looks_synced};
use crate::model::TrackId;
use crate::util::{now_ms, write_atomic};

/// Délai avant de retenter un morceau resté sans paroles.
pub const RETRY_NONE_AFTER_MS: i64 = 3 * 24 * 3600 * 1000;
/// Taille maximale de paroles saisies à la main (caractères).
pub const MAX_MANUAL_CHARS: usize = 200_000;
const RETRY_DELAY: Duration = Duration::from_millis(900);

/// D'où viennent les paroles (colonne `source`, mêmes valeurs que l'original).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Fichier voisin du morceau.
    File,
    /// Étiquettes du fichier.
    Tags,
    Lrclib,
    /// Saisies ou collées à la main.
    Manual,
    /// Rien trouvé (cache négatif).
    None,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::File => "file",
            Source::Tags => "tags",
            Source::Lrclib => "lrclib",
            Source::Manual => "manual",
            Source::None => "none",
        }
    }

    /// Une valeur inconnue est traitée comme une saisie : jamais écrasée par une recherche.
    pub fn parse(s: &str) -> Source {
        match s {
            "file" => Source::File,
            "tags" => Source::Tags,
            "lrclib" => Source::Lrclib,
            "none" => Source::None,
            _ => Source::Manual,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    None,
}

/// Réponse de `lyrics:get` / `lyrics:set` (même JSON que l'original).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LyricsResult {
    pub status: Status,
    pub source: Source,
    pub synced: Option<String>,
    pub plain: Option<String>,
    pub instrumental: bool,
}

impl LyricsResult {
    fn none() -> LyricsResult {
        LyricsResult { status: Status::None, source: Source::None, synced: None, plain: None, instrumental: false }
    }

    /// Texte à afficher : la version minutée de préférence.
    pub fn text(&self) -> Option<&str> {
        self.synced.as_deref().or(self.plain.as_deref())
    }

    /// Paroles analysées, prêtes à afficher.
    pub fn parsed(&self) -> Lrc {
        self.text().map(crate::lrc::parse).unwrap_or_default()
    }
}

/// Paroles reconnues dans un texte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Text {
    Synced(String),
    Plain(String),
}

impl Text {
    fn split(self) -> (Option<String>, Option<String>) {
        match self {
            Text::Synced(s) => (Some(s), None),
            Text::Plain(s) => (None, Some(s)),
        }
    }
}

/// Classe un texte : LRC s'il a au moins trois repères de temps, texte brut sinon, rien s'il est vide.
pub fn classify(text: &str) -> Option<Text> {
    let value = text.strip_prefix('\u{feff}').unwrap_or(text).trim();
    if value.is_empty() {
        None
    } else if looks_synced(value) {
        Some(Text::Synced(value.to_string()))
    } else {
        Some(Text::Plain(value.to_string()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LyricsError {
    #[error("Morceau introuvable.")]
    TrackNotFound,
    #[error("Aucune parole à enregistrer.")]
    NothingToSave,
    /// LRCLIB occupé (429, 5xx) après trois essais.
    #[error("LRCLIB {0}")]
    Busy(u16),
    #[error("LRCLIB {0}")]
    Http(u16),
    #[error("LRCLIB injoignable : {0}")]
    Network(String),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl LyricsError {
    /// Code de l'original, pour que l'interface choisisse son message (`busy`, `network`, `http`).
    pub fn code(&self) -> Option<&'static str> {
        match self {
            LyricsError::Busy(_) => Some("busy"),
            LyricsError::Network(_) => Some("network"),
            LyricsError::Http(_) => Some("http"),
            _ => None,
        }
    }
}

/// Ligne de la table `lyrics`.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    synced: Option<String>,
    plain: Option<String>,
    source: Source,
    instrumental: bool,
    fetched_at: i64,
}

impl Row {
    fn new(synced: Option<String>, plain: Option<String>, source: Source, instrumental: bool) -> Row {
        let keep = |s: Option<String>| s.filter(|s| !s.is_empty());
        Row { synced: keep(synced), plain: keep(plain), source, instrumental, fetched_at: now_ms() }
    }

    fn from_text(text: Text, source: Source) -> Row {
        let (synced, plain) = text.split();
        Row::new(synced, plain, source, false)
    }

    fn result(&self) -> LyricsResult {
        let found = self.synced.is_some() || self.plain.is_some() || self.instrumental;
        LyricsResult {
            status: if found { Status::Ok } else { Status::None },
            source: self.source,
            synced: self.synced.clone(),
            plain: self.plain.clone(),
            instrumental: self.instrumental,
        }
    }
}

/// Chemin du morceau sans son extension (`/a/b.flac` → `/a/b`).
fn base_of(file: &Path) -> PathBuf {
    file.with_extension("")
}

fn sibling(file: &Path, ext: &str) -> PathBuf {
    let mut s = base_of(file).into_os_string();
    s.push(ext);
    PathBuf::from(s)
}

/// Fichier voisin : `.lrc` d'abord, puis `.txt`.
fn sidecar(file: &Path) -> Option<Text> {
    [".lrc", ".txt"].iter().find_map(|ext| {
        let bytes = std::fs::read(sibling(file, ext)).ok()?;
        classify(&String::from_utf8_lossy(&bytes))
    })
}

pub struct Lyrics {
    db: SharedDb,
    http: Arc<dyn Http>,
    api: String,
    retry_delay: Duration,
}

impl Lyrics {
    /// LRCLIB à l'adresse habituelle, ou à celle de `CONTROL_LRCLIB_URL` (comme l'original).
    pub fn new(db: SharedDb) -> Lyrics {
        let api = std::env::var("CONTROL_LRCLIB_URL").ok().filter(|s| !s.is_empty());
        Lyrics::with_http(db, Arc::new(UreqHttp::default()), api.as_deref().unwrap_or(DEFAULT_API))
    }

    /// Avec un autre transport (tests) ou une autre adresse.
    pub fn with_http(db: SharedDb, http: Arc<dyn Http>, api: &str) -> Lyrics {
        Lyrics { db, http, api: api.trim_end_matches('/').to_string(), retry_delay: RETRY_DELAY }
    }

    /// Attente entre deux essais quand LRCLIB est occupé (multipliée par le numéro de l'essai).
    pub fn set_retry_delay(&mut self, d: Duration) {
        self.retry_delay = d;
    }

    fn row(&self, id: TrackId) -> rusqlite::Result<Option<Row>> {
        let db = self.db.lock();
        db.query_row(
            "SELECT synced, plain, source, instrumental, fetched_at FROM lyrics WHERE track_id = ?",
            [id],
            |r| {
                Ok(Row {
                    synced: r.get::<_, Option<String>>(0)?.filter(|s| !s.is_empty()),
                    plain: r.get::<_, Option<String>>(1)?.filter(|s| !s.is_empty()),
                    source: Source::parse(&r.get::<_, String>(2)?),
                    instrumental: r.get::<_, i64>(3)? != 0,
                    fetched_at: r.get(4)?,
                })
            },
        )
        .optional()
    }

    fn store(&self, id: TrackId, row: &Row) -> rusqlite::Result<()> {
        let db = self.db.lock();
        db.execute(
            "INSERT INTO lyrics (track_id, synced, plain, source, instrumental, fetched_at) VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(track_id) DO UPDATE SET synced = excluded.synced, plain = excluded.plain,
               source = excluded.source, instrumental = excluded.instrumental, fetched_at = excluded.fetched_at",
            params![id, row.synced, row.plain, row.source.as_str(), row.instrumental as i64, row.fetched_at],
        )?;
        Ok(())
    }

    fn track(&self, id: TrackId) -> Result<(PathBuf, Query), LyricsError> {
        let db = self.db.lock();
        db.query_row("SELECT path, title, artist, album, duration FROM tracks WHERE id = ?", [id], |r| {
            Ok((
                PathBuf::from(r.get::<_, String>(0)?),
                Query { title: r.get(1)?, artist: r.get(2)?, album: r.get(3)?, duration: r.get(4)? },
            ))
        })
        .optional()?
        .ok_or(LyricsError::TrackNotFound)
    }

    /// Paroles d'un morceau. Ordre : fichier voisin (toujours, c'est le choix explicite de l'utilisateur), cache,
    /// étiquettes du fichier, LRCLIB. `refresh` ignore le cache et les étiquettes et interroge LRCLIB.
    pub fn get(&self, id: TrackId, refresh: bool) -> Result<LyricsResult, LyricsError> {
        let (path, query) = self.track(id)?;

        if let Some(text) = sidecar(&path) {
            let row = Row::from_text(text, Source::File);
            self.store(id, &row)?;
            return Ok(row.result());
        }

        // Un cache « fichier » dont le fichier a disparu ne compte plus.
        if !refresh
            && let Some(cached) = self.row(id)?
            && cached.source != Source::File
            && (cached.source != Source::None || now_ms() - cached.fetched_at < RETRY_NONE_AFTER_MS)
        {
            return Ok(cached.result());
        }

        if !refresh && let Some(text) = embedded_lyrics(&path).as_deref().and_then(classify) {
            let row = Row::from_text(text, Source::Tags);
            self.store(id, &row)?;
            return Ok(row.result());
        }

        let row = match lrclib::lookup(self.http.as_ref(), &self.api, &query, self.retry_delay)? {
            Some(hit) => Row::new(hit.synced, hit.plain, Source::Lrclib, hit.instrumental),
            None => Row::new(None, None, Source::None, false),
        };
        self.store(id, &row)?;
        Ok(row.result())
    }

    /// Paroles saisies ou collées à la main ; un texte vide efface les paroles gardées.
    pub fn set(&self, id: TrackId, text: &str) -> Result<LyricsResult, LyricsError> {
        let text: String = text.chars().take(MAX_MANUAL_CHARS).collect();
        let Some(text) = classify(&text) else {
            self.db.lock().execute("DELETE FROM lyrics WHERE track_id = ?", [id])?;
            return Ok(LyricsResult::none());
        };
        let row = Row::from_text(text, Source::Manual);
        self.store(id, &row)?;
        Ok(row.result())
    }

    /// Écrit les paroles gardées à côté du morceau (`.lrc` si minutées, `.txt` sinon), pour que d'autres lecteurs
    /// les trouvent. Le fichier devient la source de référence. Renvoie le chemin écrit.
    pub fn save_sidecar(&self, id: TrackId) -> Result<PathBuf, LyricsError> {
        let row = self.row(id)?.ok_or(LyricsError::NothingToSave)?;
        let path = match self.track(id) {
            Ok((p, _)) => p,
            Err(LyricsError::TrackNotFound) => return Err(LyricsError::NothingToSave),
            Err(e) => return Err(e),
        };
        let (ext, text) = match (&row.synced, &row.plain) {
            (Some(s), _) => (".lrc", s),
            (None, Some(p)) => (".txt", p),
            (None, None) => return Err(LyricsError::NothingToSave),
        };
        let target = sibling(&path, ext);
        write_atomic(&target, text.as_bytes())?;
        self.store(id, &Row::new(row.synced, row.plain, Source::File, false))?;
        Ok(target)
    }
}

#[cfg(test)]
mod tests;

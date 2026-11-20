//! Suivi du visionnage : progression par épisode (`watch_progress`), « Ma liste » (`watchlist`), historique et
//! temps réellement regardé par jour (`watch_time`).

use parking_lot::Mutex;
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::error::MediaError;
use crate::library::SharedDb;
use crate::model::{Emit, Event};
use crate::util::{local_day, now_ms};

/// Au-delà de cette part de la durée, l'épisode compte comme vu.
pub const FINISHED_RATIO: f64 = 0.92;
/// Un message de progression arrive toutes les 5 s : au-delà de 20 s d'avance, c'est un saut dans la vidéo.
const MAX_STEP: f64 = 20.0;
/// Deux messages plus espacés ne se suivent pas (pause, autre appareil).
const MAX_GAP_MS: i64 = 60_000;
/// Positions récentes gardées en mémoire.
const LAST_KEEP: usize = 50;

pub const TYPES: [&str; 3] = ["movie", "tv", "anime"];

const PROGRESS_COLUMNS: &str = "media_type, media_id, season, episode, position, duration, finished, title, poster, \
     backdrop, year, genres, first_at, updated_at";

/// Une position envoyée par le lecteur.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WatchRecord {
    pub kind: String,
    pub id: String,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    /// Secondes.
    pub position: f64,
    pub duration: f64,
    pub finished: bool,
    pub title: String,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub year: Option<i64>,
    /// Genres (huit au plus sont gardés).
    pub genres: Vec<Value>,
}

/// Un titre à ajouter à « Ma liste ».
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ListEntry {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub year: Option<i64>,
    pub rating: Option<f64>,
}

// ─── Lecture tolérante de l'objet JSON envoyé par l'interface (mêmes règles que l'original) ────────────────────

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// `Number.isInteger`.
fn integer(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().filter(|f| f.is_finite() && f.fract() == 0.0).map(|f| f as i64))
}

/// `Number(v) || 0`.
fn number(v: &Value) -> f64 {
    let n = match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s.trim().parse().unwrap_or(0.0),
        Value::Bool(true) => 1.0,
        _ => 0.0,
    };
    if n.is_finite() { n } else { 0.0 }
}

/// Identifiant non vide (`String(id)`).
fn media_id(v: &Value) -> Option<String> {
    if !truthy(v) {
        return None;
    }
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn media_type(v: &Value) -> Option<String> {
    v.as_str().filter(|t| TYPES.contains(t)).map(str::to_string)
}

fn opt_text(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

fn plain_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

impl WatchRecord {
    /// `None` si le type ou l'identifiant manque.
    pub fn from_json(v: &Value) -> Option<WatchRecord> {
        Some(WatchRecord {
            kind: media_type(&v["type"])?,
            id: media_id(&v["id"])?,
            season: integer(&v["season"]),
            episode: integer(&v["episode"]),
            position: number(&v["position"]),
            duration: number(&v["duration"]),
            finished: truthy(&v["finished"]),
            title: plain_text(&v["title"]),
            poster: opt_text(&v["poster"]),
            backdrop: opt_text(&v["backdrop"]),
            year: integer(&v["year"]),
            genres: v["genres"].as_array().cloned().unwrap_or_default(),
        })
    }
}

impl ListEntry {
    pub fn from_json(v: &Value) -> Option<ListEntry> {
        Some(ListEntry {
            kind: media_type(&v["type"])?,
            id: media_id(&v["id"])?,
            title: plain_text(&v["title"]),
            poster: opt_text(&v["poster"]),
            backdrop: opt_text(&v["backdrop"]),
            year: integer(&v["year"]),
            rating: v["rating"].as_f64().filter(|f| f.is_finite()),
        })
    }
}

// ─── Lignes renvoyées ───────────────────────────────────────────────────────────────────────────────────────────

/// Progression d'un épisode (ou d'un film).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub position: f64,
    pub duration: f64,
    pub finished: bool,
    pub title: String,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub year: Option<i64>,
    pub genres: Value,
    pub first_at: i64,
    pub updated_at: i64,
}

impl Progress {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<Progress> {
        let season: i64 = r.get(2)?;
        let episode: i64 = r.get(3)?;
        let genres: String = r.get(11)?;
        Ok(Progress {
            kind: r.get(0)?,
            id: r.get(1)?,
            season: (season >= 0).then_some(season),
            episode: (episode >= 0).then_some(episode),
            position: r.get(4)?,
            duration: r.get(5)?,
            finished: r.get::<_, i64>(6)? != 0,
            title: r.get(7)?,
            poster: r.get(8)?,
            backdrop: r.get(9)?,
            year: r.get(10)?,
            genres: serde_json::from_str(&genres).unwrap_or_else(|_| json!([])),
            first_at: r.get(12)?,
            updated_at: r.get(13)?,
        })
    }
}

/// Une entrée de « Ma liste ».
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchlistItem {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub title: String,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub year: Option<i64>,
    pub rating: Option<f64>,
    pub added_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Saved {
    pub finished: bool,
}

#[derive(Debug, Clone, Copy)]
struct Last {
    position: f64,
    at: i64,
}

pub struct Watch {
    db: SharedDb,
    emit: Emit,
    /// Dernière position reçue par épisode, dans l'ordre d'arrivée de chaque clé (pour le temps regardé).
    last: Mutex<Vec<(String, Last)>>,
}

impl Watch {
    pub fn new(db: SharedDb, emit: Emit) -> Watch {
        Watch { db, emit, last: Mutex::new(Vec::new()) }
    }

    fn changed(&self, what: &str) {
        (self.emit)(Event::Json("watch:changed", json!({ "what": what })));
    }

    // ─── Progression ────────────────────────────────────────────────────────────────────────────────────────────

    pub fn save(&self, rec: &WatchRecord) -> Result<Saved, MediaError> {
        self.save_at(rec, now_ms())
    }

    /// `null` (aucune ligne) quand l'objet n'a pas de type ou d'identifiant valide, comme l'original.
    pub fn save_json(&self, rec: &Value) -> Result<Option<Saved>, MediaError> {
        match WatchRecord::from_json(rec) {
            Some(rec) => self.save(&rec).map(Some),
            None => Ok(None),
        }
    }

    fn save_at(&self, rec: &WatchRecord, now: i64) -> Result<Saved, MediaError> {
        let season = rec.season.unwrap_or(-1);
        let episode = rec.episode.unwrap_or(-1);
        let position = rec.position.max(0.0);
        let duration = rec.duration.max(0.0);
        let finished = rec.finished || (duration > 0.0 && position / duration >= FINISHED_RATIO);
        let genres: Vec<&Value> = rec.genres.iter().take(8).collect();
        let genres = serde_json::to_string(&genres).unwrap_or_else(|_| "[]".into());

        // Temps réellement regardé : seulement de petites avancées continues.
        let key = format!("{}:{}:{season}:{episode}", rec.kind, rec.id);
        let delta = {
            let mut last = self.last.lock();
            let now_last = Last { position, at: now };
            let before = match last.iter_mut().find(|(k, _)| *k == key) {
                Some((_, slot)) => Some(std::mem::replace(slot, now_last)),
                None => {
                    last.push((key, now_last));
                    None
                }
            };
            if last.len() > LAST_KEEP {
                last.remove(0);
            }
            before
                .filter(|b| now - b.at < MAX_GAP_MS)
                .map(|b| position - b.position)
                .filter(|d| *d > 0.0 && *d <= MAX_STEP)
        };

        let db = self.db.lock();
        db.execute(
            "INSERT INTO watch_progress (media_type, media_id, season, episode, position, duration, finished, title, \
             poster, backdrop, year, genres, first_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(media_type, media_id, season, episode) DO UPDATE SET
               position = excluded.position,
               duration = CASE WHEN excluded.duration > 0 THEN excluded.duration ELSE watch_progress.duration END,
               finished = CASE WHEN excluded.finished = 1 THEN 1 WHEN excluded.position < 30 THEN watch_progress.finished ELSE excluded.finished END,
               title = CASE WHEN excluded.title <> '' THEN excluded.title ELSE watch_progress.title END,
               poster = COALESCE(excluded.poster, watch_progress.poster),
               backdrop = COALESCE(excluded.backdrop, watch_progress.backdrop),
               year = COALESCE(excluded.year, watch_progress.year),
               genres = CASE WHEN excluded.genres <> '[]' THEN excluded.genres ELSE watch_progress.genres END,
               updated_at = excluded.updated_at",
            params![
                rec.kind,
                rec.id,
                season,
                episode,
                position,
                duration,
                i64::from(finished),
                rec.title,
                rec.poster,
                rec.backdrop,
                rec.year,
                genres,
                now,
                now
            ],
        )?;
        if let Some(delta) = delta {
            db.execute(
                "INSERT INTO watch_time (day, media_type, media_id, season, episode, seconds) VALUES (?, ?, ?, ?, ?, ?)
                 ON CONFLICT(day, media_type, media_id, season, episode) DO UPDATE SET seconds = seconds + excluded.seconds",
                params![local_day(now), rec.kind, rec.id, season, episode, delta],
            )?;
        }
        Ok(Saved { finished })
    }

    fn progress(&self, sql: &str, args: impl rusqlite::Params) -> Result<Vec<Progress>, MediaError> {
        let db = self.db.lock();
        let mut stmt = db.prepare(sql)?;
        let rows = stmt.query_map(args, Progress::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Toute la progression d'un titre, la plus récente d'abord.
    pub fn show(&self, kind: &str, id: &str) -> Result<Vec<Progress>, MediaError> {
        self.progress(
            &format!(
                "SELECT {PROGRESS_COLUMNS} FROM watch_progress WHERE media_type = ? AND media_id = ? \
                 ORDER BY updated_at DESC"
            ),
            params![kind, id],
        )
    }

    /// « Continuer à regarder » : le dernier visionnage de chaque titre, s'il n'est pas terminé.
    pub fn continue_list(&self, limit: i64) -> Result<Vec<Progress>, MediaError> {
        self.progress(
            &format!(
                "SELECT {PROGRESS_COLUMNS} FROM watch_progress wp
                 WHERE finished = 0 AND position >= 10
                   AND updated_at = (SELECT MAX(updated_at) FROM watch_progress
                                     WHERE media_type = wp.media_type AND media_id = wp.media_id)
                 ORDER BY updated_at DESC LIMIT ?"
            ),
            [limit],
        )
    }

    pub fn history(&self, limit: i64) -> Result<Vec<Progress>, MediaError> {
        self.progress(
            &format!(
                "SELECT {PROGRESS_COLUMNS} FROM watch_progress WHERE position >= 10 ORDER BY updated_at DESC LIMIT ?"
            ),
            [limit],
        )
    }

    /// Oublie toute la progression d'un titre.
    pub fn remove_show(&self, kind: &str, id: &str) -> Result<(), MediaError> {
        self.db
            .lock()
            .execute("DELETE FROM watch_progress WHERE media_type = ? AND media_id = ?", params![kind, id])?;
        self.changed("progress");
        Ok(())
    }

    /// Efface l'historique et le temps regardé.
    pub fn clear_history(&self) -> Result<(), MediaError> {
        {
            let mut db = self.db.lock();
            crate::db::transaction(&mut db, |tx| {
                tx.execute_batch("DELETE FROM watch_progress; DELETE FROM watch_time;")
            })?;
        }
        self.last.lock().clear();
        self.changed("progress");
        Ok(())
    }

    // ─── Ma liste ───────────────────────────────────────────────────────────────────────────────────────────────

    pub fn list(&self) -> Result<Vec<WatchlistItem>, MediaError> {
        let db = self.db.lock();
        let mut stmt = db.prepare(
            "SELECT media_type, media_id, title, poster, backdrop, year, rating, added_at FROM watchlist ORDER BY added_at DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(WatchlistItem {
                    kind: r.get(0)?,
                    id: r.get(1)?,
                    title: r.get(2)?,
                    poster: r.get(3)?,
                    backdrop: r.get(4)?,
                    year: r.get(5)?,
                    rating: r.get(6)?,
                    added_at: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Ajoute le titre à la liste, ou l'en retire. Rend `true` s'il y est maintenant.
    pub fn toggle(&self, item: &ListEntry) -> Result<bool, MediaError> {
        let added = {
            let mut db = self.db.lock();
            crate::db::transaction(&mut db, |tx| {
                let existing = tx
                    .query_row(
                        "SELECT 1 FROM watchlist WHERE media_type = ? AND media_id = ?",
                        params![item.kind, item.id],
                        |_| Ok(()),
                    )
                    .optional()?;
                if existing.is_some() {
                    tx.execute(
                        "DELETE FROM watchlist WHERE media_type = ? AND media_id = ?",
                        params![item.kind, item.id],
                    )?;
                } else {
                    tx.execute(
                        "INSERT INTO watchlist (media_type, media_id, title, poster, backdrop, year, rating, added_at) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                        params![
                            item.kind,
                            item.id,
                            item.title,
                            item.poster,
                            item.backdrop,
                            item.year,
                            item.rating,
                            now_ms()
                        ],
                    )?;
                }
                Ok(existing.is_none())
            })?
        };
        self.changed("list");
        Ok(added)
    }

    /// `false` (rien changé) quand l'objet n'a pas de type ou d'identifiant valide, comme l'original.
    pub fn toggle_json(&self, item: &Value) -> Result<bool, MediaError> {
        match ListEntry::from_json(item) {
            Some(item) => self.toggle(&item),
            None => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests;

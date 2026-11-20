//! Normalisation du volume : mesure du niveau sonore des morceaux (EBU R128) en arrière-plan, et gain à appliquer.
//!
//! Un morceau à la fois, sans gêner la lecture ni le serveur, et seulement tant que le réglage `normalize` est actif.
//! Le niveau intégré (LUFS) et la crête vraie (dBTP) vont dans `tracks.loudness` et `tracks.true_peak` ;
//! `tracks.loudness_checked` marque un morceau tenté, mesurable ou non, pour ne pas y revenir à chaque passage.
//! Le lecteur applique ensuite à chaque morceau le gain de [`gain_db`] pour que tout sonne au même niveau.
//!
//! La version d'origine passait par ffmpeg ; ici le décodage est fait par symphonia et la mesure par `ebur128`.
//! ffmpeg ne sert plus qu'aux formats que symphonia ne lit pas (Opus, ALAC…), quand il est installé.

pub mod decode;
mod measure;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::library::SharedDb;
use crate::model::{Emit, Event, TrackId};
use crate::store::Store;

pub use measure::{Level, measure_file, measure_interleaved, parse_summary};

/// On n'atténue pas davantage (dB) : un morceau très fort reste fort.
pub const MIN_GAIN: f64 = -18.0;
/// On ne relève pas davantage (dB) : un morceau très faible reste faible.
pub const MAX_GAIN: f64 = 9.0;
/// dBTP : une relève ne doit jamais pousser la crête au-delà.
const HEADROOM: f64 = -1.0;
/// Niveau visé par défaut (réglage `normalizeTarget`).
pub const DEFAULT_TARGET: f64 = -16.0;

/// Gain (dB) à appliquer à un morceau de niveau `loudness` (LUFS) et de crête `true_peak` (dBTP) pour qu'il sonne à
/// `target` LUFS. Un morceau pas encore mesuré reste inchangé ; une relève s'arrête avant d'écrêter, et sans crête
/// connue on reste prudent (pas de relève). L'option coupée se traite chez l'appelant (gain nul).
pub fn gain_db(loudness: Option<f64>, true_peak: Option<f64>, target: f64) -> f64 {
    let Some(loudness) = loudness.filter(|l| l.is_finite()) else { return 0.0 };
    let target = if target.is_finite() && target != 0.0 { target } else { DEFAULT_TARGET };
    let mut db = (target - loudness).clamp(MIN_GAIN, MAX_GAIN);
    if db > 0.0 {
        let room = true_peak.filter(|p| p.is_finite()).map_or(0.0, |p| HEADROOM - p);
        db = db.min(room).max(0.0);
    }
    db
}

/// dB → facteur linéaire.
pub fn to_linear(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// Où en est la mesure (canal `loudness:progress`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub enabled: bool,
    pub total: i64,
    pub done: i64,
    pub running: bool,
    /// Nom d'origine, gardé pour l'interface : la mesure est possible. Elle l'est toujours ici (décodeur intégré) ;
    /// ffmpeg n'est plus qu'un appoint pour quelques formats.
    pub ffmpeg: bool,
}

/// Niveau connu d'un morceau (`loudness:levels`), champs nuls tant qu'il n'est pas mesuré.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackLevel {
    pub loudness: Option<f64>,
    pub true_peak: Option<f64>,
}

/// Chemin de ffmpeg s'il est installé (relu à chaque morceau : il peut l'être en cours de route).
pub type FfmpegPath = Arc<dyn Fn() -> Option<PathBuf> + Send + Sync>;

/// Première passe peu après le démarrage, pour ne pas alourdir l'ouverture.
const FIRST_DELAY: Duration = Duration::from_secs(20);
/// Puis une relance régulière (morceaux ajoutés entre-temps).
const PERIOD: Duration = Duration::from_secs(5 * 60);
/// Pause entre deux morceaux : laisse de la place au reste.
const PAUSE: Duration = Duration::from_millis(40);
const EMIT_EVERY: Duration = Duration::from_secs(2);

struct Shared {
    db: SharedDb,
    store: Store,
    ffmpeg: FfmpegPath,
    emit: Emit,
    kicked: Mutex<bool>,
    cv: Condvar,
    closed: AtomicBool,
    running: AtomicBool,
}

/// Mesure en arrière-plan, sur un thread à elle.
pub struct Loudness {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Loudness {
    pub fn new(db: SharedDb, store: Store, ffmpeg: FfmpegPath, emit: Emit) -> Loudness {
        let shared = Arc::new(Shared {
            db,
            store,
            ffmpeg,
            emit,
            kicked: Mutex::new(false),
            cv: Condvar::new(),
            closed: AtomicBool::new(false),
            running: AtomicBool::new(false),
        });
        let worker = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("control-loudness".into())
            .spawn(move || worker.run())
            .map_err(|err| log::warn!("Thread de mesure du volume impossible : {err}"))
            .ok();
        Loudness { shared, thread: Mutex::new(thread) }
    }

    /// Le réglage `normalize` est actif.
    pub fn enabled(&self) -> bool {
        self.shared.enabled()
    }

    pub fn progress(&self) -> Progress {
        self.shared.progress()
    }

    /// Les niveaux de ces morceaux (50 au plus) : `None` pour un morceau inconnu. Le lecteur les demande pour ceux
    /// qu'il ne connaît pas encore.
    pub fn levels(&self, ids: &[TrackId]) -> BTreeMap<TrackId, Option<TrackLevel>> {
        let db = self.shared.db.lock();
        let mut out = BTreeMap::new();
        let Ok(mut stmt) = db.prepare_cached("SELECT loudness, true_peak FROM tracks WHERE id = ?") else { return out };
        for &id in ids.iter().take(50) {
            let level = stmt
                .query_row([id], |r| Ok(TrackLevel { loudness: r.get(0)?, true_peak: r.get(1)? }))
                .optional()
                .ok()
                .flatten();
            out.insert(id, level);
        }
        out
    }

    /// (Re)lance la mesure si elle est activée ; sans effet si elle tourne déjà (elle prendra les nouveaux morceaux).
    /// Rend `false` si rien ne peut se lancer (fermé, ou option coupée).
    pub fn kick(&self) -> bool {
        self.shared.kick()
    }

    /// Les morceaux déjà mesurés sont gardés ; ceci ne sert qu'à tout refaire (décodeur mis à jour, par exemple).
    pub fn reset(&self) {
        {
            let db = self.shared.db.lock();
            if let Err(err) =
                db.execute("UPDATE tracks SET loudness = NULL, true_peak = NULL, loudness_checked = 0", [])
            {
                log::warn!("Remise à zéro des niveaux impossible : {err}");
            }
        }
        self.kick();
    }

    /// Arrête la mesure (le morceau en cours est abandonné sans être marqué) et attend la fin du thread.
    pub fn close(&self) {
        self.shared.closed.store(true, Ordering::SeqCst);
        {
            let _guard = self.shared.kicked.lock();
            self.shared.cv.notify_all();
        }
        if let Some(handle) = self.thread.lock().take()
            && handle.thread().id() != std::thread::current().id()
        {
            let _ = handle.join();
        }
    }
}

impl Drop for Loudness {
    fn drop(&mut self) {
        self.close();
    }
}

impl Shared {
    fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn enabled(&self) -> bool {
        self.store.get("normalize").as_bool().unwrap_or(false)
    }

    fn progress(&self) -> Progress {
        let (total, done) = {
            let db = self.db.lock();
            let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0);
            (
                count("SELECT COUNT(*) FROM tracks WHERE kind = 'audio'"),
                count("SELECT COUNT(*) FROM tracks WHERE kind = 'audio' AND loudness_checked = 1"),
            )
        };
        Progress { enabled: self.enabled(), total, done, running: self.running.load(Ordering::SeqCst), ffmpeg: true }
    }

    fn emit_progress(&self) {
        let value = serde_json::to_value(self.progress()).unwrap_or_default();
        (self.emit)(Event::Json("loudness:progress", value));
    }

    fn kick(&self) -> bool {
        if self.closed() || !self.enabled() {
            return false;
        }
        let mut kicked = self.kicked.lock();
        *kicked = true;
        self.cv.notify_all();
        true
    }

    /// Attend `d`, ou moins si l'on ferme.
    fn pause(&self, d: Duration) {
        let mut kicked = self.kicked.lock();
        if !self.closed() {
            self.cv.wait_for(&mut kicked, d);
        }
    }

    fn run(&self) {
        let mut wait = FIRST_DELAY;
        loop {
            {
                let mut kicked = self.kicked.lock();
                if !*kicked && !self.closed() {
                    self.cv.wait_for(&mut kicked, wait);
                }
                *kicked = false;
            }
            if self.closed() {
                return;
            }
            wait = PERIOD;
            if !self.enabled() {
                continue;
            }
            self.running.store(true, Ordering::SeqCst);
            self.measure_all();
            self.running.store(false, Ordering::SeqCst);
            if self.closed() {
                return;
            }
            self.emit_progress();
        }
    }

    /// Mesure les morceaux pas encore tentés, un par un, tant que l'option reste active.
    fn measure_all(&self) {
        let mut last_emit: Option<Instant> = None;
        while !self.closed() && self.enabled() {
            let row: Option<(TrackId, String)> = {
                let db = self.db.lock();
                match db
                    .query_row(
                        "SELECT id, path FROM tracks WHERE kind = 'audio' AND loudness_checked = 0 ORDER BY id LIMIT 1",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                {
                    Ok(row) => row,
                    Err(err) => {
                        log::warn!("Mesure du volume : lecture de la base impossible : {err}");
                        return;
                    }
                }
            };
            let Some((id, path)) = row else { return };
            let file = Path::new(&path);
            // Fichier absent ou illisible : marqué comme tenté, on passe au suivant.
            let level = if file.is_file() {
                let ffmpeg = (self.ffmpeg)();
                measure_file(file, ffmpeg.as_deref(), &|| self.closed())
            } else {
                None
            };
            if self.closed() {
                return;
            }
            let saved = {
                let db = self.db.lock();
                db.execute(
                    "UPDATE tracks SET loudness = ?, true_peak = ?, loudness_checked = 1 WHERE id = ?",
                    params![level.map(|l| l.loudness), level.and_then(|l| l.true_peak), id],
                )
            };
            if let Err(err) = saved {
                // Sans quoi on reprendrait le même morceau indéfiniment.
                log::warn!("Mesure du volume : enregistrement impossible : {err}");
                return;
            }
            if last_emit.is_none_or(|t| t.elapsed() > EMIT_EVERY) {
                last_emit = Some(Instant::now());
                self.emit_progress();
            }
            self.pause(PAUSE);
        }
    }
}

#[cfg(test)]
mod tests;

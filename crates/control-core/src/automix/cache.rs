//! Cache des analyses AutoMix, dans un fichier JSON.
//!
//! L'original gardait ses résultats dans le `localStorage` de l'interface (`{ v, items: { id: { s, a } } }`, clé
//! `control.automix`), indexés par identifiant de morceau avec une signature « taille:durée ». Le Rust n'a pas de
//! `localStorage` : même forme de fichier, mais indexée par chemin, avec une signature « taille:mtime » prise sur le
//! fichier lui-même (un fichier remplacé ou retouché est donc réanalysé). Seules les réussites vont sur le disque ;
//! un échec n'est retenu qu'en mémoire, le temps de la session, pour ne pas insister.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::Analysis;
use crate::util::{path_key, write_atomic};

/// Nom du fichier de cache dans le dossier donné.
pub const CACHE_FILE: &str = "automix.json";
/// À incrémenter quand l'analyse change : les résultats en cache sont alors refaits.
pub const CACHE_VERSION: u32 = 2;
/// Les écritures sont regroupées : au plus une toutes les 2,5 s (le reste part avec [`AnalysisCache::flush`]).
const SAVE_EVERY: Duration = Duration::from_millis(2500);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    s: String,
    a: Analysis,
}

#[derive(Deserialize)]
struct Disk {
    v: u32,
    items: HashMap<String, Entry>,
}

#[derive(Serialize)]
struct DiskRef<'a> {
    v: u32,
    items: &'a HashMap<String, Entry>,
}

#[derive(Default)]
struct Inner {
    items: HashMap<String, Entry>,
    /// Chemin → signature des échecs de cette session.
    failed: HashMap<String, String>,
    dirty: bool,
    last_save: Option<Instant>,
}

/// Signature d'un fichier : « taille:mtime » (millisecondes). `None` si le fichier est introuvable.
pub fn signature(path: &Path) -> Option<String> {
    let md = std::fs::metadata(path).ok()?;
    let mtime = md.modified().ok()?.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    Some(format!("{}:{mtime}", md.len()))
}

fn key_of(path: &Path) -> String {
    path_key(&path.to_string_lossy())
}

/// Cache partagé entre threads ; une seule analyse à la fois, pour ne pas peser sur la lecture.
pub struct AnalysisCache {
    file: PathBuf,
    inner: Mutex<Inner>,
    run: Mutex<()>,
    save: Mutex<()>,
}

impl AnalysisCache {
    /// Ouvre le cache de `dir` (fichier [`CACHE_FILE`]) ; absent, illisible ou d'une autre version : vide.
    pub fn open(dir: &Path) -> AnalysisCache {
        let file = dir.join(CACHE_FILE);
        let items = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Disk>(&bytes).ok())
            .filter(|d| d.v == CACHE_VERSION)
            .map(|d| d.items)
            .unwrap_or_default();
        AnalysisCache {
            file,
            inner: Mutex::new(Inner { items, ..Inner::default() }),
            run: Mutex::new(()),
            save: Mutex::new(()),
        }
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Analyse déjà connue : `None` si elle n'a pas encore été faite (ou si le fichier a changé depuis),
    /// `Some(None)` si elle a échoué, `Some(Some(a))` sinon.
    pub fn peek(&self, path: &Path) -> Option<Option<Analysis>> {
        let sig = signature(path)?;
        self.peek_sig(&key_of(path), &sig)
    }

    fn peek_sig(&self, key: &str, sig: &str) -> Option<Option<Analysis>> {
        let inner = self.inner.lock();
        if let Some(e) = inner.items.get(key)
            && e.s == sig
        {
            return Some(Some(e.a));
        }
        if inner.failed.get(key).is_some_and(|s| s == sig) {
            return Some(None);
        }
        None
    }

    /// Analyse d'un fichier, prise dans le cache ou faite maintenant (bloquant : à appeler depuis un thread de
    /// travail). `None` si elle est impossible (format illisible, morceau trop long…).
    pub fn analyze(&self, path: &Path) -> Option<Analysis> {
        let key = key_of(path);
        let sig = signature(path)?;
        if let Some(known) = self.peek_sig(&key, &sig) {
            return known;
        }
        let _one_at_a_time = self.run.lock();
        // Un autre thread vient peut-être de la faire.
        if let Some(known) = self.peek_sig(&key, &sig) {
            return known;
        }
        let result = match super::analyze_file(path) {
            Ok(a) => Some(a),
            Err(err) => {
                log::info!("AutoMix : analyse impossible ({}) : {err:#}", path.display());
                None
            }
        };
        self.remember_sig(key, sig, result);
        result
    }

    /// Retient une analyse faite ailleurs (`None` : échec).
    pub fn remember(&self, path: &Path, analysis: Option<Analysis>) {
        if let Some(sig) = signature(path) {
            self.remember_sig(key_of(path), sig, analysis);
        }
    }

    fn remember_sig(&self, key: String, sig: String, analysis: Option<Analysis>) {
        let due = {
            let mut inner = self.inner.lock();
            match analysis {
                Some(a) => {
                    inner.failed.remove(&key);
                    inner.items.insert(key, Entry { s: sig, a });
                    inner.dirty = true;
                }
                None => {
                    inner.failed.insert(key, sig);
                }
            }
            inner.dirty && inner.last_save.is_none_or(|t| t.elapsed() >= SAVE_EVERY)
        };
        if due {
            self.flush();
        }
    }

    /// Écrit le cache s'il a changé.
    pub fn flush(&self) {
        let _one_writer = self.save.lock();
        let bytes = {
            let mut inner = self.inner.lock();
            if !inner.dirty {
                return;
            }
            let bytes = serde_json::to_vec(&DiskRef { v: CACHE_VERSION, items: &inner.items });
            inner.dirty = false;
            inner.last_save = Some(Instant::now());
            bytes
        };
        let written = bytes.map_err(std::io::Error::other).and_then(|b| write_atomic(&self.file, &b));
        if let Err(err) = written {
            log::warn!("AutoMix : écriture du cache impossible ({}) : {err}", self.file.display());
            self.inner.lock().dirty = true;
        }
    }

    /// Oublie toutes les analyses (mémoire et disque).
    pub fn forget_all(&self) {
        let _one_writer = self.save.lock();
        {
            let mut inner = self.inner.lock();
            inner.items.clear();
            inner.failed.clear();
            inner.dirty = false;
        }
        match std::fs::remove_file(&self.file) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => log::warn!("AutoMix : suppression du cache impossible : {err}"),
        }
    }
}

impl Drop for AnalysisCache {
    fn drop(&mut self) {
        self.flush();
    }
}

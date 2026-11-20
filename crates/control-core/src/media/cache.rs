//! Cache des réponses de catalogue dans la table `media_cache` : durée de vie, requêtes identiques regroupées, et
//! repli sur une réponse périmée quand le réseau est coupé (les lignes périmées restent 7 jours).
//!
//! Les réponses sont gardées déjà normalisées, au format JSON de la version d'origine : les deux versions
//! partagent le même cache.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::error::MediaError;
use crate::library::SharedDb;
use crate::util::now_ms;

pub const MINUTE: i64 = 60 * 1000;
pub const HOUR: i64 = 60 * MINUTE;
pub const DAY: i64 = 24 * HOUR;

/// Au-delà, une réponse périmée n'est plus gardée pour le mode hors ligne.
const KEEP_STALE: i64 = 7 * DAY;

pub struct Cache {
    db: SharedDb,
    /// Un verrou par clé en cours de chargement : une même requête lancée deux fois n'interroge le réseau qu'une fois.
    inflight: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl Cache {
    pub fn new(db: SharedDb) -> Cache {
        let cache = Cache { db, inflight: Mutex::new(HashMap::new()) };
        cache.purge();
        cache
    }

    fn row(&self, key: &str) -> Option<(String, i64)> {
        let db = self.db.lock();
        db.query_row("SELECT payload, expires_at FROM media_cache WHERE key = ?", [key], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .optional()
        .ok()
        .flatten()
    }

    /// Valeur fraîche. Une ligne illisible (format inattendu) compte comme absente.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let (payload, expires_at) = self.row(key)?;
        if expires_at <= now_ms() {
            return None;
        }
        serde_json::from_str(&payload).ok()
    }

    /// Valeur, même périmée.
    pub fn stale<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let (payload, _) = self.row(key)?;
        serde_json::from_str(&payload).ok()
    }

    pub fn set<T: Serialize>(&self, key: &str, value: &T, ttl_ms: i64) {
        let Ok(payload) = serde_json::to_string(value) else { return };
        let db = self.db.lock();
        let res = db.execute(
            "INSERT INTO media_cache (key, payload, expires_at) VALUES (?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET payload = excluded.payload, expires_at = excluded.expires_at",
            params![key, payload, now_ms() + ttl_ms],
        );
        if let Err(err) = res {
            log::warn!("[media] cache non enregistré : {err}");
        }
    }

    /// Le cache s'il est frais ; sinon `fetch`, mémorisé, avec repli sur le périmé en cas d'échec passager.
    /// Le verrou de la base n'est jamais tenu pendant `fetch`.
    pub fn memo<T, F>(&self, key: &str, ttl_ms: i64, fetch: F) -> Result<T, MediaError>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Result<T, MediaError>,
    {
        if let Some(hit) = self.get(key) {
            return Ok(hit);
        }
        let gate = self.inflight.lock().entry(key.to_string()).or_default().clone();
        let result = {
            let _turn = gate.lock();
            // Une requête identique vient peut-être de finir pendant l'attente.
            match self.get(key) {
                Some(hit) => Ok(hit),
                None => match fetch() {
                    Ok(value) => {
                        self.set(key, &value, ttl_ms);
                        Ok(value)
                    }
                    Err(err) if err.code.is_transient() => self.stale(key).ok_or(err),
                    Err(err) => Err(err),
                },
            }
        };
        let mut inflight = self.inflight.lock();
        // La table et nous : personne d'autre n'attend cette clé.
        if Arc::strong_count(&gate) == 2 {
            inflight.remove(key);
        }
        result
    }

    /// Supprime les réponses périmées depuis plus de 7 jours.
    pub fn purge(&self) {
        let db = self.db.lock();
        if let Err(err) = db.execute("DELETE FROM media_cache WHERE expires_at < ?", [now_ms() - KEEP_STALE]) {
            log::warn!("[media] purge du cache : {err}");
        }
    }

    pub fn clear(&self) -> Result<(), MediaError> {
        self.db.lock().execute_batch("DELETE FROM media_cache")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::error::ErrorCode;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn cache() -> Cache {
        Cache::new(Arc::new(Mutex::new(crate::db::open_in_memory().unwrap())))
    }

    #[test]
    fn memo_caches_and_falls_back_to_stale() {
        let c = cache();
        let calls = AtomicUsize::new(0);
        let fetch = || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![1, 2])
        };
        assert_eq!(c.memo("k", MINUTE, fetch).unwrap(), vec![1, 2]);
        assert_eq!(c.memo("k", MINUTE, || Ok(vec![9])).unwrap(), vec![1, 2]);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Périmé : on retente le réseau ; hors ligne, l'ancienne réponse sert encore.
        c.set("k", &vec![3], -1);
        assert_eq!(c.get::<Vec<i32>>("k"), None);
        let offline = || Err::<Vec<i32>, _>(MediaError::new(ErrorCode::Network, "hors ligne"));
        assert_eq!(c.memo("k", MINUTE, offline).unwrap(), vec![3]);
        // Une clé refusée n'est pas masquée par le cache.
        let refused = || Err::<Vec<i32>, _>(MediaError::new(ErrorCode::Auth, "non"));
        assert_eq!(c.memo("k", MINUTE, refused).unwrap_err().code, ErrorCode::Auth);
        assert_eq!(c.memo("absent", MINUTE, offline).unwrap_err().code, ErrorCode::Network);
        assert!(c.inflight.lock().is_empty());

        c.clear().unwrap();
        assert_eq!(c.stale::<Vec<i32>>("k"), None);
    }

    #[test]
    fn purge_drops_week_old_rows() {
        let c = cache();
        c.set("old", &1, -8 * DAY);
        c.set("recent", &2, -DAY);
        c.purge();
        assert_eq!(c.stale::<i32>("old"), None);
        assert_eq!(c.stale::<i32>("recent"), Some(2));
    }

    #[test]
    fn identical_requests_share_one_fetch() {
        let c = cache();
        let calls = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    c.memo("same", MINUTE, || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        Ok(7)
                    })
                    .unwrap()
                });
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

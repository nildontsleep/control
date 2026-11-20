//! « En écoute » : ce que les comptes écoutent en ce moment. Un compte ne le montre que s'il l'a choisi (désactivé
//! par défaut), et seulement à ceux avec qui il partage sa bibliothèque (un partage explicite, pas l'accès de
//! l'administrateur). Rien n'est gardé sur le disque : c'est de la mémoire vive, qui s'efface à la fin du morceau.

use std::collections::HashMap;

use parking_lot::Mutex;
use serde_json::{Map, Value, json};

use crate::hub::Hub;
use crate::util::now_ms;

/// Une pause un peu longue ne fait pas disparaître le morceau tout de suite.
const GRACE_MS: i64 = 60 * 1000;

#[derive(Debug, Clone)]
pub struct Listening {
    pub track: Value,
    pub play_id: i64,
    pub started_at: i64,
    pub expires_at: i64,
}

#[derive(Default)]
pub struct Presence {
    now: Mutex<HashMap<String, Listening>>,
}

impl Presence {
    fn enabled(hub: &Hub, owner: &str) -> bool {
        hub.db
            .lock()
            .query_row("SELECT share_listening FROM users WHERE id = ? AND disabled = 0", [owner], |r| {
                r.get::<_, i64>(0)
            })
            .map(|v| v != 0)
            .unwrap_or(false)
    }

    /// Les comptes à qui `owner` montre ce qu'il écoute : un partage de bibliothèque explicite (ou « tout le monde »).
    fn viewers(hub: &Hub, owner: &str) -> Vec<String> {
        let db = hub.db.lock();
        let rows: Vec<String> = db
            .prepare("SELECT viewer_id FROM shares WHERE owner_id = ? AND browse = 1")
            .and_then(|mut s| s.query_map([owner], |r| r.get(0)).map(|r| r.flatten().collect()))
            .unwrap_or_default();
        if rows.iter().any(|r| r == "*") {
            return db
                .prepare("SELECT id FROM users WHERE disabled = 0 AND id <> ?")
                .and_then(|mut s| s.query_map([owner], |r| r.get(0)).map(|r| r.flatten().collect()))
                .unwrap_or_default();
        }
        rows.into_iter().filter(|id| id != owner).collect()
    }

    fn visible_to(hub: &Hub, viewer: &str, owner: &str) -> bool {
        viewer != owner
            && hub
                .db
                .lock()
                .query_row(
                    "SELECT 1 FROM shares WHERE owner_id = ? AND browse = 1 AND viewer_id IN (?, '*')",
                    [owner, viewer],
                    |_| Ok(()),
                )
                .is_ok()
    }

    fn view(rec: &Listening) -> Value {
        let t = &rec.track;
        json!({
            "trackId": t["id"], "title": t["title"], "artist": t["artist"], "album": t["album"], "cover": t["cover"],
            "duration": t["duration"], "startedAt": rec.started_at,
        })
    }

    fn notify(hub: &Hub, owner: &str, payload: Value) {
        for viewer in Presence::viewers(hub, owner) {
            if let Some(entry) = hub.loaded(&viewer) {
                entry.emit("social:listening", &payload);
            }
        }
    }

    /// Un morceau commence chez ce compte (`track` : `{ id, kind, title, artist, album, cover, duration }`).
    pub fn start(&self, hub: &Hub, owner: &str, play_id: i64, track: Value) {
        if track["kind"] != "audio" || !Presence::enabled(hub, owner) {
            return;
        }
        let duration = track["duration"].as_f64().filter(|d| *d > 0.0).unwrap_or(240.0);
        let now = now_ms();
        let rec =
            Listening { track, play_id, started_at: now, expires_at: now + (duration * 1000.0) as i64 + GRACE_MS };
        let payload = json!({ "id": owner, "now": Presence::view(&rec) });
        self.now.lock().insert(owner.to_string(), rec);
        Presence::notify(hub, owner, payload);
    }

    /// Il s'arrête (fin du morceau ou passage au suivant : un nouveau `start` suit aussitôt).
    pub fn end(&self, hub: &Hub, owner: &str, play_id: Option<i64>) {
        let matches = self.now.lock().get(owner).is_some_and(|r| play_id.is_none_or(|p| p == r.play_id));
        if matches {
            self.clear(hub, owner);
        }
    }

    pub fn clear(&self, hub: &Hub, owner: &str) {
        if self.now.lock().remove(owner).is_some() {
            Presence::notify(hub, owner, json!({ "id": owner, "now": null }));
        }
    }

    pub fn sweep(&self, hub: &Hub) {
        let t = now_ms();
        let expired: Vec<String> =
            self.now.lock().iter().filter(|(_, r)| r.expires_at < t).map(|(k, _)| k.clone()).collect();
        for owner in expired {
            self.clear(hub, &owner);
        }
    }

    /// Avance l'échéance d'une écoute (essais).
    pub fn expire(&self, owner: &str) {
        if let Some(r) = self.now.lock().get_mut(owner) {
            r.expires_at = now_ms() - 1;
        }
    }

    /// Ce que `viewer` voit : `{ [ownerId]: { trackId, title, artist, … } }`.
    pub fn snapshot_for(&self, hub: &Hub, viewer: &str) -> Map<String, Value> {
        let now = now_ms();
        let list: Vec<(String, Listening)> =
            self.now.lock().iter().filter(|(_, r)| r.expires_at >= now).map(|(k, r)| (k.clone(), r.clone())).collect();
        list.into_iter()
            .filter(|(owner, _)| Presence::visible_to(hub, viewer, owner))
            .map(|(owner, rec)| (owner, Presence::view(&rec)))
            .collect()
    }
}

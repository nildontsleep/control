//! Un compte chargé : son noyau et les pages ouvertes (flux d'événements).

use std::collections::HashMap;
use std::sync::Weak;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use bytes::Bytes;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::Hub;
use crate::util::{HttpError, http_error, now_ms};
use control_core::core::{Core, Host};
use control_core::model::Event;

/// Une trame SSE : `event: <canal>\ndata: <json>\n\n`.
pub fn frame(channel: &str, payload: &Value) -> Bytes {
    Bytes::from(format!(
        "event: {channel}\ndata: {}\n\n",
        serde_json::to_string(payload).unwrap_or_else(|_| "null".into())
    ))
}

/// Les pages ouvertes d'un compte : identifiant du client → (connexion, envoi).
#[derive(Default)]
pub struct Clients {
    map: Mutex<HashMap<String, (u64, UnboundedSender<Bytes>)>>,
    seq: AtomicU64,
}

impl Clients {
    /// Envoie une trame à toutes les pages, sauf `except`.
    pub fn send(&self, channel: &str, payload: &Value, except: Option<&str>) {
        let data = frame(channel, payload);
        self.map.lock().retain(|id, (_, tx)| Some(id.as_str()) == except || tx.send(data.clone()).is_ok());
    }

    pub fn raw(&self, data: Bytes) {
        self.map.lock().retain(|_, (_, tx)| tx.send(data.clone()).is_ok());
    }

    /// Une page se connecte (elle remplace son ancienne connexion) : (numéro de connexion, flux).
    pub fn add(&self, id: &str) -> (u64, UnboundedReceiver<Bytes>) {
        let (tx, rx) = unbounded_channel();
        let conn = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.map.lock().insert(id.to_string(), (conn, tx));
        (conn, rx)
    }

    pub fn remove(&self, id: &str, conn: u64) {
        let mut map = self.map.lock();
        if map.get(id).is_some_and(|(c, _)| *c == conn) {
            map.remove(id);
        }
    }

    pub fn len(&self) -> usize {
        let mut map = self.map.lock();
        map.retain(|_, (_, tx)| !tx.is_closed());
        map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Ferme toutes les connexions.
    pub fn close_all(&self) {
        self.map.lock().clear();
    }
}

/// Traduit un événement du noyau en (canal, contenu) d'origine.
pub fn event_json(event: Event) -> (&'static str, Value) {
    match event {
        Event::LibraryChanged(what) => ("lib:changed", json!({ "what": what })),
        Event::ScanProgress(p) => ("lib:scan-progress", serde_json::to_value(p).unwrap_or(Value::Null)),
        Event::Json(channel, value) => (channel, value),
    }
}

pub struct Entry {
    pub id: String,
    pub clients: std::sync::Arc<Clients>,
    pub core: Core,
    pub last_used: AtomicI64,
    /// La liste des morceaux, compressée, gardée tant que la base n'a pas changé : (version, gzip).
    pub tracks_cache: Mutex<Option<(i64, std::sync::Arc<Vec<u8>>)>>,
    /// Modèle de l'API Subsonic (artistes, albums, genres), gardé tant que les morceaux ne changent pas.
    pub sub_model: Mutex<Option<std::sync::Arc<crate::subsonic::model::Model>>>,
}

impl Entry {
    pub fn emit(&self, channel: &str, payload: &Value) {
        self.clients.send(channel, payload, None);
    }

    pub fn touch(&self) {
        self.last_used.store(now_ms(), Ordering::SeqCst);
    }

    /// Nombre de modifications de la base depuis son ouverture (`total_changes()`).
    pub fn db_version(&self) -> i64 {
        self.core.db.lock().query_row("SELECT total_changes()", [], |r| r.get(0)).unwrap_or(-1)
    }
}

/// Ce que le noyau d'un compte demande au serveur.
pub struct ServerHost {
    pub hub: Weak<Hub>,
    pub user_id: String,
    pub clients: std::sync::Arc<Clients>,
}

impl Host for ServerHost {
    fn scan_roots(&self) -> Vec<String> {
        self.hub.upgrade().map(|h| h.roots_for(&self.user_id)).unwrap_or_default()
    }

    fn emit_others(&self, sender: &str, channel: &str, payload: &Value) {
        self.clients.send(channel, payload, Some(sender));
    }

    fn on_reset(&self) {
        if let Some(hub) = self.hub.upgrade()
            && let Some(entry) = hub.loaded(&self.user_id)
        {
            hub.mirror(&entry);
        }
    }

    fn guard_url(&self, url: &str) -> Result<String, HttpError> {
        control_core::urlguard::check_url(url).map_err(|e| http_error(400, "invalid_url", e.to_string()))
    }
}

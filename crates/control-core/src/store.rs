//! Réglages et état des fenêtres, dans un fichier JSON écrit de façon atomique.
//!
//! Le format est celui de la version Electron (`{ settings, window, servers, source }`) : un dossier de données
//! existant se relit tel quel. Seules les valeurs modifiées sont gardées dans `settings` ; les valeurs par défaut
//! viennent de [`crate::settings::defaults`].

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{Map, Value, json};

use crate::settings::{Settings, defaults};
use crate::util::write_atomic;

/// Fusion profonde : les objets se fusionnent clé par clé, le reste remplace (y compris les tableaux).
pub fn deep_merge(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(t), Value::Object(p)) => {
            for (k, v) in p {
                match t.get_mut(k) {
                    Some(existing) if existing.is_object() && v.is_object() => deep_merge(existing, v),
                    _ => {
                        t.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (t, p) => *t = p.clone(),
    }
}

#[derive(Debug, Default)]
struct Data {
    settings: Value,
    window: Value,
    servers: Value,
    source: String,
}

struct Inner {
    file: PathBuf,
    data: Data,
    dirty_since: Option<Instant>,
}

/// Fichier de réglages partagé entre threads. Les écritures sont regroupées (300 ms) puis faites par [`Store::flush`]
/// ou par le thread d'écriture lancé avec [`Store::spawn_writer`].
#[derive(Clone)]
pub struct Store {
    inner: Arc<Mutex<Inner>>,
}

impl Store {
    pub fn open(file: impl Into<PathBuf>) -> Store {
        let file = file.into();
        let mut data = Data { settings: json!({}), window: json!({}), servers: json!([]), source: "local".into() };
        if let Ok(text) = std::fs::read_to_string(&file)
            && let Ok(Value::Object(raw)) = serde_json::from_str::<Value>(&text)
        {
            if let Some(v @ Value::Object(_)) = raw.get("settings") {
                data.settings = v.clone();
            }
            if let Some(v @ Value::Object(_)) = raw.get("window") {
                data.window = v.clone();
            }
            if let Some(v @ Value::Array(_)) = raw.get("servers") {
                data.servers = v.clone();
            }
            if let Some(Value::String(s)) = raw.get("source") {
                data.source = s.clone();
            }
        }
        Store { inner: Arc::new(Mutex::new(Inner { file, data, dirty_since: None })) }
    }

    pub fn path(&self) -> PathBuf {
        self.inner.lock().file.clone()
    }

    /// Réglages modifiés par l'utilisateur (sans les valeurs par défaut).
    pub fn raw_settings(&self) -> Value {
        self.inner.lock().data.settings.clone()
    }

    /// Réglages complets : valeurs par défaut, puis celles de l'utilisateur.
    pub fn settings_value(&self) -> Value {
        let mut merged = defaults();
        deep_merge(&mut merged, &self.inner.lock().data.settings);
        merged
    }

    /// Réglages complets, typés.
    pub fn settings(&self) -> Settings {
        Settings::from_value(self.settings_value())
    }

    /// Une valeur de réglage (par défaut si absente).
    pub fn get(&self, key: &str) -> Value {
        let inner = self.inner.lock();
        match inner.data.settings.get(key) {
            Some(v) if !v.is_null() => v.clone(),
            _ => defaults().get(key).cloned().unwrap_or(Value::Null),
        }
    }

    pub fn patch(&self, patch: &Value) {
        if !patch.is_object() {
            return;
        }
        let mut inner = self.inner.lock();
        deep_merge(&mut inner.data.settings, patch);
        inner.dirty_since.get_or_insert_with(Instant::now);
    }

    pub fn reset(&self) {
        {
            let mut inner = self.inner.lock();
            inner.data.settings = json!({});
            inner.dirty_since = Some(Instant::now());
        }
        self.flush();
    }

    pub fn window_state(&self, key: &str) -> Value {
        self.inner.lock().data.window.get(key).cloned().unwrap_or(Value::Null)
    }

    pub fn set_window_state(&self, key: &str, value: Value) {
        let mut inner = self.inner.lock();
        if !inner.data.window.is_object() {
            inner.data.window = json!({});
        }
        if let Value::Object(m) = &mut inner.data.window {
            m.insert(key.to_string(), value);
        }
        inner.dirty_since.get_or_insert_with(Instant::now);
    }

    pub fn servers(&self) -> Value {
        self.inner.lock().data.servers.clone()
    }

    pub fn set_servers(&self, servers: Value) {
        let mut inner = self.inner.lock();
        inner.data.servers = servers;
        inner.dirty_since.get_or_insert_with(Instant::now);
    }

    pub fn source(&self) -> String {
        self.inner.lock().data.source.clone()
    }

    pub fn set_source(&self, source: &str) {
        let mut inner = self.inner.lock();
        inner.data.source = source.to_string();
        inner.dirty_since.get_or_insert_with(Instant::now);
    }

    /// Écrit le fichier s'il a changé depuis la dernière écriture.
    pub fn flush(&self) {
        let (file, snapshot) = {
            let mut inner = self.inner.lock();
            if inner.dirty_since.take().is_none() {
                return;
            }
            let mut m = Map::new();
            m.insert("settings".into(), inner.data.settings.clone());
            m.insert("window".into(), inner.data.window.clone());
            m.insert("servers".into(), inner.data.servers.clone());
            m.insert("source".into(), Value::String(inner.data.source.clone()));
            (inner.file.clone(), Value::Object(m))
        };
        let text = serde_json::to_string_pretty(&snapshot).unwrap_or_else(|_| "{}".into());
        if let Err(err) = write_atomic(&file, text.as_bytes()) {
            log::error!("Impossible d’enregistrer les réglages : {err}");
        }
    }

    /// Écrit au plus tard 300 ms après une modification, sur un thread à part (jamais sur celui de l'interface).
    /// Le thread s'arrête de lui-même quand le dernier `Store` est libéré.
    pub fn spawn_writer(&self) {
        let weak = Arc::downgrade(&self.inner);
        let spawned = std::thread::Builder::new().name("control-store".into()).spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(150));
                let Some(inner) = weak.upgrade() else { return };
                let due = inner.lock().dirty_since.is_some_and(|t| t.elapsed() >= Duration::from_millis(300));
                if due {
                    Store { inner }.flush();
                }
            }
        });
        if let Err(err) = spawned {
            log::warn!("Thread d’écriture des réglages impossible : {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_is_deep_for_objects_only() {
        let mut a = json!({"eq": {"enabled": false, "gains": [1, 2]}, "volume": 0.8});
        deep_merge(&mut a, &json!({"eq": {"enabled": true, "gains": [3]}, "x": {"y": 1}}));
        assert_eq!(a, json!({"eq": {"enabled": true, "gains": [3]}, "volume": 0.8, "x": {"y": 1}}));
    }

    #[test]
    fn roundtrip_keeps_only_patches() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("settings.json");
        let s = Store::open(&f);
        s.patch(&json!({"volume": 0.5, "dl": {"kind": "video"}}));
        s.flush();
        let again = Store::open(&f);
        assert_eq!(again.raw_settings(), json!({"volume": 0.5, "dl": {"kind": "video"}}));
        let typed = again.settings();
        assert!((typed.volume - 0.5).abs() < 1e-9);
        assert_eq!(typed.dl.kind, "video");
        assert_eq!(typed.dl.audio_format, "mp3");
    }
}

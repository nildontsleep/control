//! Tâches de fond : surveillance des dossiers, analyses périodiques, déchargement des comptes inactifs, battement
//! de cœur des flux d'événements, sessions expirées, mise à jour, sauvegardes, « en écoute ».

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bytes::Bytes;
use control_core::downloads::JobStatus;
use notify::Watcher as _;

use super::{Hub, IDLE_MS};
use crate::util::now_ms;

const HEARTBEAT_MS: i64 = 20 * 1000;

impl Hub {
    /// (Re)surveille tous les dossiers : un changement relance l'analyse 5 s plus tard.
    pub(super) fn watch(&self) {
        let mut watchers = self.watchers.lock();
        watchers.clear();
        for root in self.all_roots() {
            let me = self.me.clone();
            let handler = move |res: notify::Result<notify::Event>| {
                let Ok(event) = res else { return };
                // Une lecture (l'analyse elle-même ouvre les fichiers) ne change rien.
                if matches!(event.kind, notify::EventKind::Access(_)) {
                    return;
                }
                let temp = event.paths.iter().all(|p| {
                    let name = p.to_string_lossy().to_lowercase();
                    [".part", ".ytdl", ".tmp", ".temp"].iter().any(|e| name.ends_with(e))
                });
                if !event.paths.is_empty() && temp {
                    return;
                }
                if let Some(hub) = me.upgrade() {
                    hub.rescan_at.store(now_ms() + 5000, Ordering::SeqCst);
                }
            };
            match notify::recommended_watcher(handler) {
                Ok(mut w) => {
                    if w.watch(std::path::Path::new(&root), notify::RecursiveMode::Recursive).is_ok() {
                        watchers.push(w);
                    }
                }
                Err(err) => log::debug!("Surveillance impossible de {root} : {err}"),
            }
        }
    }

    fn reap(&self) {
        let now = now_ms();
        let idle: Vec<String> = self
            .loaded_entries()
            .into_iter()
            .filter(|e| e.clients.is_empty() && now - e.last_used.load(Ordering::SeqCst) >= IDLE_MS)
            .filter(|e| {
                !e.core
                    .downloads
                    .list()
                    .iter()
                    .any(|j| matches!(j.status, JobStatus::Queued | JobStatus::Downloading | JobStatus::Processing))
            })
            .map(|e| e.id.clone())
            .collect();
        for id in idle {
            if let Some(e) = self.entries.lock().remove(&id) {
                e.core.close();
            }
        }
    }

    /// Lance les tâches de fond (un fil qui se réveille chaque seconde).
    pub fn start(self: &Arc<Hub>) {
        self.watch();
        self.accounts.purge();
        self.updater.start();
        let tools_hub = Arc::downgrade(self);
        let _ = std::thread::Builder::new().name("control-tools".into()).spawn(move || {
            if let Some(hub) = tools_hub.upgrade() {
                hub.tools.refresh_versions();
                let status = serde_json::to_value(hub.tools.status()).unwrap_or_default();
                hub.broadcast("tools:status", &status);
            }
        });
        let weak = Arc::downgrade(self);
        let _ = std::thread::Builder::new().name("control-hub".into()).spawn(move || {
            let started = now_ms();
            let (mut reap, mut beat, mut purge, mut sweep, mut backup) = (started, started, started, started, started);
            let mut rescan = started;
            loop {
                std::thread::sleep(Duration::from_millis(500));
                let Some(hub) = weak.upgrade() else { return };
                if hub.is_closed() {
                    return;
                }
                let now = now_ms();
                let at = hub.rescan_at.load(Ordering::SeqCst);
                if at > 0 && now >= at {
                    hub.rescan_at.store(0, Ordering::SeqCst);
                    hub.rescan_all();
                }
                if now - reap >= 60 * 1000 {
                    reap = now;
                    hub.reap();
                }
                if now - rescan >= (hub.config.rescan_minutes * 60_000.0) as i64 {
                    rescan = now;
                    hub.rescan_all();
                }
                if now - purge >= 6 * 3600 * 1000 {
                    purge = now;
                    hub.accounts.purge();
                }
                if now - beat >= HEARTBEAT_MS {
                    beat = now;
                    for e in hub.loaded_entries() {
                        e.clients.raw(Bytes::from_static(b": ping\n\n"));
                    }
                }
                if now - sweep >= 15 * 1000 {
                    sweep = now;
                    hub.presence.sweep(&hub);
                }
                if now - backup >= 5 * 60 * 1000 {
                    backup = now;
                    hub.backups.tick(&hub);
                }
            }
        });
    }
}

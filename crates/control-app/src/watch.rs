//! Surveillance des dossiers de la bibliothèque : un fichier ajouté, modifié ou supprimé relance une analyse au bout
//! de quelques secondes de calme (les téléchargements en cours, `.part`, `.ytdl`, sont ignorés).

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::backend::Backend;

const QUIET: Duration = Duration::from_secs(5);

/// Ignore les fichiers temporaires des téléchargements et des éditeurs.
pub fn ignored(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    matches!(ext.as_str(), "part" | "ytdl" | "tmp" | "temp")
        || path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".tmp") || n.contains(".part-"))
}

/// Surveille les dossiers ; la surveillance s'arrête quand la valeur renvoyée est libérée.
pub struct FolderWatch {
    _watcher: RecommendedWatcher,
}

impl FolderWatch {
    pub fn start(backend: Arc<Backend>) -> Option<FolderWatch> {
        let (tx, rx) = crossbeam_channel::unbounded::<()>();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res
                && ev.paths.iter().any(|p| !ignored(p))
                && !matches!(ev.kind, notify::EventKind::Access(_))
            {
                let _ = tx.send(());
            }
        })
        .map_err(|e| log::warn!("Surveillance des dossiers impossible : {e}"))
        .ok()?;
        for root in backend.scan_roots() {
            if let Err(e) = watcher.watch(Path::new(&root), RecursiveMode::Recursive) {
                log::warn!("Dossier non surveillé ({root}) : {e}");
            }
        }
        let weak = Arc::downgrade(&backend);
        drop(backend);
        let spawned = std::thread::Builder::new().name("control-watch".into()).spawn(move || {
            let mut due: Option<Instant> = None;
            loop {
                let wait = due.map(|d| d.saturating_duration_since(Instant::now())).unwrap_or(Duration::from_secs(3600));
                match rx.recv_timeout(wait) {
                    Ok(()) => due = Some(Instant::now() + QUIET),
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        if due.take().is_some() {
                            let Some(b) = weak.upgrade() else { return };
                            let roots = b.scan_roots();
                            b.library.scan(&roots);
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("Surveillance des dossiers impossible : {e}");
        }
        Some(FolderWatch { _watcher: watcher })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_files_are_ignored() {
        assert!(ignored(Path::new("/m/a.mp3.part")));
        assert!(ignored(Path::new("/m/a.ytdl")));
        assert!(ignored(Path::new("/m/cover.jpg.123-4.tmp")));
        assert!(!ignored(Path::new("/m/a.mp3")));
        assert!(!ignored(Path::new("/m/Party.flac")));
    }
}

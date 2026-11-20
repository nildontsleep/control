//! Services du bureau : un seul noyau (réglages, base, bibliothèque…) partagé par les threads de travail.
//!
//! Rien ici ne tourne sur le thread de l'interface : les commandes partent sur un thread (`run`), et leur résultat
//! revient à l'interface par `AppHandle::update`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;

use control_core::covers::Covers;
use control_core::library::{Library, SharedDb};
use control_core::model::Emit;
use control_core::store::Store;

pub struct Backend {
    pub data_dir: PathBuf,
    pub store: Store,
    pub db: SharedDb,
    pub library: Arc<Library>,
    pub covers: Covers,
}

impl Backend {
    /// Ouvre le dossier de données (le crée au besoin). Une base illisible est mise de côté et recréée plutôt que
    /// d'empêcher l'application de démarrer.
    pub fn open(data_dir: &Path, emit: Emit) -> anyhow::Result<Backend> {
        std::fs::create_dir_all(data_dir)?;
        let store = Store::open(data_dir.join("settings.json"));
        store.spawn_writer();
        let db_file = data_dir.join("library.db");
        let conn = match control_core::db::open(&db_file) {
            Ok(c) => c,
            Err(err) => {
                log::error!("Base illisible ({err}) : elle est mise de côté et recréée.");
                let aside = data_dir.join(format!("library.db.broken-{}", control_core::util::now_ms()));
                let _ = std::fs::rename(&db_file, &aside);
                control_core::db::open(&db_file)?
            }
        };
        let db: SharedDb = Arc::new(Mutex::new(conn));
        let covers = Covers::new(data_dir.join("covers"));
        let library = Arc::new(Library::new(db.clone(), covers.clone(), data_dir.join("thumbs"), emit));
        Ok(Backend { data_dir: data_dir.to_path_buf(), store, db, library, covers })
    }

    /// Dossiers à analyser : ceux de la bibliothèque, plus celui des téléchargements s'il n'y est pas déjà.
    pub fn scan_roots(&self) -> Vec<String> {
        let s = self.store.settings();
        let mut roots = s.library_folders.clone();
        let dl = if s.download_dir.is_empty() {
            crate::paths::default_download_dir().to_string_lossy().into_owned()
        } else {
            s.download_dir.clone()
        };
        let lower = dl.to_lowercase();
        if Path::new(&dl).exists() && !roots.iter().any(|r| lower.starts_with(&r.to_lowercase())) {
            roots.push(dl);
        }
        roots
    }

    /// Enregistre ce qui doit l'être avant de quitter.
    pub fn close(&self) {
        self.library.close();
        self.store.flush();
    }
}

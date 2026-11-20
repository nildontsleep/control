//! Services du bureau : un seul noyau (réglages, base, bibliothèque, tags, paroles, téléchargements, films et
//! séries, scrobbling, Discord, normalisation, mises à jour) partagé par les threads de travail.
//!
//! Rien ici ne tourne sur le thread de l'interface : les commandes partent sur un thread (`Ui::spawn`), et leur
//! résultat revient à l'interface par `AppHandle::update`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;

use control_core::automix::AnalysisCache;
use control_core::covers::Covers;
use control_core::discord::{CoverRef, DiscordClient, DiscordOptions};
use control_core::downloads::{Downloads, DownloadsConfig};
use control_core::library::{Library, SharedDb};
use control_core::loudness::Loudness;
use control_core::lyrics::Lyrics;
use control_core::media::MediaService;
use control_core::model::Emit;
use control_core::scrobble::{Scrobbler, ScrobblerOptions};
use control_core::store::Store;
use control_core::tags::Tags;
use control_core::tools::{Tool, Tools};
use control_core::updater::{Edition, Updater, UpdaterOptions};

pub struct Backend {
    pub data_dir: PathBuf,
    pub store: Store,
    pub db: SharedDb,
    pub library: Arc<Library>,
    pub covers: Covers,
    pub tags: Tags,
    pub lyrics: Lyrics,
    pub tools: Arc<Tools>,
    pub downloads: Downloads,
    pub media: MediaService,
    pub scrobbler: Scrobbler,
    pub discord: DiscordClient,
    pub loudness: Loudness,
    pub updater: Updater,
    pub automix: Mutex<AnalysisCache>,
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
        let library = Arc::new(Library::new(db.clone(), covers.clone(), data_dir.join("thumbs"), emit.clone()));
        let tags = Tags::new(library.clone(), emit.clone());
        let lyrics = Lyrics::new(db.clone());
        let tools = Arc::new(Tools::new(data_dir.join("bin"), data_dir.join("tmp"), emit.clone()));
        let downloads = Downloads::new(DownloadsConfig {
            tools: tools.clone(),
            library: library.clone(),
            store: store.clone(),
            file: data_dir.join("downloads.json"),
            emit: emit.clone(),
            cookies_file: None,
            default_dir: crate::paths::default_download_dir(),
        });
        let media = MediaService::new(db.clone(), store.clone(), emit.clone());
        let scrobbler = Scrobbler::new(ScrobblerOptions { dir: data_dir.to_path_buf(), db: db.clone(), emit: emit.clone(), keys: None, guard: None });
        scrobbler.resume();
        let cover_covers = covers.clone();
        let cover_lib = library.clone();
        let cover_tools = tools.clone();
        let discord = DiscordClient::new(DiscordOptions {
            emit: emit.clone(),
            cover_file: Some(Arc::new(move |r: &CoverRef| match r {
                CoverRef::Cover(hash) => cover_covers.path(hash, false).filter(|p| p.exists()),
                CoverRef::VideoThumb(id) => cover_lib.video_thumb(*id, cover_tools.path(Tool::Ffmpeg).as_deref()),
            })),
            cache_file: Some(data_dir.join("discord-covers.json")),
            ipc_dir: None,
        });
        discord.configure_from(&store.settings().discord);
        let ff_tools = tools.clone();
        let loudness = Loudness::new(db.clone(), store.clone(), Arc::new(move || ff_tools.path(Tool::Ffmpeg)), emit.clone());
        let exe = std::env::current_exe().unwrap_or_default();
        let edition = Edition::detect(&exe);
        let self_file = std::env::var_os("APPIMAGE").map(PathBuf::from).or_else(|| (edition == Edition::Portable).then(|| exe.clone()));
        let updater = Updater::new(UpdaterOptions {
            version: env!("CARGO_PKG_VERSION").to_string(),
            edition,
            dir: data_dir.join("updates"),
            emit: emit.clone(),
            self_file,
            token: None,
        });
        let automix = Mutex::new(AnalysisCache::open(data_dir));
        Ok(Backend {
            data_dir: data_dir.to_path_buf(),
            store,
            db,
            library,
            covers,
            tags,
            lyrics,
            tools,
            downloads,
            media,
            scrobbler,
            discord,
            loudness,
            updater,
            automix,
        })
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

    /// Chemin de ffmpeg (installé par l'application, sinon celui du système).
    pub fn ffmpeg(&self) -> Option<PathBuf> {
        self.tools.path(Tool::Ffmpeg)
    }

    /// Enregistre ce qui doit l'être avant de quitter.
    pub fn close(&self) {
        self.library.close();
        self.loudness.close();
        self.scrobbler.close();
        self.downloads.kill_all();
        self.discord.shutdown();
        self.store.flush();
    }
}

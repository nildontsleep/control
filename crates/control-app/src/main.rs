//! control : lecteur musique et vidéo, interface Rune.

mod audio;
mod backend;
mod format;
mod i18n;
mod icons;
mod os;
#[cfg(target_os = "linux")]
mod mpris;
mod paths;
mod player;
mod settings_ui;
mod sheets;
mod state;
mod theme;
mod ui;
mod video;
mod views;
mod watch;
mod window;

use std::sync::Arc;

use rune::{AppSpec, Tx, Waker};

use control_core::model::Event;

use crate::backend::Backend;
use crate::state::Ui;

/// Bibliothèque d'exemple pour les rendus sans fenêtre (`--snap`) quand `CONTROL_DATA` n'est pas défini.
fn placeholder() -> Ui {
    let dir = std::env::temp_dir().join(format!("control-preview-{}", std::process::id()));
    let backend = Backend::open(&dir, control_core::model::emit_nothing()).expect("dossier temporaire");
    Ui::new(Arc::new(backend), None)
}

fn setup(s: &mut Ui, waker: Waker) {
    s.set_images(waker);
    // Le vrai moteur audio, sauf si on le demande muet (essais) ; sans carte son il joue en silence.
    if std::env::var_os("CONTROL_NO_AUDIO").is_none() {
        match audio::EngineTransport::new(s.backend.clone()) {
            Ok(t) => s.player.set_transport(Box::new(t)),
            Err(err) => log::error!("Moteur audio indisponible : {err}"),
        }
    }
    s.reload_library();
    s.rescan();
    // Fichiers passés au lancement (« Ouvrir avec control ») : ajoutés à la bibliothèque, puis joués.
    let files: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .filter(|a| control_core::probe::is_media_file(std::path::Path::new(a)) && std::path::Path::new(a).is_file())
        .collect();
    if !files.is_empty() {
        s.spawn(
            move |b| b.library.add_paths(&files, None),
            |s, ids, _| {
                s.pending_play = ids;
                s.reload_library();
            },
        );
    }
    s.watch_folders();
    // Mises à jour : au lancement puis toutes les 6 heures, si le réglage le permet.
    let store = s.backend.store.clone();
    s.backend.updater.spawn_checks(move || store.settings().update.auto);
    s.backend.updater.cleanup();
    // Outils et téléchargements : état des outils, liste des téléchargements, mise à jour quotidienne de yt-dlp.
    views::downloads::on_open(s);
    #[cfg(target_os = "linux")]
    if let Some(h) = s.handle.clone() {
        s.mpris = mpris::Mpris::start(h);
    }
}

fn poll(s: &mut Ui, clock: f64, tx: &mut Tx) -> bool {
    s.clock = clock;
    let mut changed = s.tick(clock, tx);
    let before = s.toasts.len();
    s.toasts.retain(|t| t.until > clock);
    changed |= s.toasts.len() != before;
    if let Some(next) = s.toasts.iter().map(|t| t.until).reduce(f64::min) {
        tx.wake_after(std::time::Duration::from_secs_f64((next - clock).max(0.05)));
    }
    changed
}

/// Une scène : son nom et ce qu'elle change à l'état de départ.
type Scene = (&'static str, fn(&mut Ui));

/// Scènes des rendus sans fenêtre (`--scene`) : chaque vue principale.
const SCENES: &[Scene] = &[
    ("songs", |s| s.route = state::Route::Songs),
    ("albums", |s| s.route = state::Route::Albums),
    ("artists", |s| s.route = state::Route::Artists),
    ("videos", |s| s.route = state::Route::Videos),
    ("stats", |s| s.route = state::Route::Stats),
    ("downloads", |s| s.navigate(state::Route::Downloads)),
    ("downloads-queue", |s| views::downloads::scene(s, "queue")),
    ("downloads-playlist", |s| views::downloads::scene(s, "playlist")),
    ("downloads-search", |s| views::downloads::scene(s, "search")),
    ("downloads-install", |s| views::downloads::scene(s, "install")),
    ("downloads-missing", |s| views::downloads::scene(s, "missing")),
    ("settings-downloads", |s| s.sheet = Some(state::Sheet::Settings(state::SettingsTab::Downloads))),
    ("settings-cookies", |s| {
        s.sheet = Some(state::Sheet::Settings(state::SettingsTab::Downloads));
        s.downloads.popup = Some("set-cookies");
    }),
    ("stats-7d", |s| {
        s.route = state::Route::Stats;
        s.settings.stats_range = "7d".into();
    }),
    ("stats-year", |s| {
        s.route = state::Route::Stats;
        s.settings.stats_range = "year".into();
    }),
    ("recap", |s| views::recap::scene(s, false, 0)),
    ("recap-time", |s| views::recap::scene(s, false, 1)),
    ("recap-artist", |s| views::recap::scene(s, false, 2)),
    ("recap-tracks", |s| views::recap::scene(s, false, 3)),
    ("recap-sound", |s| views::recap::scene(s, false, 4)),
    ("recap-habits", |s| views::recap::scene(s, false, 5)),
    ("recap-summary", |s| views::recap::scene(s, false, 99)),
    ("recap-offer", |s| views::recap::scene(s, true, 0)),
    ("downloads", |s| s.route = state::Route::Downloads),
    ("settings", |s| s.sheet = Some(state::Sheet::Settings(state::SettingsTab::General))),
    ("appearance", |s| s.sheet = Some(state::Sheet::Settings(state::SettingsTab::Appearance))),
    ("equalizer", |s| s.sheet = Some(state::Sheet::Equalizer)),
    ("light", |s| s.settings.theme = "light".into()),
    ("nowplaying", |s| s.now_playing = true),
    ("lyrics", |s| {
        s.now_playing = true;
        s.settings.np_tab = "lyrics".into();
    }),
    ("immersive", |s| s.immersive = true),
    ("tags", |s| tag_scene(s, 1, None)),
    ("tags-many", |s| tag_scene(s, 4, None)),
    ("tags-rename", |s| tag_scene(s, 4, Some(sheets::tag_editor::Tool::Rename))),
    ("tags-number", |s| tag_scene(s, 4, Some(sheets::tag_editor::Tool::Number))),
    ("smart", |s| smart_scene(s, sheets::smart::SmartScene::New)),
    ("smart-edit", |s| smart_scene(s, sheets::smart::SmartScene::EditFirst)),
    ("pl-edit", |s| smart_scene(s, sheets::smart::SmartScene::CustomizeFirst)),
];

/// Scène : l'éditeur de tags s'ouvre sur les premiers morceaux dès que la bibliothèque est chargée.
fn tag_scene(s: &mut Ui, count: usize, tool: Option<sheets::tag_editor::Tool>) {
    s.tag_editor.scene.set(Some(sheets::tag_editor::SceneOpen { count, tool }));
    s.sheet = Some(state::Sheet::TagEditor);
}

/// Scène : éditeur de playlist intelligente ou personnalisation, une fois la bibliothèque chargée.
fn smart_scene(s: &mut Ui, scene: sheets::smart::SmartScene) {
    s.smart.scene.set(Some(scene));
    s.sheet = Some(match scene {
        sheets::smart::SmartScene::CustomizeFirst => state::Sheet::PlaylistEdit,
        _ => state::Sheet::Smart,
    });
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let data_dir = paths::data_dir();
    let spec = AppSpec {
        min_size: Some((920.0, 600.0)),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(ui::on_key),
        scenes: SCENES,
        ..AppSpec::new("control", (1280.0, 820.0), placeholder, ui::view)
    };
    let app = rune::App::new(spec);
    let handle = app.handle();
    let emit_handle = handle.clone();
    let emit: control_core::model::Emit = Arc::new(move |ev: Event| {
        emit_handle.update(move |s: &mut Ui, _| s.on_event(ev));
    });
    let backend = match Backend::open(&data_dir, emit) {
        Ok(b) => Arc::new(b),
        Err(err) => {
            eprintln!("control : impossible d’ouvrir le dossier de données {} : {err}", data_dir.display());
            std::process::exit(1);
        }
    };
    let close_backend = backend.clone();
    let (options, maximized) = window::restore(&backend.store);
    app.window_options(options)
        .maximized(maximized)
        .on_window(window::remember)
        .init(move || Ui::new(backend, Some(handle)))
        .on_close(|s: &mut Ui, _tx| {
            window::save(&s.backend.store);
            s.player.save_session(&s.backend);
            s.backend.store.flush();
        })
        .run();
    close_backend.close();
}

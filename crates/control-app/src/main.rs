//! control : lecteur musique et vidéo, interface Rune.

mod backend;
mod format;
mod i18n;
mod icons;
mod os;
mod paths;
mod player;
mod settings_ui;
mod sheets;
mod state;
mod theme;
mod ui;
mod views;

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
    s.reload_library();
    s.rescan();
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

/// Scènes des rendus sans fenêtre (`--scene`) : chaque vue principale.
const SCENES: &[(&str, fn(&mut Ui))] = &[
    ("songs", |s| s.route = state::Route::Songs),
    ("albums", |s| s.route = state::Route::Albums),
    ("artists", |s| s.route = state::Route::Artists),
    ("videos", |s| s.route = state::Route::Videos),
    ("stats", |s| s.route = state::Route::Stats),
    ("downloads", |s| s.route = state::Route::Downloads),
    ("settings", |s| s.sheet = Some(state::Sheet::Settings(state::SettingsTab::General))),
    ("appearance", |s| s.sheet = Some(state::Sheet::Settings(state::SettingsTab::Appearance))),
    ("equalizer", |s| s.sheet = Some(state::Sheet::Equalizer)),
    ("light", |s| s.settings.theme = "light".into()),
];

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
    app.init(move || Ui::new(backend, Some(handle)))
        .on_close(|s: &mut Ui, _tx| {
            s.player.save_session(&s.backend);
            s.backend.store.flush();
        })
        .run();
    close_backend.close();
}

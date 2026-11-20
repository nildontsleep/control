//! Place de la fenêtre : relue au lancement, enregistrée à la fermeture (`window.main` du fichier de réglages, au
//! format de la version Electron : `{ bounds: { x, y, width, height }, maximized }` en points, plus `scale`).

use std::cell::RefCell;

use rune::{WindowHandle, WindowOptions};
use serde_json::{Value, json};

use control_core::store::Store;

thread_local! {
    static MAIN: RefCell<Option<WindowHandle>> = const { RefCell::new(None) };
}

/// Retient la fenêtre principale (appelé à sa création, sur le thread de l'interface).
pub fn remember(handle: &WindowHandle) {
    MAIN.with(|m| *m.borrow_mut() = Some(handle.clone()));
}

/// Options d'ouverture d'après la place enregistrée, et si la fenêtre était agrandie.
pub fn restore(store: &Store) -> (WindowOptions, bool) {
    let saved = store.window_state("main");
    let mut options = WindowOptions { min_size: Some((920.0, 600.0)), ..WindowOptions::default() };
    let maximized = saved.get("maximized").and_then(Value::as_bool).unwrap_or(false);
    let b = &saved["bounds"];
    let num = |k: &str| b.get(k).and_then(Value::as_f64);
    if let (Some(x), Some(y), Some(w), Some(h)) = (num("x"), num("y"), num("width"), num("height"))
        && w >= 400.0
        && h >= 300.0
    {
        let scale = saved.get("scale").and_then(Value::as_f64).filter(|s| *s > 0.0).unwrap_or(1.0);
        options.position = Some(((x * scale).round() as i32, (y * scale).round() as i32));
        options.physical_size = Some(((w * scale).round() as u32, (h * scale).round() as u32));
    }
    (options, maximized)
}

/// Enregistre la place de la fenêtre (à la fermeture). Rien sous Wayland, qui ne donne pas la position.
pub fn save(store: &Store) {
    let placement = MAIN.with(|m| m.borrow().as_ref().and_then(WindowHandle::placement));
    let Some(p) = placement else { return };
    if p.minimized || p.full_screen {
        return;
    }
    let (frame_pos, size) = match (p.maximized, p.restore) {
        (true, Some((pos, size))) => (pos, size),
        _ => (p.position, p.content_size),
    };
    let s = p.scale.max(0.1);
    store.set_window_state(
        "main",
        json!({
            "bounds": { "x": (frame_pos.0 as f64 / s).round(), "y": (frame_pos.1 as f64 / s).round(),
                        "width": (size.0 as f64 / s).round(), "height": (size.1 as f64 / s).round() },
            "maximized": p.maximized,
            "scale": s,
        }),
    );
}

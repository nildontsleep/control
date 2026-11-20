//! Films, séries et animés (à porter depuis `src/renderer/js/views/{watch,title,watching,mylist,shelf}.js`).

use rune::{Cx, El};
use serde_json::Value;

use crate::state::Ui;
use crate::theme::Palette;

/// Pages de la partie « Regarder ».
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WatchRoute {
    /// Accueil films et séries.
    Movies,
    /// Accueil animés.
    Anime,
    /// Ma liste.
    MyList,
}

#[derive(Default)]
pub struct WatchState {}

pub fn on_open(_ui: &mut Ui, _route: &WatchRoute) {}

/// Événements `watch:*` du noyau.
pub fn on_event(_ui: &mut Ui, _channel: &str, _payload: &Value) {}

pub fn view(_ui: &Ui, p: &Palette, _cx: &mut Cx<Ui>, _route: &WatchRoute) -> El {
    crate::views::not_found(p)
}

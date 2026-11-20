//! Statistiques d'écoute (à porter depuis `src/renderer/js/views/stats.js`).

use rune::{Cx, El};

use crate::state::Ui;
use crate::theme::Palette;

/// État de la vue des statistiques.
#[derive(Default)]
pub struct StatsState {}

/// La vue s'ouvre : charger les statistiques.
pub fn on_open(_ui: &mut Ui) {}

pub fn view(_ui: &Ui, p: &Palette, _cx: &mut Cx<Ui>) -> El {
    crate::views::not_found(p)
}

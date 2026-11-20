//! Téléchargements yt-dlp (à porter depuis `src/renderer/js/views/downloads.js`).

use rune::{Cx, El};
use serde_json::Value;

use crate::state::Ui;
use crate::theme::Palette;

#[derive(Default)]
pub struct DownloadsState {}

impl DownloadsState {
    /// Téléchargements en cours (pastille de la barre latérale).
    pub fn active(&self) -> usize {
        0
    }
}

pub fn on_open(_ui: &mut Ui) {}

/// Événements `dl:*` et `tools:*` du noyau.
pub fn on_event(_ui: &mut Ui, _channel: &str, _payload: &Value) {}

pub fn view(_ui: &Ui, p: &Palette, _cx: &mut Cx<Ui>) -> El {
    crate::views::not_found(p)
}

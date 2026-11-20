//! Playlists intelligentes et personnalisation des playlists (à porter depuis `src/renderer/js/ui/smart.js` et
//! `ui/playlist-edit.js`).

use rune::El;
use rune_kit::Kit;

use control_core::model::PlaylistId;

use crate::state::Ui;
use crate::theme::Palette;

#[derive(Default)]
pub struct SmartState {}

/// Ouvre l'éditeur de règles (nouvelle playlist si `None`).
pub fn open_smart(_ui: &mut Ui, _playlist: Option<PlaylistId>) {}

/// Ouvre la personnalisation (nom, description, pochette).
pub fn open_customize(_ui: &mut Ui, _playlist: PlaylistId) {}

pub fn sheet(_ui: &Ui, _p: &Palette, _k: &Kit, _layer: &mut Vec<El>) {}

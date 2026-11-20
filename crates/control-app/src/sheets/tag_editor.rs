//! Éditeur de tags (à porter depuis `src/renderer/js/ui/tag-editor.js`).

use rune::El;
use rune_kit::Kit;

use control_core::model::TrackId;

use crate::state::Ui;
use crate::theme::Palette;

#[derive(Default)]
pub struct TagEditorState {}

/// Ouvre l'éditeur pour ces morceaux.
pub fn open(_ui: &mut Ui, _ids: Vec<TrackId>) {}

pub fn sheet(_ui: &Ui, _p: &Palette, _k: &Kit, _layer: &mut Vec<El>) {}

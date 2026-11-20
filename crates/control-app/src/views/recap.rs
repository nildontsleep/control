//! Récap façon « Wrapped » (à porter depuis `src/renderer/js/ui/wrapped.js` et `stats/recap-prompt.js`).

use rune::El;
use rune_kit::Kit;

use crate::state::Ui;
use crate::theme::Palette;

/// État du récap ouvert.
#[derive(Default)]
pub struct RecapState {}

/// Feuille du récap (`Sheet::Recap`).
pub fn sheet(_ui: &Ui, _p: &Palette, _k: &Kit, _layer: &mut Vec<El>) {}

/// Au démarrage : proposer le récap du mois ou de la semaine écoulés.
pub fn maybe_prompt(_ui: &mut Ui) {}

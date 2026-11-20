//! AutoMix : analyse des morceaux (tempo, grille de battements, début et fin audibles) et plan des mixages calés.
//!
//! L'original analysait dans l'interface (décodage par Web Audio, calcul en arrière-plan, cache dans le
//! `localStorage`). Ici : décodage par symphonia, mixage en mono et rééchantillonnage à [`ANALYSIS_RATE`], puis
//! l'algorithme d'origine ([`analyze_mono`]), et un cache dans un fichier JSON ([`AnalysisCache`]).
//! Aucune lecture n'est faite ici : le moteur audio consomme [`Analysis`] et [`plan_mix`].

mod analysis;
mod cache;
mod plan;
mod resample;

use std::borrow::Cow;
use std::path::Path;

use anyhow::{Result, bail};

pub use analysis::{ANALYSIS_RATE, Analysis, MAX_BPM, MIN_BPM, analyze_mono};
pub use cache::{AnalysisCache, CACHE_FILE, CACHE_VERSION, signature};
pub use plan::{MixOptions, MixPlan, plan_mix, usable};
pub use resample::{Resampler, resample};

use crate::loudness::decode::decode_file;

/// Au-delà (mix de DJ, livres audio), pas la peine.
pub const MAX_DURATION: f64 = 20.0 * 60.0;
/// Fichiers plus gros : pas d'analyse.
pub const MAX_BYTES: u64 = 160_000_000;

/// Analyse d'un signal mono à `rate` Hz (rééchantillonné à [`ANALYSIS_RATE`] au besoin).
pub fn analyze_samples(mono: &[f32], rate: u32) -> Analysis {
    let mono =
        if rate == ANALYSIS_RATE { Cow::Borrowed(mono) } else { Cow::Owned(resample(mono, rate, ANALYSIS_RATE)) };
    analyze_mono(&mono)
}

/// Décode un fichier en mono à [`ANALYSIS_RATE`] (moyenne des voies). Échoue pour un fichier de plus de
/// [`MAX_BYTES`] octets ou de [`MAX_DURATION`] secondes, ou illisible.
pub fn decode_mono(path: &Path) -> Result<Vec<f32>> {
    let size = std::fs::metadata(path)?.len();
    if size > MAX_BYTES {
        bail!("fichier trop gros");
    }
    let mut mono = Vec::new();
    let mut resampler: Option<Resampler> = None;
    // Une seconde de plus que la limite : de quoi savoir qu'elle est dépassée sans tout décoder.
    let (format, frames) = decode_file(path, Some(MAX_DURATION + 1.0), |format, samples| {
        let r = resampler.get_or_insert_with(|| Resampler::new(format.rate, ANALYSIS_RATE));
        let scale = 1.0 / format.channels as f32;
        for frame in samples.chunks_exact(format.channels) {
            r.push(frame.iter().sum::<f32>() * scale, &mut mono);
        }
        true
    })?;
    if frames as f64 / format.rate as f64 > MAX_DURATION {
        bail!("morceau trop long");
    }
    Ok(mono)
}

/// Analyse d'un fichier (bloquant). Voir [`AnalysisCache::analyze`] pour la version avec cache.
pub fn analyze_file(path: &Path) -> Result<Analysis> {
    Ok(analyze_mono(&decode_mono(path)?))
}

#[cfg(test)]
mod tests;

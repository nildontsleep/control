//! Plan d'un mixage calé entre deux morceaux analysés (`planMix` d'origine), utilisé par le lecteur.

use serde::{Deserialize, Serialize};

use super::analysis::{Analysis, MAX_BPM, MIN_BPM, round_to};

/// Ce que l'on retient d'une analyse pour décider d'un mixage calé : tempo fiable et morceau assez long.
pub fn usable(a: &Analysis) -> bool {
    a.conf >= 1.35 && a.dur >= 40.0 && a.bpm >= MIN_BPM && a.bpm <= MAX_BPM
}

/// Réglages d'un plan de mixage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MixOptions {
    /// Vitesse de lecture du morceau en cours.
    pub rate: f64,
    /// Écart de tempo maximal rattrapé en accélérant ou ralentissant le suivant (0,08 : 8 %).
    pub max_stretch: f64,
    /// Instant (dans le précédent) avant lequel le mixage ne peut pas commencer, par exemple quand on arrive dans la
    /// fin du morceau par un saut ; le mixage est alors plus court.
    pub from: f64,
}

impl Default for MixOptions {
    fn default() -> Self {
        MixOptions { rate: 1.0, max_stretch: 0.08, from: 0.0 }
    }
}

/// Plan de transition (noms JSON d'origine). Instants en secondes dans chaque fichier.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MixPlan {
    /// Début du mixage dans le précédent (sur un temps de sa grille).
    pub start_out: f64,
    /// Entrée du suivant (premier temps de sa grille après son début audible).
    pub start_in: f64,
    /// Durée du mixage (8 ou 16 temps, entre 5 et 12 s).
    pub length: f64,
    /// Durée d'un temps du précédent.
    pub beat_out: f64,
    /// Durée d'un temps du suivant (tempo ramené au double ou à la moitié si c'est plus proche).
    pub beat_in: f64,
    /// Vitesse du suivant par rapport à sa vitesse normale : celle qui cale son tempo sur le précédent.
    pub ratio: f64,
    /// Vitesse du suivant pendant le mixage, par rapport à la vitesse normale (le précédent joue à `rate`).
    pub rate_in: f64,
    pub grid_out: f64,
    pub grid_in: f64,
}

fn clamp(v: f64, a: f64, b: f64) -> f64 {
    b.min(a.max(v))
}

/// Plan de transition entre deux morceaux analysés : `None` si un mixage calé n'est pas possible (alors on fait un
/// fondu enchaîné simple).
pub fn plan_mix(out: &Analysis, inc: &Analysis, opts: MixOptions) -> Option<MixPlan> {
    if !usable(out) || !usable(inc) {
        return None;
    }
    // Tempo du suivant ramené à celui du précédent (un morceau « en double » ou « en moitié » compte comme proche).
    let mut fold = 1.0;
    for k in [2.0, 0.5] {
        if ((inc.bpm * k) / out.bpm - 1.0).abs() < ((inc.bpm * fold) / out.bpm - 1.0).abs() {
            fold = k;
        }
    }
    let inc_bpm = inc.bpm * fold;
    let ratio = out.bpm / inc_bpm;
    if (ratio - 1.0).abs() > opts.max_stretch {
        return None;
    }
    let beat_out = 60.0 / out.bpm;
    let beat_in = 60.0 / inc_bpm;
    // Durée du mixage : 16 temps, ou 8 si c'est trop long ; entre 5 et 12 s.
    let mut beats = 16.0;
    if beat_out * beats > 12.0 {
        beats = 8.0;
    }
    let mut length = clamp(beat_out * beats, 5.0, 12.0);
    // Début du mixage dans le précédent : le dernier temps de sa grille qui laisse la place d'un mixage avant la fin
    // audible.
    let latest = out.end - length;
    let mut k = ((latest - out.grid_end) / beat_out).floor();
    let mut start_out = out.grid_end + k * beat_out;
    if start_out < opts.from {
        // Trop tard pour le mixage prévu : premier temps possible, et un mixage plus court qui tient avant la fin.
        k = ((opts.from - out.grid_end) / beat_out).ceil();
        start_out = out.grid_end + k * beat_out;
        let fit = ((out.end - start_out) / beat_out).floor();
        if fit < 4.0 {
            return None;
        }
        length = length.min(fit * beat_out);
    }
    if start_out < 5.0 {
        return None;
    }
    // Entrée du suivant : le premier temps de sa grille à partir du début audible.
    let j = ((inc.start - 0.03 - inc.grid_start) / beat_in).ceil();
    let start_in = inc.grid_start + j * beat_in;
    Some(MixPlan {
        start_out: round_to(start_out, 3),
        start_in: round_to(start_in, 3),
        length: round_to(length, 3),
        beat_out: round_to(beat_out, 4),
        beat_in: round_to(beat_in, 4),
        ratio: round_to(ratio, 4),
        rate_in: round_to(ratio * opts.rate, 4),
        grid_out: out.grid_end,
        grid_in: inc.grid_start,
    })
}

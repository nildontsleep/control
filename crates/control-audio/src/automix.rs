//! AutoMix : plan d'une transition calée sur le tempo (`planMix` de `automix/analysis.js`).
//!
//! L'analyse des morceaux (tempo, grille de battements, début et fin audibles) est faite ailleurs
//! (`control-core::automix`) ; le moteur n'en reçoit que le résultat, sous forme de [`BeatInfo`].

/// Tempo minimal et maximal reconnus par l'analyse.
pub const MIN_BPM: f64 = 70.0;
pub const MAX_BPM: f64 = 180.0;
/// Confiance minimale dans le tempo mesuré pour oser un mixage calé.
pub const MIN_CONFIDENCE: f64 = 1.35;
/// Durée minimale d'un morceau mixable (s).
pub const MIN_DURATION: f64 = 40.0;
/// Écart de tempo maximal rattrapé en accélérant ou ralentissant le morceau suivant (8 %).
pub const MAX_STRETCH: f64 = 0.08;
/// Fondu simple quand le mixage calé n'est pas possible (s).
pub const FALLBACK_FADE: f64 = 6.0;
/// Le morceau entrant tourne, muet, ce temps (s) avant le début du mixage, le temps de se caler.
pub const MIX_LEAD: f64 = 1.2;
/// Le mixage se prépare quand son début est à moins de ce temps (s, temps réel).
pub const PREPARE_AHEAD: f64 = 6.0;
/// Atténuation des graves d'un morceau pendant l'échange des basses (dB).
pub const BASS_CUT_DB: f64 = -30.0;
/// Fréquence du plateau grave de l'échange des basses (Hz).
pub const BASS_FREQ: f64 = 200.0;

/// Résultat de l'analyse d'un morceau, dans ses propres secondes (vitesse 1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeatInfo {
    /// Durée du morceau (s).
    pub duration: f64,
    /// Tempo (battements par minute).
    pub bpm: f64,
    /// Confiance dans le tempo (rapport du meilleur score à la moyenne ; fiable à partir de 1,35).
    pub confidence: f64,
    /// Début et fin audibles, hors silences de tête et de queue (s).
    pub audible_start: f64,
    pub audible_end: f64,
    /// Un temps de la grille près du début (`gridStart`) et un près de la fin (`gridEnd`) (s).
    pub grid_start: f64,
    pub grid_end: f64,
}

impl BeatInfo {
    /// Analyse assez fiable pour un mixage calé (`usable`).
    pub fn usable(&self) -> bool {
        self.confidence >= MIN_CONFIDENCE && self.duration >= MIN_DURATION && (MIN_BPM..=MAX_BPM).contains(&self.bpm)
    }
}

/// Plan d'un mixage : où il commence dans chacun des deux morceaux, combien de temps il dure, à quelle vitesse joue
/// le morceau entrant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MixPlan {
    /// Début du mixage dans le morceau sortant (s, sur un temps de sa grille).
    pub start_out: f64,
    /// Point du morceau entrant calé sur ce début (s, sur un temps de sa grille).
    pub start_in: f64,
    /// Durée du mixage, en secondes du morceau sortant.
    pub length: f64,
    /// Durée d'un temps dans chacun des morceaux (le tempo de l'entrant ramené au double ou à la moitié si besoin).
    pub beat_out: f64,
    pub beat_in: f64,
    /// Vitesse de l'entrant par rapport à sa vitesse normale : celle qui cale son tempo sur le sortant.
    pub ratio: f64,
    /// Vitesse absolue de l'entrant pendant le mixage (le sortant joue à la vitesse générale).
    pub rate_in: f64,
}

impl MixPlan {
    /// Durée du mixage en temps du morceau sortant.
    pub fn beats(&self) -> f64 {
        self.length / self.beat_out
    }
}

/// Options du plan.
#[derive(Debug, Clone, Copy)]
pub struct PlanOptions {
    /// Vitesse générale de lecture.
    pub rate: f64,
    /// Écart de tempo maximal accepté.
    pub max_stretch: f64,
    /// Instant (dans le sortant) avant lequel le mixage ne peut pas commencer (arrivée tardive par un saut).
    pub from: f64,
}

impl Default for PlanOptions {
    fn default() -> Self {
        PlanOptions { rate: 1.0, max_stretch: MAX_STRETCH, from: 0.0 }
    }
}

/// Plan de transition entre deux morceaux analysés ; `None` si un mixage calé n'est pas possible (on fait alors un
/// fondu enchaîné simple de [`FALLBACK_FADE`] secondes calé sur la fin audible).
pub fn plan_mix(out: &BeatInfo, inc: &BeatInfo, opts: PlanOptions) -> Option<MixPlan> {
    if !out.usable() || !inc.usable() {
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
    // Durée : 16 temps, ou 8 si c'est trop long ; entre 5 et 12 s.
    let mut beats = 16.0;
    if beat_out * beats > 12.0 {
        beats = 8.0;
    }
    let mut length = (beat_out * beats).clamp(5.0, 12.0);
    // Début dans le précédent : le dernier temps de sa grille qui laisse la place du mixage avant la fin audible.
    let latest = out.audible_end - length;
    let mut k = ((latest - out.grid_end) / beat_out).floor();
    let mut start_out = out.grid_end + k * beat_out;
    if start_out < opts.from {
        // Trop tard pour le mixage prévu : premier temps possible, et un mixage plus court qui tient avant la fin.
        k = ((opts.from - out.grid_end) / beat_out).ceil();
        start_out = out.grid_end + k * beat_out;
        let fit = ((out.audible_end - start_out) / beat_out).floor();
        if fit < 4.0 {
            return None;
        }
        length = length.min(fit * beat_out);
    }
    if start_out < 5.0 {
        return None;
    }
    // Entrée du suivant : le premier temps de sa grille à partir du début audible.
    let j = ((inc.audible_start - 0.03 - inc.grid_start) / beat_in).ceil();
    let start_in = inc.grid_start + j * beat_in;
    Some(MixPlan { start_out, start_in, length, beat_out, beat_in, ratio, rate_in: ratio * opts.rate })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn info(bpm: f64) -> BeatInfo {
        BeatInfo {
            duration: 200.0,
            bpm,
            confidence: 2.0,
            audible_start: 0.4,
            audible_end: 196.0,
            grid_start: 0.25,
            grid_end: 190.1,
        }
    }

    fn on_grid(t: f64, anchor: f64, beat: f64) -> bool {
        let k = (t - anchor) / beat;
        (k - k.round()).abs() < 1e-6
    }

    #[test]
    fn mix_enters_on_beats_with_8_or_16_beats() {
        for bpm in [70.0, 79.0, 90.0, 120.0, 128.0, 150.0, 174.0, 180.0] {
            let out = info(bpm);
            let inc_bpm = if bpm * 1.03 <= MAX_BPM { bpm * 1.03 } else { bpm / 1.03 };
            let inc = BeatInfo { bpm: inc_bpm, grid_start: 0.61, audible_start: 0.5, ..info(bpm) };
            let p = plan_mix(&out, &inc, PlanOptions::default()).unwrap();
            let beats = p.beats();
            assert!((beats - 16.0).abs() < 1e-9 || (beats - 8.0).abs() < 1e-9, "{bpm} BPM : {beats} temps");
            assert!(p.length >= 5.0 && p.length <= 12.0);
            assert!(on_grid(p.start_out, out.grid_end, p.beat_out), "début du sortant hors grille");
            assert!(on_grid(p.start_in, inc.grid_start, p.beat_in), "entrée hors grille");
            assert!(p.start_in >= inc.audible_start - 0.03 && p.start_in < inc.audible_start - 0.03 + p.beat_in);
            assert!(p.start_out + p.length <= out.audible_end + 1e-9);
            assert!(out.audible_end - (p.start_out + p.length) < p.beat_out, "le mixage finit au plus près de la fin");
            assert!((p.ratio - bpm / inc_bpm).abs() < 1e-9);
        }
        // Lent : 16 temps dépasseraient 12 s, on en garde 8.
        let p = plan_mix(&info(75.0), &info(75.0), PlanOptions::default()).unwrap();
        assert!((p.beats() - 8.0).abs() < 1e-9);
        let p = plan_mix(&info(128.0), &info(128.0), PlanOptions::default()).unwrap();
        assert!((p.beats() - 16.0).abs() < 1e-9);
    }

    #[test]
    fn tempo_ratio_is_clamped_with_half_and_double_folding() {
        let out = info(120.0);
        // 8 % : accepté, 9 % : refusé.
        assert!(plan_mix(&out, &info(120.0 / 1.079), PlanOptions::default()).is_some());
        assert!(plan_mix(&out, &info(120.0 / 1.09), PlanOptions::default()).is_none());
        assert!(plan_mix(&out, &info(120.0 * 1.09), PlanOptions::default()).is_none());
        // Double : 90 BPM se replie en 180 contre 174.
        let p = plan_mix(&info(174.0), &info(90.0), PlanOptions::default()).unwrap();
        assert!((p.beat_in - 60.0 / 180.0).abs() < 1e-9);
        assert!((p.ratio - 174.0 / 180.0).abs() < 1e-9);
        // Moitié : 160 BPM se replie en 80 contre 82.
        let p = plan_mix(&info(82.0), &info(160.0), PlanOptions::default()).unwrap();
        assert!((p.ratio - 82.0 / 80.0).abs() < 1e-9);
        // La vitesse générale s'applique à l'entrant.
        let p = plan_mix(&out, &info(118.0), PlanOptions { rate: 1.25, ..Default::default() }).unwrap();
        assert!((p.rate_in - p.ratio * 1.25).abs() < 1e-12);
    }

    #[test]
    fn falls_back_when_unusable_or_too_far() {
        let out = info(120.0);
        // Tempos trop éloignés (même repliés).
        assert!(plan_mix(&out, &info(100.0), PlanOptions::default()).is_none());
        assert!(plan_mix(&out, &info(140.0), PlanOptions::default()).is_none());
        // Confiance insuffisante, morceau trop court, tempo hors plage.
        assert!(plan_mix(&out, &BeatInfo { confidence: 1.34, ..info(120.0) }, PlanOptions::default()).is_none());
        assert!(plan_mix(&BeatInfo { duration: 39.0, ..out }, &info(120.0), PlanOptions::default()).is_none());
        assert!(plan_mix(&out, &BeatInfo { bpm: 69.0, ..info(120.0) }, PlanOptions::default()).is_none());
        assert!(plan_mix(&out, &BeatInfo { bpm: 181.0, ..info(120.0) }, PlanOptions::default()).is_none());
        // Pas de place avant 5 s.
        let short = BeatInfo { duration: 40.0, audible_end: 9.0, grid_end: 4.0, ..out };
        assert!(plan_mix(&short, &info(120.0), PlanOptions::default()).is_none());
    }

    #[test]
    fn late_arrival_shortens_the_mix() {
        let out = info(120.0); // temps de 0,5 s, mixage de 8 s
        let normal = plan_mix(&out, &info(120.0), PlanOptions::default()).unwrap();
        // On arrive 3 s après le début prévu : premier temps possible, et un mixage qui tient avant la fin.
        let from = normal.start_out + 3.0;
        let p = plan_mix(&out, &info(120.0), PlanOptions { from, ..Default::default() }).unwrap();
        assert!(p.start_out >= from && p.start_out < from + p.beat_out);
        assert!(on_grid(p.start_out, out.grid_end, p.beat_out));
        assert!(p.length < normal.length && p.start_out + p.length <= out.audible_end + 1e-9);
        assert!(on_grid(p.length, 0.0, p.beat_out));
        // Moins de 4 temps avant la fin : impossible.
        let p = plan_mix(&out, &info(120.0), PlanOptions { from: out.audible_end - 1.5, ..Default::default() });
        assert!(p.is_none());
    }
}

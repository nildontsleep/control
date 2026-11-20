//! Égaliseur 10 bandes : mêmes fréquences, mêmes filtres, même Q, mêmes préréglages et même compensation de gain
//! que le graphe Web Audio de `player.js`.

use super::biquad::{Biquad, Coeffs, FilterKind};

/// Fréquences centrales des bandes (Hz).
pub const EQ_BANDS: [f64; 10] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
/// Q des bandes en cloche (les plateaux l'ignorent, comme Web Audio).
pub const EQ_Q: f64 = 1.1;

/// Préréglages, dans l'ordre de l'interface d'origine.
pub const EQ_PRESETS: [(&str, [f32; 10]); 10] = [
    ("flat", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("bass", [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("treble", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 4.0, 5.0, 6.0]),
    ("vocal", [-2.0, -2.0, -1.0, 1.0, 3.0, 3.0, 2.0, 0.0, -1.0, -2.0]),
    ("rock", [4.0, 3.0, 2.0, -1.0, -2.0, -1.0, 2.0, 3.0, 4.0, 4.0]),
    ("electronic", [5.0, 4.0, 1.0, 0.0, -2.0, 2.0, 1.0, 2.0, 4.0, 5.0]),
    ("classical", [3.0, 2.0, 1.0, 1.0, -1.0, -1.0, 0.0, 2.0, 3.0, 4.0]),
    ("acoustic", [4.0, 3.0, 2.0, 1.0, 1.0, 1.0, 2.0, 3.0, 3.0, 2.0]),
    ("jazz", [3.0, 2.0, 1.0, 2.0, -2.0, -2.0, 0.0, 1.0, 2.0, 3.0]),
    ("loudness", [6.0, 4.0, 0.0, 0.0, -1.0, 0.0, -1.0, 0.0, 4.0, 2.0]),
];

/// Gains d'un préréglage par son nom.
pub fn preset(name: &str) -> Option<[f32; 10]> {
    EQ_PRESETS.iter().find(|(n, _)| *n == name).map(|(_, g)| *g)
}

/// Type de filtre de la bande `i` : plateau grave pour la première, plateau aigu pour la dernière, cloche sinon.
pub fn band_kind(i: usize) -> FilterKind {
    match i {
        0 => FilterKind::LowShelf,
        9 => FilterKind::HighShelf,
        _ => FilterKind::Peaking,
    }
}

/// Préampli (dB) : compense 60 % du plus fort gain positif pour éviter la saturation (`applyEq`).
pub fn preamp_db(enabled: bool, gains: &[f32; 10]) -> f32 {
    if !enabled {
        return 0.0;
    }
    let peak = gains.iter().copied().filter(|g| g.is_finite()).fold(0.0f32, f32::max);
    -peak * 0.6
}

/// Taille des sous-blocs entre deux mises à jour des coefficients pendant un changement de réglage.
const SUB_BLOCK: usize = 32;
/// Constante de temps du lissage des gains (s).
const SMOOTH_SECS: f64 = 0.03;

pub struct Equalizer {
    sample_rate: f64,
    filters: [Biquad; 10],
    current: [f64; 10],
    target: [f64; 10],
    preamp: f64,
    preamp_target: f64,
    k: f64,
    active: bool,
}

impl Equalizer {
    pub fn new(sample_rate: u32) -> Equalizer {
        let sr = sample_rate as f64;
        Equalizer {
            sample_rate: sr,
            filters: [Biquad::new(Coeffs::IDENTITY); 10],
            current: [0.0; 10],
            target: [0.0; 10],
            preamp: 0.0,
            preamp_target: 0.0,
            k: 1.0 - (-(SUB_BLOCK as f64) / (SMOOTH_SECS * sr)).exp(),
            active: false,
        }
    }

    /// Nouveaux réglages ; les gains y glissent en ~30 ms (pas de claquement).
    pub fn set(&mut self, enabled: bool, gains: [f32; 10]) {
        for (i, g) in gains.iter().enumerate() {
            let g = if enabled && g.is_finite() { g.clamp(-24.0, 24.0) as f64 } else { 0.0 };
            self.target[i] = g;
        }
        self.preamp_target = preamp_db(enabled, &gains) as f64;
    }

    fn settled(&self) -> bool {
        self.current == self.target && self.preamp == self.preamp_target
    }

    fn neutral(&self) -> bool {
        self.settled() && self.current.iter().all(|g| *g == 0.0) && self.preamp == 0.0
    }

    /// Réponse (dB) de la chaîne complète réglée sur ses cibles, préampli compris (tests, affichage).
    pub fn response_db(&self, freq: f64) -> f64 {
        let mut db = self.preamp_target;
        for (i, g) in self.target.iter().enumerate() {
            db +=
                Coeffs::new(band_kind(i), self.sample_rate, EQ_BANDS[i], EQ_Q, *g).magnitude_db(freq, self.sample_rate);
        }
        db
    }

    fn update_coeffs(&mut self) {
        for (i, f) in self.filters.iter_mut().enumerate() {
            f.set(Coeffs::new(band_kind(i), self.sample_rate, EQ_BANDS[i], EQ_Q, self.current[i]));
        }
    }

    /// Fait avancer le lissage d'un sous-bloc ; rend le préampli (linéaire) du début et de la fin du sous-bloc.
    fn step(&mut self) -> (f32, f32) {
        let before = super::gain::db_to_linear(self.preamp as f32);
        let mut changed = false;
        for i in 0..10 {
            let (c, t) = (self.current[i], self.target[i]);
            if c != t {
                let n = c + (t - c) * self.k;
                self.current[i] = if (t - n).abs() < 0.005 { t } else { n };
                changed = true;
            }
        }
        if self.preamp != self.preamp_target {
            let n = self.preamp + (self.preamp_target - self.preamp) * self.k;
            self.preamp = if (self.preamp_target - n).abs() < 0.005 { self.preamp_target } else { n };
        }
        if changed {
            self.update_coeffs();
        }
        (before, super::gain::db_to_linear(self.preamp as f32))
    }

    /// Traite un bloc stéréo entrelacé sur place.
    pub fn process(&mut self, buf: &mut [f32]) {
        if self.neutral() {
            if self.active {
                // Retour au neutre : les filtres sont l'identité, on les libère (et leurs états).
                self.filters.iter_mut().for_each(Biquad::reset);
                self.active = false;
            }
            return;
        }
        if !self.active {
            self.active = true;
            self.update_coeffs();
        }
        for chunk in buf.chunks_mut(SUB_BLOCK * 2) {
            let (g0, g1) = if self.settled() {
                let g = super::gain::db_to_linear(self.preamp as f32);
                (g, g)
            } else {
                self.step()
            };
            let frames = chunk.len() / 2;
            let dg = (g1 - g0) / frames.max(1) as f32;
            for (j, frame) in chunk.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let g = g0 + dg * j as f32;
                let (mut l, mut r) = (frame[0] * g, frame[1] * g);
                for f in self.filters.iter_mut() {
                    (l, r) = f.process(l, r);
                }
                frame[0] = l;
                frame[1] = r;
            }
        }
        self.filters.iter_mut().for_each(Biquad::flush_denormals);
    }
}

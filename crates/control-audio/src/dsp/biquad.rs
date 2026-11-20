//! Filtres biquadratiques, avec les formules de Web Audio (`BiquadFilterNode`), pour que l'égaliseur et
//! l'échange des basses d'AutoMix sonnent comme dans la version Electron.

use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    /// Plateau grave : la fréquence est le milieu de la pente (le gain y vaut la moitié). Q ignoré, comme Web Audio.
    LowShelf,
    /// Plateau aigu, symétrique du précédent.
    HighShelf,
    /// Cloche : le gain est atteint exactement à la fréquence centrale.
    Peaking,
}

/// Coefficients normalisés (a0 = 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coeffs {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Default for Coeffs {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Coeffs {
    pub const IDENTITY: Coeffs = Coeffs { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    /// Formules de l'« Audio EQ Cookbook » telles que la spécification Web Audio les reprend (plateaux avec S = 1).
    pub fn new(kind: FilterKind, sample_rate: f64, freq: f64, q: f64, gain_db: f64) -> Coeffs {
        // Au-delà de Nyquist, le filtre n'a pas de sens : on le garde juste en dessous.
        let freq = freq.clamp(1.0, sample_rate * 0.45);
        let a = 10f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let (b0, b1, b2, a0, a1, a2) = match kind {
            FilterKind::Peaking => {
                let alpha = sin / (2.0 * q.max(1e-4));
                (1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a)
            }
            FilterKind::LowShelf => {
                let alpha = sin / 2.0 * 2f64.sqrt();
                let k = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + k),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - k),
                    (a + 1.0) + (a - 1.0) * cos + k,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - k,
                )
            }
            FilterKind::HighShelf => {
                let alpha = sin / 2.0 * 2f64.sqrt();
                let k = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + k),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - k),
                    (a + 1.0) - (a - 1.0) * cos + k,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - k,
                )
            }
        };
        Coeffs { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    /// Gain (dB) du filtre à une fréquence donnée : |H(e^jω)|.
    pub fn magnitude_db(&self, freq: f64, sample_rate: f64) -> f64 {
        let w = 2.0 * PI * freq / sample_rate;
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        // H(z) = (b0 + b1 z⁻¹ + b2 z⁻²) / (1 + a1 z⁻¹ + a2 z⁻²), z⁻ⁿ = cos(nω) − j sin(nω)
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = -(self.b1 * s1 + self.b2 * s2);
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = -(self.a1 * s1 + self.a2 * s2);
        let num = nr * nr + ni * ni;
        let den = (dr * dr + di * di).max(1e-300);
        10.0 * (num / den).log10()
    }
}

/// Un filtre stéréo en forme directe transposée II, calculé en f64 (les graves à 31 Hz restent stables).
#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    c: Coeffs,
    s: [[f64; 2]; 2],
}

impl Biquad {
    pub fn new(c: Coeffs) -> Biquad {
        Biquad { c, s: [[0.0; 2]; 2] }
    }

    pub fn set(&mut self, c: Coeffs) {
        self.c = c;
    }

    pub fn coeffs(&self) -> Coeffs {
        self.c
    }

    pub fn reset(&mut self) {
        self.s = [[0.0; 2]; 2];
    }

    #[inline]
    fn tick(c: &Coeffs, s: &mut [f64; 2], x: f64) -> f64 {
        let y = c.b0 * x + s[0];
        s[0] = c.b1 * x - c.a1 * y + s[1];
        s[1] = c.b2 * x - c.a2 * y;
        y
    }

    /// Filtre une trame stéréo.
    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let c = self.c;
        let yl = Self::tick(&c, &mut self.s[0], l as f64);
        let yr = Self::tick(&c, &mut self.s[1], r as f64);
        (yl as f32, yr as f32)
    }

    /// Filtre un bloc stéréo entrelacé.
    pub fn process_block(&mut self, buf: &mut [f32]) {
        for frame in buf.as_chunks_mut::<2>().0 {
            let (l, r) = self.process(frame[0], frame[1]);
            frame[0] = l;
            frame[1] = r;
        }
        self.flush_denormals();
    }

    /// Des états dénormalisés ralentissent énormément certains processeurs : on les met à zéro (après chaque bloc).
    pub fn flush_denormals(&mut self) {
        for s in self.s.iter_mut().flatten() {
            if s.abs() < 1e-25 {
                *s = 0.0;
            }
        }
    }
}

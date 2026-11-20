//! Rééchantillonnage en continu vers la fréquence d'analyse.
//!
//! L'original laissait faire `OfflineAudioContext.decodeAudioData`. Ici : un passe-bas Butterworth d'ordre 8 sous
//! la nouvelle fréquence de Nyquist (pas de repliement des aigus dans la bande analysée), puis une interpolation
//! linéaire. Largement assez fin pour une enveloppe d'attaques de 23 ms, et sans tout garder en mémoire.

/// Cellule biquadratique (forme directe II transposée), en f64.
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    /// Passe-bas (formules de R. Bristow-Johnson).
    fn lowpass(cutoff: f64, rate: f64, q: f64) -> Biquad {
        let w0 = 2.0 * std::f64::consts::PI * cutoff / rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha;
        Biquad {
            b0: (1.0 - cos) / 2.0 / a0,
            b1: (1.0 - cos) / a0,
            b2: (1.0 - cos) / 2.0 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Rééchantillonneur alimenté échantillon par échantillon.
#[derive(Debug, Clone)]
pub struct Resampler {
    /// Échantillons d'entrée par échantillon de sortie.
    step: f64,
    filters: Vec<Biquad>,
    /// Rang du prochain échantillon d'entrée.
    n: u64,
    /// Rang du prochain échantillon de sortie.
    k: u64,
    prev: f64,
    passthrough: bool,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Resampler {
        let (from, to) = (from.max(1), to.max(1));
        let step = from as f64 / to as f64;
        // Q des quatre cellules d'un Butterworth d'ordre 8 : 1 / (2 cos((2k − 1) π / 16)).
        let filters = if from > to {
            (1..=4)
                .map(|k| {
                    let q = 1.0 / (2.0 * ((2 * k - 1) as f64 * std::f64::consts::PI / 16.0).cos());
                    Biquad::lowpass(0.45 * to as f64, from as f64, q)
                })
                .collect()
        } else {
            Vec::new()
        };
        Resampler { step, filters, n: 0, k: 0, prev: 0.0, passthrough: from == to }
    }

    /// Ajoute un échantillon ; les échantillons de sortie prêts vont dans `out`.
    pub fn push(&mut self, sample: f32, out: &mut Vec<f32>) {
        if self.passthrough {
            out.push(sample);
            return;
        }
        let mut x = sample as f64;
        for f in &mut self.filters {
            x = f.process(x);
        }
        let n = self.n as f64;
        // L'échantillon de sortie k tombe à l'instant k·step (en rangs d'entrée), entre n − 1 et n.
        loop {
            let t = self.k as f64 * self.step;
            if t > n {
                break;
            }
            let frac = t - (n - 1.0);
            let y = if self.n == 0 { x } else { self.prev + (x - self.prev) * frac };
            out.push(y as f32);
            self.k += 1;
        }
        self.prev = x;
        self.n += 1;
    }
}

/// Rééchantillonne un signal entier.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    let mut r = Resampler::new(from, to);
    let mut out = Vec::with_capacity((input.len() as f64 * to as f64 / from.max(1) as f64) as usize + 1);
    for &s in input {
        r.push(s, &mut out);
    }
    out
}

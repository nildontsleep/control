//! Gains lissés, limiteur et calcul du gain de normalisation.

/// dB → facteur linéaire.
#[inline]
pub fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Facteur linéaire → dB.
#[inline]
pub fn linear_to_db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

/// Courbe du volume de l'interface (0..1) : quadratique, comme `volumeCurve` de la version Electron.
#[inline]
pub fn volume_curve(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    v * v
}

/// Bornes de la normalisation (repris de `loudness.js`).
pub const NORM_MIN_GAIN_DB: f64 = -18.0;
pub const NORM_MAX_GAIN_DB: f64 = 9.0;
const NORM_HEADROOM_DBTP: f64 = -1.0;

/// Gain de normalisation (dB) d'un morceau de niveau `loudness` (LUFS) et de crête vraie `true_peak` (dBTP) pour
/// atteindre `target` (LUFS). Même règle que `gainDb` de `loudness.js` : un morceau non mesuré reste inchangé, et une
/// relève s'arrête avant que la crête ne dépasse −1 dBTP (sans crête connue, pas de relève).
pub fn normalization_gain_db(loudness: Option<f64>, true_peak: Option<f64>, target: f64) -> f64 {
    let Some(loudness) = loudness.filter(|l| l.is_finite()) else { return 0.0 };
    let db = (target - loudness).clamp(NORM_MIN_GAIN_DB, NORM_MAX_GAIN_DB);
    if db > 0.0 {
        let room = true_peak.filter(|p| p.is_finite()).map_or(0.0, |p| NORM_HEADROOM_DBTP - p);
        return db.min(room).max(0.0);
    }
    db
}

/// Gain qui rejoint sa cible par une rampe linéaire de durée fixe : un changement de volume ne claque jamais.
#[derive(Clone, Copy, Debug)]
pub struct Smoothed {
    current: f32,
    target: f32,
    step: f32,
    remaining: u32,
    ramp: u32,
}

impl Smoothed {
    /// `ramp` : durée de la rampe en échantillons (au moins 1).
    pub fn new(value: f32, ramp: u32) -> Smoothed {
        Smoothed { current: value, target: value, step: 0.0, remaining: 0, ramp: ramp.max(1) }
    }

    pub fn set_target(&mut self, target: f32) {
        if (target - self.target).abs() < 1e-7 && (self.remaining > 0 || self.current == target) {
            return;
        }
        self.target = target;
        self.remaining = self.ramp;
        self.step = (target - self.current) / self.ramp as f32;
    }

    /// Saute directement à une valeur (pas de rampe).
    pub fn set_now(&mut self, value: f32) {
        self.current = value;
        self.target = value;
        self.remaining = 0;
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        if self.remaining > 0 {
            self.remaining -= 1;
            self.current = if self.remaining == 0 { self.target } else { self.current + self.step };
        }
        self.current
    }

    pub fn current(&self) -> f32 {
        self.current
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    pub fn is_settled(&self) -> bool {
        self.remaining == 0
    }

    /// Applique le gain (rampe comprise) à un bloc stéréo entrelacé.
    pub fn apply(&mut self, buf: &mut [f32]) {
        if self.is_settled() {
            let g = self.current;
            if g != 1.0 {
                buf.iter_mut().for_each(|s| *s *= g);
            }
            return;
        }
        for frame in buf.as_chunks_mut::<2>().0 {
            let g = self.tick();
            frame[0] *= g;
            frame[1] *= g;
        }
    }
}

/// Fenêtre d'anticipation du limiteur (trames) : sa latence est `LIMITER_LOOKAHEAD - 1`.
pub const LIMITER_LOOKAHEAD: usize = 64;

/// Limiteur à anticipation : le gain descend en douceur *avant* une crête (maximum glissant puis moyenne sur la
/// fenêtre d'anticipation), remonte lentement ensuite. Par construction, le gain appliqué à chaque échantillon ne
/// dépasse jamais celui qui le ramène au seuil : la sortie ne dépasse pas le seuil (−1 dBFS), donc jamais 0 dBFS.
#[derive(Clone, Debug)]
pub struct Limiter {
    threshold: f32,
    release: f32,
    /// Gain voulu (avec relâchement), par trame, sur la fenêtre.
    wanted: [f32; LIMITER_LOOKAHEAD],
    /// Minimum glissant de `wanted`, sur la fenêtre (pour la moyenne).
    held: [f32; LIMITER_LOOKAHEAD],
    delay: [f32; LIMITER_LOOKAHEAD * 2],
    pos: usize,
    last: f32,
}

impl Limiter {
    pub fn new(sample_rate: u32) -> Limiter {
        Limiter {
            threshold: db_to_linear(-1.0),
            release: 1.0 - (-1.0 / (0.12 * sample_rate as f32)).exp(),
            wanted: [1.0; LIMITER_LOOKAHEAD],
            held: [1.0; LIMITER_LOOKAHEAD],
            delay: [0.0; LIMITER_LOOKAHEAD * 2],
            pos: 0,
            last: 1.0,
        }
    }

    pub fn threshold(&self) -> f32 {
        self.threshold
    }

    pub fn reset(&mut self) {
        self.wanted = [1.0; LIMITER_LOOKAHEAD];
        self.held = [1.0; LIMITER_LOOKAHEAD];
        self.delay = [0.0; LIMITER_LOOKAHEAD * 2];
        self.last = 1.0;
    }

    /// Traite un bloc stéréo entrelacé (sur place, avec la latence de l'anticipation).
    pub fn process(&mut self, buf: &mut [f32]) {
        const N: usize = LIMITER_LOOKAHEAD;
        for frame in buf.as_chunks_mut::<2>().0 {
            let (l, r) = (frame[0], frame[1]);
            let peak = l.abs().max(r.abs());
            let need = if peak > self.threshold { self.threshold / peak } else { 1.0 };
            // Relâchement : le gain remonte vers 1 avec une constante de temps de 120 ms, mais jamais au-dessus du besoin.
            let rel = self.last + (1.0 - self.last) * self.release;
            let g = need.min(rel);
            self.last = g;
            let p = self.pos;
            self.wanted[p] = g;
            self.held[p] = self.wanted.iter().copied().fold(1.0, f32::min);
            let gain = self.held.iter().sum::<f32>() / N as f32;
            // Sortie : l'échantillon arrivé il y a N − 1 trames (toutes les fenêtres moyennées le contiennent).
            let out = (p + 1) % N;
            let dl = self.delay[out * 2];
            let dr = self.delay[out * 2 + 1];
            self.delay[p * 2] = l;
            self.delay[p * 2 + 1] = r;
            self.pos = out;
            // Filet de sécurité contre les arrondis : la sortie reste dans [−1, 1].
            frame[0] = (dl * gain).clamp(-1.0, 1.0);
            frame[1] = (dr * gain).clamp(-1.0, 1.0);
        }
    }
}

//! Analyse d'un morceau : tempo, début et fin audibles, repères de la grille de battements.
//!
//! Portage fidèle de `automix/analysis.js` : mêmes constantes, mêmes boucles, mêmes arrondis (les tableaux
//! intermédiaires restent en f32 comme les `Float32Array` d'origine, les calculs en f64 comme les nombres JS),
//! pour qu'un morceau donne la même grille qu'avant.

use serde::{Deserialize, Serialize};

/// Hz : de quoi suivre les kicks et les caisses claires, léger à calculer.
pub const ANALYSIS_RATE: u32 = 11025;
const RATE: f64 = ANALYSIS_RATE as f64;
/// Échantillons par image d'analyse (≈ 23 ms).
const HOP: usize = 256;
const HOP_SEC: f64 = HOP as f64 / RATE;
pub const MIN_BPM: f64 = 70.0;
pub const MAX_BPM: f64 = 180.0;

/// Résultat de l'analyse (noms JSON d'origine). Tous les instants sont en secondes depuis le début du fichier.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    /// Durée du son analysé.
    pub dur: f64,
    /// Tempo estimé (70 à 180 BPM), au centième.
    pub bpm: f64,
    /// Confiance dans le tempo : score du meilleur candidat rapporté à la moyenne (≥ 1,35 : fiable).
    pub conf: f64,
    /// Début audible (silence de tête retiré).
    pub start: f64,
    /// Fin audible (silence de queue retiré).
    pub end: f64,
    /// Premier temps de la grille dans les 30 premières secondes audibles : pour entrer dans le morceau, la grille
    /// est `grid_start + k · 60 / bpm`.
    pub grid_start: f64,
    /// Premier temps de la grille dans les 30 dernières secondes audibles : recalée sur la fin (une petite erreur de
    /// tempo s'accumule sur le morceau), c'est elle qui place la sortie d'un mixage.
    pub grid_end: f64,
}

/// `Math.round` : moitié vers +∞.
pub(super) fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// Arrondi à `n` décimales, comme `round` d'origine.
pub(super) fn round_to(v: f64, n: i32) -> f64 {
    let k = 10f64.powi(n);
    js_round(v * k) / k
}

/// Valeur d'un signal échantillonné en un point fractionnaire (interpolation linéaire).
fn at(arr: &[f32], x: f64) -> f64 {
    let i = x.floor();
    if i < 0.0 || i >= arr.len() as f64 - 1.0 {
        return 0.0;
    }
    let i = i as usize;
    let f = x - i as f64;
    arr[i] as f64 * (1.0 - f) + arr[i + 1] as f64 * f
}

/// Enveloppe d'attaques : où le son « frappe ». Deux bandes (graves : kick ; reste : caisse claire, charleston)
/// dont on garde la hausse d'énergie d'une image à l'autre, puis on retire la moyenne locale pour ne garder que les
/// pics. Rend aussi le niveau efficace de chaque image.
fn onset_envelope(x: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let frames = x.len() / HOP;
    let mut low = vec![0f32; frames];
    let mut high = vec![0f32; frames];
    let mut level = vec![0f32; frames];
    let mut lp = 0f64;
    let a = 1.0 - (-2.0 * std::f64::consts::PI * 180.0 / RATE).exp();
    for f in 0..frames {
        let (mut el, mut eh, mut all) = (0f64, 0f64, 0f64);
        for &s in &x[f * HOP..(f + 1) * HOP] {
            let s = s as f64;
            lp += a * (s - lp);
            let hp = s - lp;
            el += lp * lp;
            eh += hp * hp;
            all += s * s;
        }
        low[f] = (el / HOP as f64) as f32;
        high[f] = (eh / HOP as f64) as f32;
        level[f] = (all / HOP as f64).sqrt() as f32;
    }
    let lift = |arr: &[f32], f: usize| (((1e-6 + arr[f] as f64).ln()) - (1e-6 + arr[f - 1] as f64).ln()).max(0.0);
    let mut env = vec![0f32; frames];
    for (f, e) in env.iter_mut().enumerate().skip(1) {
        *e = (lift(&low, f) * 1.0 + lift(&high, f) * 0.6) as f32;
    }
    // Retire la moyenne locale (≈ 1 s) : seuls les pics bien au-dessus du fond restent.
    let win = js_round(RATE / HOP as f64) as usize;
    let mut out = vec![0f32; frames];
    let mut sum = 0f64;
    for f in 0..frames {
        sum += env[f] as f64;
        if f >= win {
            sum -= env[f - win] as f64;
        }
        let mean = sum / (f + 1).min(win) as f64;
        out[f] = (env[f] as f64 - mean * 1.2).max(0.0) as f32;
    }
    (out, level)
}

/// Début et fin audibles (hors silences de tête et de queue), en secondes.
fn audible_range(level: &[f32]) -> (f64, f64) {
    let mut sorted = level.to_vec();
    sorted.sort_by(f32::total_cmp);
    let loud = sorted.get((sorted.len() as f64 * 0.95).floor() as usize).map_or(0.0, |&v| v as f64);
    if loud < 1e-4 {
        return (0.0, level.len() as f64 * HOP_SEC);
    }
    let thr_start = (loud * 0.01).max(3e-4); // −40 dB
    let thr_end = (loud * 0.03).max(3e-4); // −30 dB
    let mut s = 0usize;
    while s < level.len() && (level[s] as f64) < thr_start {
        s += 1;
    }
    let mut e = level.len() as i64 - 1;
    while e > s as i64 && (level[e as usize] as f64) < thr_end {
        e -= 1;
    }
    let start = (s as i64 - 1).max(0) as f64 * HOP_SEC;
    let end = (level.len() as i64).min(e + 2) as f64 * HOP_SEC;
    (start, end)
}

/// Corrélation de l'enveloppe avec elle-même décalée de `lag` images (fractionnaire), sur [from, to[.
fn autocorr(env: &[f32], lag: f64, from: i64, to: i64) -> f64 {
    let mut sum = 0f64;
    let mut n = 0u64;
    let stop = to as f64 - lag - 1.0;
    let mut i = from;
    while (i as f64) < stop {
        let v = if i >= 0 { env.get(i as usize).map_or(0.0, |&v| v as f64) } else { 0.0 };
        sum += v * at(env, i as f64 + lag);
        n += 1;
        i += 1;
    }
    if n > 0 { sum / n as f64 } else { 0.0 }
}

/// Tempo le plus probable de la zone [from, to[ (en images) : (bpm, confiance).
fn estimate_tempo(env: &[f32], from: i64, to: i64) -> (f64, f64) {
    let mut candidates: Vec<(f64, f64)> = Vec::with_capacity(441);
    let mut bpm = MIN_BPM;
    while bpm <= MAX_BPM {
        let lag = 60.0 / bpm / HOP_SEC;
        // On additionne la corrélation à 1, 2 et 4 temps : un vrai tempo se répète à chaque mesure.
        let score = autocorr(env, lag, from, to)
            + 0.6 * autocorr(env, lag * 2.0, from, to)
            + 0.4 * autocorr(env, lag * 4.0, from, to);
        // Préférence douce pour les tempos courants (autour de 115 BPM) : tranche entre un tempo et sa moitié ou
        // son double.
        let prior = (-0.5 * ((bpm / 115.0).log2() / 0.75).powi(2)).exp();
        candidates.push((bpm, score * (0.6 + 0.4 * prior)));
        bpm += 0.25;
    }
    let mut best = candidates[0];
    let mut total = 0f64;
    for &c in &candidates {
        total += c.1;
        if c.1 > best.1 {
            best = c;
        }
    }
    let mean = total / candidates.len() as f64;
    // Affine autour du meilleur candidat : maximum local de la corrélation simple.
    let mut bpm = best.0;
    let mut top = -1f64;
    let mut b = best.0 - 0.6;
    while b <= best.0 + 0.6 {
        let sc = autocorr(env, 60.0 / b / HOP_SEC, from, to);
        if sc > top {
            top = sc;
            bpm = b;
        }
        b += 0.02;
    }
    (bpm, if mean > 0.0 { best.1 / mean } else { 0.0 })
}

/// Instant (en secondes) d'un temps fort de la zone [from, to[ pour un tempo donné : repère de la grille.
fn beat_phase(env: &[f32], bpm: f64, from: i64, to: i64) -> f64 {
    let period = 60.0 / bpm / HOP_SEC; // images par temps
    let mut best_phase = 0f64;
    let mut best_score = -1f64;
    let mut phase = 0f64;
    while phase < period {
        let mut s = 0f64;
        let mut x = from as f64 + phase;
        while x < to as f64 - 1.0 {
            s += at(env, x);
            x += period;
        }
        if s > best_score {
            best_score = s;
            best_phase = phase;
        }
        phase += 0.25;
    }
    (from as f64 + best_phase) * HOP_SEC
}

/// Affine le tempo par mesures de plus en plus espacées : on repère un temps fort dans une fenêtre au début puis
/// dans une autre plus loin, et on divise la durée qui les sépare par le nombre de temps entre eux. Le nombre de
/// temps n'est juste que si le tempo déjà connu dérive de moins d'un demi-temps sur l'écart : on double donc l'écart
/// à chaque tour, chaque mesure rendant la suivante possible. Au bout, une erreur de 0,5 % qui décalerait la grille
/// de plusieurs dixièmes de seconde sur un morceau entier tombe à quelques millisecondes.
fn refine_tempo(env: &[f32], bpm0: f64, start_f: i64, end_f: i64) -> f64 {
    let to_f = |s: f64| js_round(s / HOP_SEC) as i64;
    let win = to_f(10.0);
    let margin = to_f(3.0);
    let first = start_f + margin;
    let last = end_f - margin - win; // début de la dernière fenêtre possible
    if last - first < to_f(30.0) {
        return bpm0;
    }
    let phase_at = |from: i64, bpm: f64| beat_phase(env, bpm, from, from + win);
    let mut bpm = bpm0;
    let mut gap = to_f(15.0);
    loop {
        let to = (first + gap).min(last);
        let a = phase_at(first, bpm);
        let b = phase_at(to, bpm);
        let beats = js_round((b - a) / (60.0 / bpm));
        if beats < 8.0 {
            return bpm;
        }
        let next = 60.0 / ((b - a) / beats);
        // Un écart de plus de 2 % avec le tempo mesuré à grande échelle : mesure aberrante, on garde l'ancien.
        if (next / bpm0 - 1.0).abs() > 0.02 {
            return bpm;
        }
        bpm = next;
        if to >= last {
            break;
        }
        gap *= 2;
    }
    // Une dernière passe sur les fenêtres extrêmes, avec le tempo désormais précis.
    let a = phase_at(first, bpm);
    let b = phase_at(last, bpm);
    let beats = js_round((b - a) / (60.0 / bpm));
    if beats >= 8.0 {
        let next = 60.0 / ((b - a) / beats);
        if (next / bpm - 1.0).abs() < 0.005 {
            bpm = next;
        }
    }
    bpm
}

/// Analyse d'un signal mono déjà à [`ANALYSIS_RATE`] (`analyzeSamples` d'origine).
pub fn analyze_mono(mono: &[f32]) -> Analysis {
    let dur = mono.len() as f64 / RATE;
    let (env, level) = onset_envelope(mono);
    let (start, end) = audible_range(&level);
    let start_f = (start / HOP_SEC).floor() as i64;
    let end_f = (env.len() as i64).min((end / HOP_SEC).ceil() as i64);
    // Le tempo se mesure sur la partie centrale (l'intro et l'outro sont rarement représentatives), sinon sur tout.
    let span = end_f - start_f;
    let long = span as f64 > 60.0 / HOP_SEC * 2.0;
    let mid_from = if long { start_f + (span as f64 * 0.1).floor() as i64 } else { start_f };
    let mid_to = if long { end_f - (span as f64 * 0.1).floor() as i64 } else { end_f };
    let (coarse, conf) = estimate_tempo(&env, mid_from, mid_to);
    let bpm = refine_tempo(&env, coarse, start_f, end_f);
    let win = js_round(30.0 / HOP_SEC) as i64;
    let grid_start = beat_phase(&env, bpm, start_f, end_f.min(start_f + win));
    let grid_end = beat_phase(&env, bpm, start_f.max(end_f - win), end_f);
    Analysis {
        dur: round_to(dur, 3),
        bpm: round_to(bpm, 2),
        conf: round_to(conf, 2),
        start: round_to(start, 3),
        end: round_to(end, 3),
        grid_start: round_to(grid_start, 3),
        grid_end: round_to(grid_end, 3),
    }
}

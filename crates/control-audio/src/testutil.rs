//! Outils des tests : fichiers WAV générés, signaux, mesures.
#![allow(clippy::unwrap_used)]

use std::io::Write;
use std::path::Path;

/// Écrit un WAV PCM 16 bits (échantillons entrelacés sur `channels` canaux).
pub fn write_wav(path: &Path, sample_rate: u32, channels: u16, samples: &[f32]) {
    let data_len = (samples.len() * 2) as u32;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&channels.to_le_bytes()).unwrap();
    f.write_all(&sample_rate.to_le_bytes()).unwrap();
    f.write_all(&(sample_rate * channels as u32 * 2).to_le_bytes()).unwrap();
    f.write_all(&(channels * 2).to_le_bytes()).unwrap();
    f.write_all(&16u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data_len.to_le_bytes()).unwrap();
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        f.write_all(&v.to_le_bytes()).unwrap();
    }
    f.flush().unwrap();
}

/// Sinusoïde mono.
pub fn sine(freq: f64, sample_rate: u32, secs: f64, amp: f32) -> Vec<f32> {
    let n = (secs * sample_rate as f64).round() as usize;
    (0..n).map(|i| amp * (2.0 * std::f64::consts::PI * freq * i as f64 / sample_rate as f64).sin() as f32).collect()
}

/// Bruit blanc reproductible (générateur congruentiel), dans [−amp, amp].
pub fn noise(n: usize, seed: u64, amp: f32) -> Vec<f32> {
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((x >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0) * amp
        })
        .collect()
}

/// Canal gauche d'un tampon stéréo entrelacé.
pub fn left(stereo: &[f32]) -> Vec<f32> {
    stereo.as_chunks::<2>().0.iter().map(|f| f[0]).collect()
}

pub fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

pub fn db(x: f64) -> f64 {
    20.0 * x.max(1e-12).log10()
}

/// Fréquence dominante par comptage des passages par zéro montants (interpolés).
pub fn zero_crossing_freq(x: &[f32], sample_rate: u32) -> f64 {
    let mut first = None;
    let mut last = 0.0;
    let mut count = 0usize;
    for i in 1..x.len() {
        let (a, b) = (x[i - 1], x[i]);
        if a < 0.0 && b >= 0.0 {
            let t = (i - 1) as f64 + (-a / (b - a)) as f64;
            if first.is_none() {
                first = Some(t);
            } else {
                count += 1;
            }
            last = t;
        }
    }
    match first {
        Some(f) if count > 0 => count as f64 * sample_rate as f64 / (last - f),
        _ => 0.0,
    }
}

#![allow(clippy::unwrap_used)]

use super::biquad::{Coeffs, FilterKind};
use super::eq::{EQ_BANDS, EQ_PRESETS, EQ_Q, Equalizer, band_kind, preamp_db, preset};
use super::fade::{equal_power, smoothstep};
use super::gain::{LIMITER_LOOKAHEAD, Limiter, Smoothed, normalization_gain_db};
use super::stretch::TimeStretch;
use crate::testutil::{db, left, noise, rms, sine, zero_crossing_freq};

const SR: u32 = 44_100;

fn stereo(mono: &[f32]) -> Vec<f32> {
    mono.iter().flat_map(|&s| [s, s]).collect()
}

#[test]
fn eq_bands_hit_their_gain_at_the_center() {
    for (i, &f) in EQ_BANDS.iter().enumerate() {
        for g in [-12.0, -6.0, 3.0, 6.0, 12.0] {
            let c = Coeffs::new(band_kind(i), SR as f64, f, EQ_Q, g);
            let at = c.magnitude_db(f, SR as f64);
            // Cloches : le gain exact au centre ; plateaux (Web Audio, S = 1) : la moitié au point milieu.
            let want = if band_kind(i) == FilterKind::Peaking { g } else { g / 2.0 };
            assert!((at - want).abs() < 0.5, "bande {f} Hz, {g} dB : {at:.2} dB au centre");
        }
    }
    // Plateaux : tout le gain loin du point milieu.
    let low = Coeffs::new(FilterKind::LowShelf, SR as f64, 31.0, EQ_Q, 6.0);
    assert!((low.magnitude_db(5.0, SR as f64) - 6.0).abs() < 0.5);
    assert!(low.magnitude_db(1000.0, SR as f64).abs() < 0.1);
    let high = Coeffs::new(FilterKind::HighShelf, SR as f64, 16000.0, EQ_Q, 6.0);
    assert!(high.magnitude_db(100.0, SR as f64).abs() < 0.1);
}

#[test]
fn eq_measured_on_a_sine_matches_the_band_gain() {
    // Chaque bande en cloche seule à +6 dB : gain mesuré à sa fréquence = 6 dB moins le préampli (−3,6 dB).
    for (i, &f) in EQ_BANDS.iter().enumerate().skip(1).take(8) {
        let mut gains = [0.0f32; 10];
        gains[i] = 6.0;
        let mut eq = Equalizer::new(SR);
        eq.set(true, gains);
        let mut buf = stereo(&sine(f, SR, 1.0, 0.25));
        eq.process(&mut buf);
        let l = left(&buf);
        let tail = &l[l.len() / 2..];
        let measured = db(rms(tail) / (0.25 / 2f64.sqrt()));
        let want = 6.0 + preamp_db(true, &gains) as f64;
        assert!((measured - want).abs() < 0.5, "{f} Hz : {measured:.2} dB au lieu de {want:.2}");
        assert!((eq.response_db(f) - want).abs() < 0.5);
    }
    // Désactivé : transparent.
    let mut eq = Equalizer::new(SR);
    eq.set(false, preset("rock").unwrap());
    let mut buf = stereo(&sine(1000.0, SR, 0.2, 0.5));
    let before = buf.clone();
    eq.process(&mut buf);
    assert_eq!(buf, before);
}

#[test]
fn eq_presets_and_preamp_match_the_original() {
    assert_eq!(EQ_PRESETS.len(), 10);
    assert_eq!(preset("bass").unwrap(), [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    assert_eq!(preset("loudness").unwrap(), [6.0, 4.0, 0.0, 0.0, -1.0, 0.0, -1.0, 0.0, 4.0, 2.0]);
    assert!(preset("nope").is_none());
    assert!((preamp_db(true, &preset("bass").unwrap()) + 3.6).abs() < 1e-6);
    assert_eq!(preamp_db(true, &preset("flat").unwrap()), 0.0);
    assert_eq!(preamp_db(false, &preset("bass").unwrap()), 0.0);
}

#[test]
fn eq_changes_are_smoothed() {
    // Passage brutal de 0 à +12 dB sur la bande de 1 kHz : pas de saut dans le signal.
    let mut eq = Equalizer::new(SR);
    let mut buf = stereo(&sine(1000.0, SR, 0.4, 0.2));
    let (a, b) = buf.split_at_mut(SR as usize / 5 * 2);
    eq.process(a);
    let mut gains = [0.0; 10];
    gains[5] = 12.0;
    eq.set(true, gains);
    eq.process(b);
    let l = left(&buf);
    let max_step = l.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
    // Pente maximale d'une sinusoïde de 1 kHz d'amplitude 0,2 × 4 (+12 dB) × préampli (−7,2 dB) : ≈ 0,05.
    assert!(max_step < 0.08, "saut de {max_step}");
}

#[test]
fn equal_power_crossfade_keeps_energy() {
    for i in 0..=100 {
        let (o, n) = equal_power(i as f64 / 100.0);
        assert!(((o * o + n * n) - 1.0).abs() < 1e-6);
    }
    assert_eq!(equal_power(0.0), (1.0, 0.0));
    let (o, n) = equal_power(1.0);
    assert!(o.abs() < 1e-6 && (n - 1.0).abs() < 1e-6);
    // Deux bruits non corrélés de même niveau : le mélange garde le même niveau d'un bout à l'autre du fondu.
    let len = SR as usize * 2;
    let a = noise(len, 1, 0.5);
    let b = noise(len, 2, 0.5);
    let mix: Vec<f32> = (0..len)
        .map(|i| {
            let (o, n) = equal_power(i as f64 / len as f64);
            a[i] * o + b[i] * n
        })
        .collect();
    let reference = rms(&a);
    for w in mix.chunks(len / 20) {
        let d = db(rms(w) / reference);
        assert!(d.abs() < 0.5, "écart de {d:.2} dB pendant le fondu");
    }
    assert!((smoothstep(0.5) - 0.5).abs() < 1e-12 && smoothstep(2.0) == 1.0);
}

#[test]
fn limiter_never_exceeds_full_scale() {
    let mut lim = Limiter::new(SR);
    // Bruit poussé de +20 dB (crêtes à 5) puis une sinusoïde, et des impulsions isolées.
    let mut buf = stereo(&noise(SR as usize, 7, 5.0));
    buf.extend(stereo(&sine(220.0, SR, 0.5, 3.0)));
    for i in (0..buf.len()).step_by(4410) {
        buf[i] = 8.0;
    }
    lim.process(&mut buf);
    let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak <= 1.0, "crête {peak}");
    assert!(peak <= lim.threshold() + 1e-6, "crête {peak} au-dessus du seuil");
    // Sous le seuil : intact, à la latence près.
    let mut lim = Limiter::new(SR);
    let quiet = stereo(&sine(440.0, SR, 0.2, 0.5));
    let mut out = quiet.clone();
    lim.process(&mut out);
    let lat = (LIMITER_LOOKAHEAD - 1) * 2;
    for (o, i) in out[lat..].iter().zip(&quiet) {
        assert!((o - i).abs() < 1e-6);
    }
}

#[test]
fn smoothed_gain_ramps_linearly() {
    let mut g = Smoothed::new(0.0, 4);
    g.set_target(1.0);
    let v: Vec<f32> = (0..6).map(|_| g.tick()).collect();
    assert_eq!(v, vec![0.25, 0.5, 0.75, 1.0, 1.0, 1.0]);
    assert!(g.is_settled());
}

#[test]
fn normalization_follows_loudness_js() {
    // −10 LUFS vers −16 : −6 dB.
    assert!((normalization_gain_db(Some(-10.0), Some(-0.5), -16.0) + 6.0).abs() < 1e-9);
    // Relève limitée par la crête vraie (−1 dBTP) : −22 LUFS, crête −4 dBTP → +3 dB au lieu de +6.
    assert!((normalization_gain_db(Some(-22.0), Some(-4.0), -16.0) - 3.0).abs() < 1e-9);
    // Sans crête connue : pas de relève.
    assert_eq!(normalization_gain_db(Some(-22.0), None, -16.0), 0.0);
    // Bornes −18 / +9 dB.
    assert!((normalization_gain_db(Some(10.0), None, -16.0) + 18.0).abs() < 1e-9);
    assert!((normalization_gain_db(Some(-40.0), Some(-30.0), -16.0) - 9.0).abs() < 1e-9);
    // Pas mesuré : inchangé.
    assert_eq!(normalization_gain_db(None, None, -16.0), 0.0);
}

/// Étire tout un signal stéréo à vitesse constante.
fn stretch_all(input: &[f32], rate: f64) -> Vec<f32> {
    let mut ts = TimeStretch::new(SR);
    ts.reset(rate);
    let mut out = Vec::new();
    let mut buf = vec![0.0f32; 1024];
    let mut chunks = input.chunks(1500);
    loop {
        let n = ts.pull(&mut buf, rate);
        out.extend_from_slice(&buf[..n * 2]);
        if n * 2 == buf.len() {
            continue;
        }
        if ts.is_done() {
            break;
        }
        match chunks.next() {
            Some(c) => ts.push(c),
            None => ts.finish(),
        }
    }
    out
}

#[test]
fn time_stretch_keeps_duration_ratio_and_pitch() {
    let secs = 3.0;
    let input = stereo(&sine(440.0, SR, secs, 0.5));
    let frames_in = input.len() / 2;
    for rate in [0.5, 0.75, 0.9, 1.04, 1.25, 1.5, 2.0] {
        let out = stretch_all(&input, rate);
        let frames_out = out.len() / 2;
        let want = frames_in as f64 / rate;
        assert!((frames_out as f64 - want).abs() <= 2.0, "vitesse {rate} : {frames_out} trames au lieu de {want:.0}");
        let l = left(&out);
        let mid = &l[l.len() / 5..l.len() * 4 / 5];
        let f = zero_crossing_freq(mid, SR);
        assert!((f / 440.0 - 1.0).abs() < 0.01, "vitesse {rate} : {f:.1} Hz");
        // Pas de trou ni de saut d'amplitude : le niveau reste celui de la sinusoïde.
        for w in mid.chunks(2048) {
            let r = rms(w) / (0.5 / 2f64.sqrt());
            assert!(r > 0.8 && r < 1.2, "vitesse {rate} : niveau {r:.2}");
        }
    }
}

#[test]
fn time_stretch_at_unit_rate_is_transparent() {
    let input = stereo(&noise(SR as usize, 3, 0.5));
    let out = stretch_all(&input, 1.0);
    assert_eq!(out.len(), input.len());
    let err = out.iter().zip(&input).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
    assert!(err < 1e-5, "écart {err}");
}

#[test]
fn time_stretch_position_tracks_input() {
    let input = stereo(&sine(300.0, SR, 2.0, 0.5));
    let mut ts = TimeStretch::new(SR);
    let rate = 1.3;
    ts.reset(rate);
    ts.push(&input);
    ts.finish();
    let mut buf = vec![0.0f32; 2 * 4410];
    let mut produced = 0usize;
    while !ts.is_done() {
        let n = ts.pull(&mut buf, rate);
        produced += n;
        if produced < 70_000 {
            let pos = ts.position();
            assert!((pos - produced as f64 * rate).abs() < 2.0, "{pos} contre {}", produced as f64 * rate);
        }
        if n == 0 {
            break;
        }
    }
}

use super::*;
use crate::loudness::decode::write_wav;

const SR: u32 = ANALYSIS_RATE;

/// Bruit déterministe dans [−1, 1].
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    }
}

/// Piste de « kicks » à `bpm` : silence jusqu'à `t0`, un coup par temps jusqu'à `until`, puis silence jusqu'à
/// `total` secondes. Chaque coup : sinus grave amorti et petit bruit d'attaque.
fn click_track(bpm: f64, t0: f64, until: f64, total: f64, rate: u32) -> Vec<f32> {
    let n = (total * rate as f64) as usize;
    let mut out = vec![0f32; n];
    let mut noise = Lcg(7);
    let beat = 60.0 / bpm;
    let mut t = t0;
    while t < until {
        let first = (t * rate as f64).round() as usize;
        for i in 0..(0.12 * rate as f64) as usize {
            let Some(slot) = out.get_mut(first + i) else { break };
            let tau = i as f64 / rate as f64;
            let kick = 0.7 * (2.0 * std::f64::consts::PI * 60.0 * tau).sin() * (-tau / 0.04).exp();
            let click = 0.3 * noise.next() as f64 * (-tau / 0.006).exp();
            *slot += (kick + click) as f32;
        }
        t += beat;
    }
    out
}

/// Écart (s) entre un instant et la grille `t0 + k · beat` la plus proche.
fn phase_error(at: f64, t0: f64, bpm: f64) -> f64 {
    let beat = 60.0 / bpm;
    let d = (at - t0) / beat;
    (d - d.round()) * beat
}

fn assert_grid(a: &Analysis, bpm: f64, t0: f64) {
    assert!((a.bpm - bpm).abs() < 0.3, "tempo {} au lieu de {bpm} ({a:?})", a.bpm);
    assert!(a.conf >= 1.35, "confiance {} ({a:?})", a.conf);
    // Une image d'analyse fait 23 ms : la grille tombe sur l'image où l'attaque commence.
    for (what, at) in [("début", a.grid_start), ("fin", a.grid_end)] {
        let err = phase_error(at, t0, bpm);
        assert!(err.abs() < 0.035, "grille ({what}) décalée de {err:.3} s ({a:?})");
    }
}

#[test]
fn click_track_tempo_and_beat_phase() {
    let mono = click_track(120.0, 2.0, 90.0, 93.0, SR);
    let a = analyze_samples(&mono, SR);
    assert_grid(&a, 120.0, 2.0);
    assert!((a.start - 2.0).abs() < 0.05, "début audible {}", a.start);
    assert!(a.end > 89.5 && a.end < 90.5, "fin audible {}", a.end);
    assert!((a.dur - 93.0).abs() < 0.01);
    assert!(usable(&a));
}

#[test]
fn other_tempos_without_octave_errors() {
    for (bpm, t0) in [(95.0, 1.3), (128.0, 0.71), (174.0, 0.4), (72.0, 3.05)] {
        let mono = click_track(bpm, t0, 75.0, 76.0, SR);
        assert_grid(&analyze_samples(&mono, SR), bpm, t0);
    }
}

#[test]
fn other_sample_rates_are_resampled() {
    let rate = 44_100;
    let mono = click_track(128.0, 1.0, 70.0, 72.0, rate);
    let a = analyze_samples(&mono, rate);
    assert_grid(&a, 128.0, 1.0);
    assert!((a.dur - 72.0).abs() < 0.01, "durée {}", a.dur);
}

#[test]
fn head_and_tail_silence_trimmed() {
    let mut mono = vec![0f32; SR as usize];
    mono.extend((0..SR as usize * 10).map(|i| 0.3 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SR as f32).sin()));
    mono.extend(std::iter::repeat_n(0f32, SR as usize * 4));
    let a = analyze_samples(&mono, SR);
    assert!((a.start - 1.0).abs() < 0.05, "début {}", a.start);
    assert!((a.end - 11.0).abs() < 0.05, "fin {}", a.end);
    assert!((a.dur - 15.0).abs() < 0.001);
    // Un son tenu n'a pas de tempo fiable.
    assert!(!usable(&a));
}

#[test]
fn silence_and_tiny_inputs_do_not_panic() {
    // Tout muet : rien à retirer (215 images entières de 256 échantillons).
    let silent = analyze_samples(&vec![0f32; SR as usize * 5], SR);
    assert_eq!((silent.start, silent.end), (0.0, 4.992));
    assert_eq!(silent.conf, 0.0);
    let empty = analyze_samples(&[], SR);
    assert_eq!((empty.dur, empty.start, empty.end), (0.0, 0.0, 0.0));
    let _ = analyze_samples(&[0.5; 300], 48_000);
}

#[test]
fn analysis_json_uses_original_names() {
    let a = Analysis { dur: 200.0, bpm: 120.0, conf: 2.0, start: 0.1, end: 199.0, grid_start: 0.5, grid_end: 180.5 };
    let v = serde_json::to_value(a).unwrap();
    assert_eq!(
        v,
        serde_json::json!({ "dur": 200.0, "bpm": 120.0, "conf": 2.0, "start": 0.1, "end": 199.0,
            "gridStart": 0.5, "gridEnd": 180.5 })
    );
    assert_eq!(serde_json::from_value::<Analysis>(v).unwrap(), a);
}

fn track(bpm: f64) -> Analysis {
    Analysis { dur: 200.0, bpm, conf: 2.0, start: 0.2, end: 198.0, grid_start: 0.25, grid_end: 180.25 }
}

#[test]
fn mix_plan_on_the_grid() {
    let out = track(120.0);
    let inc = track(122.0);
    let p = plan_mix(&out, &inc, MixOptions::default()).unwrap();
    assert_eq!(p.beat_out, 0.5);
    assert_eq!(p.length, 8.0, "16 temps à 120 BPM");
    // Dernier temps de la grille qui laisse 8 s avant la fin audible (198 s).
    assert_eq!(p.start_out, 189.75);
    assert!(((p.start_out - out.grid_end) / p.beat_out).fract().abs() < 1e-9);
    assert_eq!(p.start_in, 0.25);
    assert_eq!(p.ratio, round(120.0 / 122.0));
    assert_eq!(p.rate_in, p.ratio);
    assert_eq!((p.grid_out, p.grid_in), (180.25, 0.25));

    assert!(plan_mix(&out, &track(61.0), MixOptions::default()).is_none(), "61 BPM : hors de la plage analysée");
    // Tempo moitié : ramené au double.
    let p = plan_mix(&track(160.0), &track(81.0), MixOptions::default()).unwrap();
    assert_eq!(p.ratio, round(160.0 / 162.0), "81 BPM compte comme 162");
    assert_eq!(p.beat_out, 0.375);
    assert_eq!(p.length, 6.0, "16 temps à 160 BPM");

    // Trop d'écart de tempo : fondu simple.
    assert!(plan_mix(&out, &track(140.0), MixOptions::default()).is_none());
    // Analyse peu fiable.
    assert!(plan_mix(&out, &Analysis { conf: 1.2, ..inc }, MixOptions::default()).is_none());

    // Arrivée tardive par un saut : mixage plus court, sur un temps.
    let late = plan_mix(&out, &inc, MixOptions { from: 193.1, rate: 1.1, ..MixOptions::default() }).unwrap();
    assert_eq!(late.start_out, 193.25);
    assert_eq!(late.length, 4.5);
    assert_eq!(late.rate_in, round(120.0 / 122.0 * 1.1));
    assert!(plan_mix(&out, &inc, MixOptions { from: 196.5, ..MixOptions::default() }).is_none(), "moins de 4 temps");
}

fn round(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

#[test]
fn file_analysis_and_cache() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("kicks.wav");
    let rate = 44_100;
    let mono = click_track(120.0, 1.5, 60.0, 62.0, rate);
    let stereo: Vec<f32> = mono.iter().flat_map(|&s| [s, s]).collect();
    write_wav(&wav, 2, rate, &stereo);

    let a = analyze_file(&wav).unwrap();
    assert_grid(&a, 120.0, 1.5);

    let cache_dir = dir.path().join("cache");
    {
        let cache = AnalysisCache::open(&cache_dir);
        assert_eq!(cache.peek(&wav), None, "pas encore analysé");
        assert_eq!(cache.analyze(&wav), Some(a));
        assert_eq!(cache.peek(&wav), Some(Some(a)));

        let bad = dir.path().join("bad.mp3");
        std::fs::write(&bad, b"pas du son").unwrap();
        assert_eq!(cache.analyze(&bad), None);
        assert_eq!(cache.peek(&bad), Some(None), "échec retenu en mémoire");
    }
    // Rouvert : la réussite est sur le disque, pas l'échec.
    let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(cache_dir.join(CACHE_FILE)).unwrap()).unwrap();
    assert_eq!(raw["v"], CACHE_VERSION);
    assert_eq!(raw["items"].as_object().unwrap().len(), 1);
    let cache = AnalysisCache::open(&cache_dir);
    assert_eq!(cache.peek(&wav), Some(Some(a)));
    assert_eq!(cache.peek(&dir.path().join("bad.mp3")), None);

    // Fichier remplacé : la signature change, l'analyse est à refaire.
    write_wav(&wav, 2, rate, &stereo[..stereo.len() - 2 * rate as usize]);
    assert_eq!(cache.peek(&wav), None);

    cache.forget_all();
    assert!(!cache_dir.join(CACHE_FILE).exists());
}

#[test]
fn too_long_files_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("long.wav");
    // 21 minutes à 400 Hz : assez long, sans être lourd.
    write_wav(&wav, 1, 400, &vec![0.0; 400 * 60 * 21]);
    let err = analyze_file(&wav).unwrap_err();
    assert!(err.to_string().contains("trop long"), "{err}");
}

#[test]
fn resampler_keeps_length_and_low_tones() {
    let rate = 48_000;
    let tone: Vec<f32> =
        (0..rate * 2).map(|i| (2.0 * std::f32::consts::PI * 200.0 * i as f32 / rate as f32).sin()).collect();
    let out = resample(&tone, rate, SR);
    assert!((out.len() as i64 - 2 * SR as i64).abs() <= 1, "{}", out.len());
    let rms = (out[2000..].iter().map(|s| s * s).sum::<f32>() / (out.len() - 2000) as f32).sqrt();
    assert!((rms - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.02, "rms {rms}");
    // Au-dessus de la nouvelle fréquence de Nyquist : coupé.
    let high: Vec<f32> =
        (0..rate * 2).map(|i| (2.0 * std::f32::consts::PI * 9000.0 * i as f32 / rate as f32).sin()).collect();
    let out = resample(&high, rate, SR);
    let rms = (out[2000..].iter().map(|s| s * s).sum::<f32>() / (out.len() - 2000) as f32).sqrt();
    assert!(rms < 0.02, "repliement {rms}");
}

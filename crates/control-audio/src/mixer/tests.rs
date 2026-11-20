//! Enchaînements rendus hors ligne avec des fichiers WAV générés.
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use crossbeam_channel::Receiver;

use super::transition::{Stage, Transition};
use crate::automix::BeatInfo;
use crate::dsp::gain::LIMITER_LOOKAHEAD;
use crate::testutil::{db, left, rms, sine, write_wav, zero_crossing_freq};
use crate::{Engine, EngineConfig, EngineEvent, OfflineRenderer, TransitionKind};

/// Banc d'essai : un moteur hors ligne, rendu par petits blocs, avec l'heure (temps réel simulé) de chaque événement.
struct Rig {
    engine: Engine,
    r: OfflineRenderer,
    events: Receiver<EngineEvent>,
    sr: u32,
    t: f64,
    log: Vec<(f64, EngineEvent)>,
    out: Vec<f32>,
    dir: tempfile::TempDir,
}

const CHUNK: usize = 256;

impl Rig {
    fn new(sr: u32) -> Rig {
        let (engine, r) = Engine::offline(EngineConfig::default(), sr);
        let events = engine.events();
        Rig { engine, r, events, sr, t: 0.0, log: Vec::new(), out: Vec::new(), dir: tempfile::tempdir().unwrap() }
    }

    fn wav(&self, name: &str, sr: u32, mono: &[f32]) -> PathBuf {
        let p = self.dir.path().join(name);
        write_wav(&p, sr, 1, mono);
        p
    }

    /// Rend `secs` secondes (en gardant le son si `keep`) ; `each` est appelé après chaque bloc.
    fn run_with(&mut self, secs: f64, keep: bool, mut each: impl FnMut(&Rig)) {
        let mut buf = vec![0.0f32; CHUNK * 2];
        let blocks = (secs * self.sr as f64 / CHUNK as f64).round() as usize;
        for _ in 0..blocks {
            self.r.render(&mut buf);
            self.t += CHUNK as f64 / self.sr as f64;
            while let Ok(e) = self.events.try_recv() {
                self.log.push((self.t, e));
            }
            if keep {
                self.out.extend_from_slice(&buf);
            }
            each(self);
        }
    }

    fn run(&mut self, secs: f64, keep: bool) {
        self.run_with(secs, keep, |_| {});
    }

    /// Heure du premier événement qui satisfait `f`.
    fn when(&self, f: impl Fn(&EngineEvent) -> bool) -> Option<f64> {
        self.log.iter().find(|(_, e)| f(e)).map(|(t, _)| *t)
    }

    fn kinds(&self) -> Vec<String> {
        self.log.iter().map(|(_, e)| format!("{e:?}")).collect()
    }
}

fn dc(secs: f64, sr: u32, v: f32) -> Vec<f32> {
    vec![v; (secs * sr as f64) as usize]
}

#[test]
fn gapless_switch_is_sample_exact() {
    let sr = 44_100;
    let mut rig = Rig::new(sr);
    let a = rig.wav("a.wav", sr, &dc(0.5, sr, 0.25));
    let b = rig.wav("b.wav", sr, &dc(0.5, sr, -0.25));
    rig.engine.load(1, &a, 0.0);
    rig.engine.preload_next(2, &b);
    rig.run(1.3, true);
    let started = rig.when(|e| *e == EngineEvent::Started { id: 1 }).unwrap();
    let advanced = rig.when(|e| *e == EngineEvent::Advanced { from: 1, to: 2 }).unwrap();
    let ended = rig.when(|e| *e == EngineEvent::Ended { id: 2 }).unwrap();
    assert!(started < 0.01);
    assert!((advanced - 0.5).abs() < 0.01, "{:?}", rig.kinds());
    assert!((ended - 1.0).abs() < 0.02, "{:?}", rig.kinds());
    assert!(rig.when(|e| *e == EngineEvent::NearEnd { id: 1 }).is_some());
    assert!(rig.when(|e| matches!(e, EngineEvent::Ended { id: 1 })).is_none());
    // Le son : 0,25 jusqu'à l'échantillon 22 050 exactement, puis −0,25, sans trou (à la latence du limiteur près).
    let l = left(&rig.out);
    let lat = LIMITER_LOOKAHEAD - 1;
    let switch = 22_050 + lat;
    for (i, v) in l.iter().enumerate().take(44_100 + lat).skip(1000) {
        let want = if i < switch { 0.25 } else { -0.25 };
        assert!((v - want).abs() < 2e-3, "échantillon {i} : {v}");
    }
    assert!(l[44_100 + lat + 10..].iter().all(|v| v.abs() < 1e-6));
    let s = rig.engine.status();
    assert_eq!(s.track, Some(2));
    assert!(!s.playing && s.position == 0.0);
}

#[test]
fn crossfade_is_equal_power_and_switches_at_the_start() {
    let sr = 22_050;
    let mut rig = Rig::new(sr);
    let a = rig.wav("a.wav", sr, &sine(440.0, sr, 10.0, 0.3));
    let b = rig.wav("b.wav", sr, &sine(660.0, sr, 10.0, 0.3));
    rig.engine.set_crossfade(3.0);
    rig.engine.load(1, &a, 0.0);
    rig.engine.preload_next(2, &b);
    let mut during = Vec::new();
    rig.run_with(18.0, true, |rig| {
        if rig.t > 7.2 && rig.t < 9.7 {
            during.push(rig.engine.status());
        }
    });
    let advanced = rig.when(|e| *e == EngineEvent::Advanced { from: 1, to: 2 }).unwrap();
    // Le fondu commence quand il reste la durée du fondu (+ 50 ms) : le suivant devient aussitôt le morceau en cours.
    assert!(advanced > 6.94 && advanced < 6.98, "{advanced}");
    assert!(during.iter().all(|s| s.track == Some(2) && s.transition == TransitionKind::Crossfade && s.deck == 1));
    let ended = rig.when(|e| *e == EngineEvent::Ended { id: 2 }).unwrap();
    assert!((ended - (advanced + 10.0)).abs() < 0.05, "{ended}");
    // Énergie constante d'un bout à l'autre (deux sinusoïdes de même niveau, non corrélées).
    let l = left(&rig.out);
    let reference = rms(&l[sr as usize..sr as usize * 6]);
    let w = sr as usize / 10;
    let from = (7.0 * sr as f64) as usize;
    for chunk in l[from..from + 29 * w].chunks(w) {
        let d = db(rms(chunk) / reference);
        assert!(d.abs() < 0.5, "écart de {d:.2} dB pendant le fondu");
    }
    // Après le fondu : seul le suivant, à 660 Hz.
    let f = zero_crossing_freq(&l[(11.0 * sr as f64) as usize..(12.0 * sr as f64) as usize], sr);
    assert!((f - 660.0).abs() < 2.0);
    assert_eq!(rig.engine.status().transition, TransitionKind::None);
}

#[test]
fn seeking_during_a_crossfade_keeps_only_the_new_track() {
    let sr = 22_050;
    let mut rig = Rig::new(sr);
    let a = rig.wav("a.wav", sr, &sine(440.0, sr, 10.0, 0.3));
    let b = rig.wav("b.wav", sr, &sine(660.0, sr, 10.0, 0.3));
    rig.engine.set_crossfade(4.0);
    rig.engine.load(1, &a, 5.0);
    rig.engine.preload_next(2, &b);
    rig.run(2.0, false);
    assert_eq!(rig.engine.status().transition, TransitionKind::Crossfade);
    rig.engine.seek(1.0);
    rig.run(0.5, true);
    let s = rig.engine.status();
    assert_eq!((s.track, s.transition), (Some(2), TransitionKind::None));
    assert!((s.position - 1.5).abs() < 0.03, "{}", s.position);
    let l = left(&rig.out);
    let f = zero_crossing_freq(&l[l.len() / 2..], sr);
    assert!((f - 660.0).abs() < 2.0);
}

fn beats(duration: f64, bpm: f64, start: f64, end: f64) -> BeatInfo {
    BeatInfo {
        duration,
        bpm,
        confidence: 2.0,
        audible_start: start,
        audible_end: end,
        grid_start: start,
        grid_end: start + ((end - start - 5.0) * bpm / 60.0).floor() * 60.0 / bpm,
    }
}

#[test]
fn automix_falls_back_to_a_fade_on_the_audible_end() {
    let sr = 22_050;
    // 45 s : son jusqu'à 40 s puis silence ; le suivant commence par 2 s de silence.
    let mut a = sine(440.0, sr, 40.0, 0.3);
    a.extend(dc(5.0, sr, 0.0));
    let mut b = dc(2.0, sr, 0.0);
    b.extend(sine(660.0, sr, 43.0, 0.3));
    // Tempos trop éloignés (100 et 140 BPM) : pas de mixage calé, fondu de secours de 6 s calé sur la fin audible.
    let mut rig = Rig::new(sr);
    let (pa, pb) = (rig.wav("a.wav", sr, &a), rig.wav("b.wav", sr, &b));
    rig.engine.load(1, &pa, 30.0);
    rig.engine.preload_next(2, &pb);
    rig.engine.set_automix(true, Some(beats(45.0, 100.0, 0.0, 40.0)), Some(beats(45.0, 140.0, 2.0, 45.0)));
    let mut first = None;
    rig.run_with(6.0, false, |rig| {
        if first.is_none() && rig.engine.status().track == Some(2) {
            first = Some(rig.engine.status());
        }
    });
    let advanced = rig.when(|e| *e == EngineEvent::Advanced { from: 1, to: 2 }).unwrap();
    assert!((advanced - 3.95).abs() < 0.03, "{advanced}");
    let first = first.unwrap();
    assert_eq!(first.transition, TransitionKind::Crossfade);
    // L'entrant saute son silence de tête (début audible − 50 ms).
    assert!((first.position - 1.95).abs() < 0.03, "{}", first.position);

    // Analyse inconnue : même fondu de secours, sur la fin du fichier.
    let mut rig = Rig::new(sr);
    let (pa, pb) = (rig.wav("a.wav", sr, &a), rig.wav("b.wav", sr, &b));
    rig.engine.load(1, &pa, 30.0);
    rig.engine.preload_next(2, &pb);
    rig.engine.set_automix(true, None, None);
    rig.run(10.0, false);
    let advanced = rig.when(|e| *e == EngineEvent::Advanced { from: 1, to: 2 }).unwrap();
    assert!((advanced - 8.95).abs() < 0.03, "{advanced}");
}

/// Piste de clics : un bref 1,5 kHz sur chaque temps.
fn clicks(sr: u32, secs: f64, bpm: f64, first: f64) -> Vec<f32> {
    let mut x = dc(secs, sr, 0.0);
    let beat = 60.0 / bpm;
    let mut t = first;
    while t < secs - 0.1 {
        let i0 = (t * sr as f64) as usize;
        for k in 0..(0.015 * sr as f64) as usize {
            x[i0 + k] = 0.5 * (2.0 * std::f64::consts::PI * 1500.0 * k as f64 / sr as f64).sin() as f32;
        }
        t += beat;
    }
    x
}

#[test]
fn automix_matches_tempo_enters_on_beat_and_switches_mid_mix() {
    let sr = 22_050;
    let mut rig = Rig::new(sr);
    let pa = rig.wav("a.wav", sr, &clicks(sr, 60.0, 120.0, 0.25));
    let pb = rig.wav("b.wav", sr, &clicks(sr, 60.0, 125.0, 0.1));
    let out = BeatInfo { grid_end: 55.25, ..beats(60.0, 120.0, 0.25, 59.5) };
    let inc = BeatInfo { audible_start: 0.05, ..beats(60.0, 125.0, 0.1, 59.5) };
    let plan = crate::plan_mix(&out, &inc, Default::default()).unwrap();
    assert!(
        (plan.start_out - 51.25).abs() < 1e-9 && (plan.length - 8.0).abs() < 1e-9 && (plan.start_in - 0.1).abs() < 1e-9
    );
    rig.engine.load(1, &pa, 42.0);
    rig.engine.preload_next(2, &pb);
    rig.engine.set_automix(true, Some(out), Some(inc));
    let mut worst: f64 = 0.0;
    let mut speeds = Vec::new();
    let mut mixing_seen = false;
    rig.run_with(20.0, false, |rig| {
        let m = rig.r.mixer();
        if let Some(Transition::Mix(mix)) = m.transition()
            && mix.stage == Stage::Mixing
        {
            mixing_seen = true;
            let p_out = m.deck_position(mix.out);
            let target = mix.plan.start_in + (p_out - mix.plan.start_out) * mix.plan.ratio;
            worst = worst.max((m.deck_position(mix.inc) - target).abs());
            speeds.push(m.deck_speed(mix.inc));
            assert_eq!(m.current_deck(), if mix.switched { mix.inc } else { mix.out });
        }
    });
    assert!(mixing_seen);
    // Pas de fondu de secours : AutoMix attendait son moment.
    let started = rig.when(|e| *e == EngineEvent::Started { id: 2 }).unwrap();
    let advanced = rig.when(|e| *e == EngineEvent::Advanced { from: 1, to: 2 }).unwrap();
    // Entrée sur le temps de 51,25 s (9,25 s après le départ à 42 s), changement au milieu des 16 temps.
    assert!((started - 9.25).abs() < 0.03, "{started}");
    assert!((advanced - 13.25).abs() < 0.03, "{advanced}");
    // Calage : l'entrant reste à quelques millisecondes de sa cible pendant tout le mixage.
    assert!(worst < 0.015, "écart de calage {worst:.4} s");
    assert!(speeds.iter().all(|s| (s / 0.96 - 1.0).abs() <= 0.041), "{speeds:?}");
    // Fin du mixage à 59,25 s (17,25 s), puis la vitesse revient en douceur vers 1.
    let s = rig.engine.status();
    assert_eq!((s.track, s.transition), (Some(2), TransitionKind::None));
    let speed = rig.r.mixer().deck_speed(rig.r.mixer().current_deck());
    assert!(speed > 0.96 && speed < 1.0, "{speed}");
    rig.run(7.0, false);
    assert!((rig.r.mixer().deck_speed(rig.r.mixer().current_deck()) - 1.0).abs() < 1e-9);
}

#[test]
fn pause_seek_and_rate_keep_positions_right() {
    let sr = 22_050;
    let mut rig = Rig::new(sr);
    let a = rig.wav("a.wav", sr, &sine(440.0, sr, 10.0, 0.3));
    rig.engine.load(7, &a, 0.0);
    rig.run(1.0, false);
    let s = rig.engine.status();
    assert!(s.playing && (s.position - 1.0).abs() < 0.03, "{s:?}");
    assert_eq!(s.duration, Some(10.0));
    rig.engine.pause();
    rig.run(0.5, true);
    let s = rig.engine.status();
    assert!(!s.playing && (s.position - 1.0).abs() < 0.04, "{s:?}");
    assert!(rig.out[rig.out.len() / 2..].iter().all(|v| *v == 0.0), "silence en pause");
    rig.engine.seek(5.0);
    rig.r.render(&mut []);
    assert!((rig.engine.position() - 5.0).abs() < 1e-6);
    rig.engine.play();
    rig.run(1.0, false);
    assert!((rig.engine.position() - 6.0).abs() < 0.03);
    // Vitesse 1,5 : la position avance 1,5 fois plus vite, la hauteur ne change pas.
    rig.engine.set_rate(1.5);
    rig.out.clear();
    rig.run(1.0, true);
    assert!((rig.engine.position() - 7.5).abs() < 0.05, "{}", rig.engine.position());
    let f = zero_crossing_freq(&left(&rig.out)[2000..], sr);
    assert!((f / 440.0 - 1.0).abs() < 0.01, "{f} Hz");
}

#[test]
fn volume_mute_and_track_gain_with_limiter() {
    let sr = 22_050;
    let mut rig = Rig::new(sr);
    let a = rig.wav("a.wav", sr, &sine(440.0, sr, 10.0, 0.5));
    rig.engine.load(1, &a, 0.0);
    rig.run(1.0, true);
    let unity = rms(&rig.out[sr as usize..]);
    // Volume 0,5 : courbe quadratique, −12 dB.
    rig.engine.set_volume(0.5);
    rig.out.clear();
    rig.run(1.0, true);
    let half = rms(&rig.out[sr as usize / 2..]);
    assert!((db(half / unity) + 12.04).abs() < 0.3, "{:.2} dB", db(half / unity));
    rig.engine.set_muted(true);
    rig.out.clear();
    rig.run(0.5, true);
    assert!(rig.out[2000..].iter().all(|v| v.abs() < 1e-6));
    // +18 dB de normalisation sur une sinusoïde à −6 dBFS : le limiteur tient la crête sous 0 dBFS.
    rig.engine.set_muted(false);
    rig.engine.set_volume(1.0);
    rig.engine.set_track_gain_db(1, 18.0);
    rig.out.clear();
    rig.run(1.0, true);
    let peak = rig.out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak <= 0.9 && peak > 0.8, "crête {peak}");
    // Le gain d'un morceau annoncé avant son chargement s'applique dès son arrivée.
    rig.engine.set_track_gain_db(2, -6.0);
    rig.engine.load(2, &a, 0.0);
    rig.out.clear();
    rig.run(1.0, true);
    let quiet = rms(&rig.out[sr as usize..]);
    assert!((db(quiet / unity) + 6.0).abs() < 0.3, "{:.2} dB", db(quiet / unity));
}

#[test]
fn eq_applies_to_the_mix() {
    let sr = 44_100;
    let mut rig = Rig::new(sr);
    let a = rig.wav("a.wav", sr, &sine(1000.0, sr, 3.0, 0.2));
    rig.engine.load(1, &a, 0.0);
    rig.run(0.5, true);
    let flat = rms(&rig.out[sr as usize / 2..]);
    let mut gains = [0.0; 10];
    gains[5] = 6.0;
    rig.engine.set_eq(true, gains);
    rig.out.clear();
    rig.run(0.5, true);
    let boosted = rms(&rig.out[sr as usize / 2..]);
    // +6 dB moins le préampli (60 % du gain maximal).
    assert!((db(boosted / flat) - 2.4).abs() < 0.5, "{:.2} dB", db(boosted / flat));
}

#[test]
fn errors_are_reported_with_the_track_id() {
    let mut rig = Rig::new(22_050);
    let missing = rig.dir.path().join("absent.flac");
    rig.engine.load(3, &missing, 0.0);
    rig.engine.preload_next(4, &missing);
    rig.run(0.1, false);
    let errors: Vec<_> = rig
        .log
        .iter()
        .filter_map(
            |(_, e)| if let EngineEvent::Error { id, message } = e { Some((*id, message.clone())) } else { None },
        )
        .collect();
    assert_eq!(errors.len(), 2, "{:?}", rig.kinds());
    assert_eq!(errors[0].0, 3);
    assert!(errors[0].1.contains("introuvable"));
    assert_eq!(errors[1].0, 4);
    assert!(!rig.engine.status().playing);
}

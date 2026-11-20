use super::decode::write_wav;
use super::*;
use std::time::Duration;

use serde_json::json;

/// Sinus de `freq` Hz, d'amplitude crête `amp`, identique sur chaque voie.
fn sine(freq: f64, amp: f64, secs: f64, rate: u32, channels: usize) -> Vec<f32> {
    let n = (secs * rate as f64) as usize;
    let mut out = Vec::with_capacity(n * channels);
    for i in 0..n {
        let v = (amp * (2.0 * std::f64::consts::PI * freq * i as f64 / rate as f64).sin()) as f32;
        out.extend(std::iter::repeat_n(v, channels));
    }
    out
}

fn db_to_amp(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

#[test]
fn gain_toward_target_bounded_without_clipping() {
    let t = -16.0;
    assert_eq!(gain_db(Some(-8.0), Some(-0.2), t), -8.0, "un morceau fort est atténué");
    assert_eq!(gain_db(Some(-23.0), Some(-10.0), t), 7.0, "un morceau faible est relevé");
    assert_eq!(gain_db(Some(-23.0), Some(-3.0), t), 2.0, "la relève s'arrête avant la crête (−1 dBTP)");
    assert_eq!(gain_db(Some(-23.0), None, t), 0.0, "crête inconnue : pas de relève");
    assert_eq!(gain_db(Some(-40.0), Some(-30.0), t), MAX_GAIN, "relève plafonnée");
    assert_eq!(gain_db(Some(5.0), None, t), MIN_GAIN, "atténuation plafonnée");
    assert_eq!(gain_db(None, None, t), 0.0, "pas encore mesuré : inchangé");
    assert_eq!(gain_db(Some(-23.0), Some(0.5), t), 0.0, "crête déjà au-dessus de la marge : rien");
    assert_eq!(gain_db(Some(-8.0), None, f64::NAN), -8.0, "niveau visé invalide : −16 LUFS");
    assert!((to_linear(-6.0206) - 0.5).abs() < 1e-4);
}

#[test]
fn ffmpeg_summary_parsed_and_silence_recognized() {
    let summary = "\n[Parsed_ebur128_0 @ 0x55] Summary:\n\n  Integrated loudness:\n    I:         -14.3 LUFS\n    \
                   Threshold: -24.4 LUFS\n\n  Loudness range:\n    LRA:         5.2 LU\n\n  True peak:\n    \
                   Peak:       -0.4 dBFS\n";
    assert_eq!(parse_summary(summary), Some(Level { loudness: -14.3, true_peak: Some(-0.4) }));
    assert_eq!(parse_summary("I:         -inf LUFS"), None);
    assert_eq!(parse_summary("rien d'utile"), None);
    assert_eq!(parse_summary("I: -20.0 LUFS"), Some(Level { loudness: -20.0, true_peak: None }));
}

#[test]
fn sine_level_matches_theory() {
    // EBU Tech 3341 : un sinus de 1 kHz à −20 dBFS sur les deux voies d'un stéréo vaut −20 LUFS.
    let rate = 48_000;
    let stereo = sine(997.0, db_to_amp(-20.0), 10.0, rate, 2);
    let l = measure_interleaved(&stereo, 2, rate).unwrap();
    assert!((l.loudness - -20.0).abs() < 0.5, "stéréo : {}", l.loudness);
    let peak = l.true_peak.unwrap();
    assert!((peak - -20.0).abs() < 0.5, "crête : {peak}");

    // En mono, une seule voie compte : 3 dB de moins (10·log10(2)).
    let mono = sine(997.0, db_to_amp(-20.0), 10.0, 44_100, 1);
    let l = measure_interleaved(&mono, 1, 44_100).unwrap();
    assert!((l.loudness - (-20.0 - 3.0103)).abs() < 0.5, "mono : {}", l.loudness);

    assert_eq!(measure_interleaved(&vec![0.0; 44_100 * 4], 2, 44_100), None, "silence : rien à mesurer");
}

#[test]
fn file_measured_through_decoder() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("sine.wav");
    write_wav(&f, 2, 44_100, &sine(997.0, db_to_amp(-12.0), 6.0, 44_100, 2));
    let l = measure_file(&f, None, &|| false).unwrap();
    assert!((l.loudness - -12.0).abs() < 0.5, "{}", l.loudness);
    assert!((l.true_peak.unwrap() - -12.0).abs() < 0.5);

    let quiet = dir.path().join("silence.wav");
    write_wav(&quiet, 2, 44_100, &vec![0.0; 44_100 * 2 * 3]);
    assert_eq!(measure_file(&quiet, None, &|| false), None);

    // Illisible par symphonia (Opus, fichier abîmé) et pas de ffmpeg : aucune valeur.
    let opus = dir.path().join("x.opus");
    std::fs::write(&opus, b"OggS pas vraiment de l'opus").unwrap();
    assert_eq!(measure_file(&opus, None, &|| false), None);

    // Arrêt demandé : rien n'est rendu.
    assert_eq!(measure_file(&f, None, &|| true), None);
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !f() {
        assert!(Instant::now() < deadline, "délai dépassé : {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn background_worker_measures_only_when_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let loud = dir.path().join("fort.wav");
    let soft = dir.path().join("doux.wav");
    write_wav(&loud, 2, 44_100, &sine(440.0, 0.125 * 0.8, 4.0, 44_100, 2));
    write_wav(&soft, 2, 44_100, &sine(440.0, 0.125 * 0.05, 4.0, 44_100, 2));
    let db = crate::db::open(&dir.path().join("library.db")).unwrap();
    for (path, kind) in [
        (loud.to_string_lossy().into_owned(), "audio"),
        (soft.to_string_lossy().into_owned(), "audio"),
        ("/nulle/part.mp3".to_string(), "audio"),
        ("/film.mkv".to_string(), "video"),
    ] {
        db.execute("INSERT INTO tracks (path, kind, title, added_at) VALUES (?, ?, 't', 1)", params![path, kind])
            .unwrap();
    }
    let db: SharedDb = Arc::new(Mutex::new(db));
    let store = Store::open(dir.path().join("settings.json"));
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let emit: Emit = Arc::new(move |e| {
        if let Event::Json(ch, v) = e {
            sink.lock().push((ch, v));
        }
    });
    let loudness = Loudness::new(Arc::clone(&db), store.clone(), Arc::new(|| None), emit);

    let p = loudness.progress();
    assert_eq!(p, Progress { enabled: false, total: 3, done: 0, running: false, ffmpeg: true });
    assert!(!loudness.kick(), "rien ne se lance tant que l'option est coupée");

    store.patch(&json!({ "normalize": true }));
    assert!(loudness.kick());
    // Le dernier événement, envoyé une fois la passe finie, donne l'état final.
    wait_until("mesure terminée", || {
        events.lock().last().is_some_and(|(_, v)| v["done"] == 3 && v["running"] == false)
    });

    let ids: Vec<TrackId> = (1..=3).collect();
    let mut asked = ids.clone();
    asked.push(99_999);
    let levels = loudness.levels(&asked);
    let fort = levels[&1].unwrap();
    let doux = levels[&2].unwrap();
    assert!(fort.loudness.unwrap() > doux.loudness.unwrap() + 10.0, "{fort:?} vs {doux:?}");
    // Crête du sinus : 0,125 × 0,8 = −20 dBFS.
    assert!((fort.true_peak.unwrap() - -20.0).abs() < 1.0, "{fort:?}");
    assert_eq!(levels[&3], Some(TrackLevel { loudness: None, true_peak: None }), "fichier absent : tenté, sans valeur");
    assert_eq!(levels[&99_999], None);
    assert_eq!(serde_json::to_value(&levels).unwrap()["1"]["truePeak"].as_f64(), fort.true_peak, "noms JSON d'origine");
    assert!(events.lock().iter().all(|(ch, _)| *ch == "loudness:progress"));

    // Tout refaire.
    loudness.reset();
    wait_until("nouvelle mesure", || {
        let p = loudness.progress();
        p.done == 3 && !p.running
    });
    assert!(loudness.levels(&[1])[&1].unwrap().loudness.is_some());

    loudness.close();
    assert!(!loudness.kick(), "fermé : plus rien ne se lance");
}

/// Avec ffmpeg installé (sinon ignoré) : un Opus, que symphonia ne lit pas, est mesuré par ffmpeg, et les deux
/// mesures concordent sur un même fichier.
#[test]
fn ffmpeg_fallback_for_opus() {
    let Some(ffmpeg) = crate::process::which("ffmpeg") else { return };
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("sine.wav");
    write_wav(&wav, 2, 48_000, &sine(997.0, db_to_amp(-18.0), 6.0, 48_000, 2));
    let opus = dir.path().join("sine.opus");
    let ok = std::process::Command::new(&ffmpeg)
        .args(["-v", "quiet", "-y", "-i"])
        .arg(&wav)
        .args(["-c:a", "libopus", "-b:a", "128k"])
        .arg(&opus)
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        return; // ffmpeg sans libopus
    }
    assert_eq!(measure_file(&opus, None, &|| false), None, "sans ffmpeg : pas de valeur");
    let by_ffmpeg = measure_file(&opus, Some(&ffmpeg), &|| false).unwrap();
    assert!((by_ffmpeg.loudness - -18.0).abs() < 0.5, "{by_ffmpeg:?}");
    let ours = measure_file(&wav, None, &|| false).unwrap();
    let theirs = measure::measure_ffmpeg(&ffmpeg, &wav, &|| false).unwrap().unwrap();
    assert!((ours.loudness - theirs.loudness).abs() < 0.2, "{ours:?} vs {theirs:?}");
    assert!((ours.true_peak.unwrap() - theirs.true_peak.unwrap()).abs() < 0.3, "{ours:?} vs {theirs:?}");
}

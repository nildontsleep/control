#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use super::{OpenOptions, Read, Source};
use crate::testutil::{left, sine, write_wav, zero_crossing_freq};

fn opts(rate: u32) -> OpenOptions {
    OpenOptions { ffmpeg: None, sample_rate: rate }
}

/// Dent de scie de période 1000 échantillons : la valeur lue donne la position exacte (modulo 1000).
fn saw(n: usize) -> Vec<f32> {
    (0..n).map(|i| (i % 1000) as f32 / 1000.0 * 0.5 - 0.25).collect()
}

fn saw_index(v: f32) -> i64 {
    ((v + 0.25) / 0.5 * 1000.0).round() as i64
}

fn read_all(src: &mut Source) -> Vec<f32> {
    let mut out = Vec::new();
    let mut buf = vec![0.0; 4096];
    loop {
        match src.read(&mut buf).unwrap() {
            Read::Frames(n) => out.extend_from_slice(&buf[..n * 2]),
            Read::Pending => std::thread::sleep(std::time::Duration::from_millis(2)),
            Read::End => return out,
        }
    }
}

fn first_frame(src: &mut Source) -> f32 {
    let mut buf = [0.0f32; 2];
    loop {
        match src.read(&mut buf).unwrap() {
            Read::Frames(_) => return buf[0],
            Read::Pending => std::thread::sleep(std::time::Duration::from_millis(2)),
            Read::End => panic!("fin inattendue"),
        }
    }
}

fn check_seeks(src: &mut Source, rate: u32, tolerance: i64) {
    for t in [1.2345, 0.0, 2.5, 0.5001, 1.0] {
        let at = src.seek(t).unwrap();
        assert!((at - t).abs() < 1e-9);
        let want = (t * rate as f64).round() as i64 % 1000;
        let got = saw_index(first_frame(src));
        let diff = (got - want).rem_euclid(1000).min((want - got).rem_euclid(1000));
        assert!(diff <= tolerance, "déplacement à {t} s : échantillon {got} au lieu de {want}");
    }
}

#[test]
fn wav_has_exact_duration_and_seeks_to_the_sample() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saw.wav");
    write_wav(&path, 44_100, 1, &saw(3 * 44_100));
    let mut src = Source::open(&path, &opts(44_100)).unwrap();
    assert_eq!(src.backend(), "symphonia");
    assert!((src.duration().unwrap() - 3.0).abs() < 1e-9);
    let all = read_all(&mut src);
    assert_eq!(all.len(), 3 * 44_100 * 2);
    // Mono dupliqué sur les deux canaux.
    assert_eq!(all[2001], all[2000]);
    check_seeks(&mut src, 44_100, 0);
}

#[test]
fn resampled_source_keeps_duration_pitch_and_timing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tone.wav");
    // 22,05 kHz → 48 kHz : 2 s de 1 kHz, puis 1 s de silence, puis 1 s à 0,5.
    let mut mono = sine(1000.0, 22_050, 2.0, 0.5);
    mono.extend(std::iter::repeat_n(0.0, 22_050));
    mono.extend(std::iter::repeat_n(0.5, 22_050));
    write_wav(&path, 22_050, 1, &mono);
    let mut src = Source::open(&path, &opts(48_000)).unwrap();
    assert!((src.duration().unwrap() - 4.0).abs() < 1e-9);
    let all = read_all(&mut src);
    assert!((all.len() as i64 / 2 - 4 * 48_000).abs() <= 2, "{} trames", all.len() / 2);
    let l = left(&all);
    let f = zero_crossing_freq(&l[4800..90_000], 48_000);
    assert!((f - 1000.0).abs() < 2.0, "{f} Hz");
    // Le palier commence à 3 s, aussi après un déplacement à 2,5 s.
    let step = |x: &[f32]| x.iter().position(|v| *v > 0.25).unwrap();
    let from = 5 * 48_000 / 2;
    assert!(((from + step(&l[from..])) as i64 - 3 * 48_000).abs() < 40);
    src.seek(2.5).unwrap();
    let after = left(&read_all(&mut src));
    assert!((step(&after) as i64 - 24_000).abs() < 40, "palier à {}", step(&after));
}

#[test]
fn stereo_channels_stay_separate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lr.wav");
    let frames: Vec<f32> = (0..4410).flat_map(|_| [0.5, -0.25]).collect();
    write_wav(&path, 44_100, 2, &frames);
    let mut src = Source::open(&path, &opts(44_100)).unwrap();
    let all = read_all(&mut src);
    assert!((all[100] - 0.5).abs() < 1e-3 && (all[101] + 0.25).abs() < 1e-3);
}

#[test]
fn missing_or_garbage_files_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let err = Source::open(&dir.path().join("absent.mp3"), &opts(44_100)).err().unwrap();
    assert!(format!("{err:#}").contains("introuvable"));
    let junk = dir.path().join("junk.mp3");
    std::fs::write(&junk, b"pas du son du tout").unwrap();
    assert!(Source::open(&junk, &opts(44_100)).is_err());
}

fn ffmpeg() -> Option<PathBuf> {
    control_core::process::which("ffmpeg")
}

#[test]
fn ffmpeg_parses_duration_lines() {
    assert_eq!(
        super::ffmpeg::parse_duration("  Duration: 00:03:25.12, start: 0.000000, bitrate: 320 kb/s"),
        Some(205.12)
    );
    assert_eq!(super::ffmpeg::parse_duration("  Duration: N/A, bitrate: N/A"), None);
    assert_eq!(super::ffmpeg::parse_duration("Stream #0:0: Audio: opus"), None);
}

#[test]
fn ffmpeg_pipe_reads_and_seeks() {
    let Some(exe) = ffmpeg() else {
        eprintln!("ffmpeg absent : test ignoré");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saw.wav");
    // En stéréo : ffmpeg atténue de 3 dB un mono dupliqué sur deux canaux, ce qui fausserait la lecture des valeurs.
    let stereo: Vec<f32> = saw(3 * 44_100).into_iter().flat_map(|s| [s, s]).collect();
    write_wav(&path, 44_100, 2, &stereo);
    let mut src = Source::open_ffmpeg(&path, &exe, 44_100).unwrap();
    assert_eq!(src.backend(), "ffmpeg");
    assert!((src.duration().unwrap() - 3.0).abs() < 0.02);
    let all = read_all(&mut src);
    assert!((all.len() as i64 / 2 - 3 * 44_100).abs() <= 2);
    check_seeks(&mut src, 44_100, 2);
}

#[test]
fn opus_goes_through_ffmpeg() {
    let Some(exe) = ffmpeg() else { return };
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("tone.wav");
    write_wav(&wav, 48_000, 1, &sine(440.0, 48_000, 2.0, 0.5));
    let opus = dir.path().join("tone.opus");
    let ok = std::process::Command::new(&exe)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&wav)
        .args(["-c:a", "libopus"])
        .arg(&opus)
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        eprintln!("pas d'encodeur Opus : test ignoré");
        return;
    }
    // Sans ffmpeg, symphonia ne sait pas décoder l'Opus.
    assert!(Source::open(&opus, &opts(48_000)).is_err());
    let mut src = Source::open(&opus, &OpenOptions { ffmpeg: Some(exe), sample_rate: 48_000 }).unwrap();
    assert_eq!(src.backend(), "ffmpeg");
    let l = left(&read_all(&mut src));
    assert!((l.len() as i64 - 96_000).abs() < 2_000, "{} trames", l.len());
    let f = zero_crossing_freq(&l[10_000..80_000], 48_000);
    assert!((f - 440.0).abs() < 2.0, "{f} Hz");
}

#[test]
fn common_formats_decode_with_symphonia() {
    let Some(exe) = ffmpeg() else { return };
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("tone.wav");
    write_wav(&wav, 44_100, 1, &sine(440.0, 44_100, 3.0, 0.5));
    for (ext, codec) in [("flac", "flac"), ("mp3", "libmp3lame"), ("ogg", "libvorbis"), ("m4a", "aac"), ("mka", "flac")]
    {
        let file = dir.path().join(format!("tone.{ext}"));
        let ok = std::process::Command::new(&exe)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&wav)
            .args(["-c:a", codec])
            .arg(&file)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            eprintln!("pas d'encodeur {codec} : {ext} ignoré");
            continue;
        }
        // Sans ffmpeg : symphonia seul doit savoir le lire.
        let mut src = Source::open(&file, &opts(48_000)).unwrap_or_else(|e| panic!("{ext} : {e:#}"));
        assert_eq!(src.backend(), "symphonia", "{ext}");
        let l = left(&read_all(&mut src));
        // Les encodeurs avec perte ajoutent parfois un peu de silence (délai de l'encodeur).
        assert!((l.len() as f64 / 48_000.0 - 3.0).abs() < 0.1, "{ext} : {} trames", l.len());
        let f = zero_crossing_freq(&l[10_000..120_000], 48_000);
        assert!((f - 440.0).abs() < 2.0, "{ext} : {f} Hz");
    }
}

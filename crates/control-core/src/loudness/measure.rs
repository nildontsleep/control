//! Mesure d'un morceau : niveau intégré (LUFS) et crête vraie (dBTP), par symphonia + `ebur128`, ou par le filtre
//! `ebur128` de ffmpeg pour les formats que symphonia ne lit pas.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use ebur128::{EbuR128, Mode};
use regex::Regex;

use super::decode::decode_file;

/// On ne mesure que les 10 premières minutes d'un long fichier.
const MEASURE_SECONDS: f64 = 600.0;
/// ffmpeg : au-delà, on abandonne le morceau.
const FFMPEG_TIMEOUT: Duration = Duration::from_secs(180);
const FFMPEG_MAX_OUTPUT: u64 = 4 * 1024 * 1024;

/// Niveau mesuré d'un morceau.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    /// LUFS intégrés.
    pub loudness: f64,
    /// dBTP (crête vraie, la plus haute des voies).
    pub true_peak: Option<f64>,
}

/// Compteur EBU R128 alimenté bloc par bloc.
struct Meter {
    meter: EbuR128,
    channels: usize,
}

impl Meter {
    fn new(channels: usize, rate: u32) -> Result<Meter> {
        let meter = EbuR128::new(channels as u32, rate, Mode::I | Mode::TRUE_PEAK)
            .map_err(|e| anyhow!("mesure impossible : {e:?}"))?;
        Ok(Meter { meter, channels })
    }

    fn add(&mut self, interleaved: &[f32]) -> Result<()> {
        self.meter.add_frames_f32(interleaved).map_err(|e| anyhow!("mesure impossible : {e:?}"))
    }

    /// `None` pour un morceau muet (ou trop court pour un bloc de mesure).
    fn level(&self) -> Option<Level> {
        let loudness = self.meter.loudness_global().ok().filter(|l| l.is_finite())?;
        let peak = (0..self.channels as u32).filter_map(|c| self.meter.true_peak(c).ok()).fold(0.0f64, f64::max);
        let true_peak = (peak > 0.0).then(|| 20.0 * peak.log10()).filter(|p| p.is_finite());
        Some(Level { loudness, true_peak })
    }
}

/// Mesure d'un signal déjà en mémoire (PCM f32 entrelacé).
pub fn measure_interleaved(interleaved: &[f32], channels: usize, rate: u32) -> Option<Level> {
    if channels == 0 || rate == 0 {
        return None;
    }
    let mut meter = Meter::new(channels, rate).ok()?;
    meter.add(interleaved).ok()?;
    meter.level()
}

/// Mesure d'un fichier : `None` s'il est muet, illisible, ou si `cancel` a demandé l'arrêt. Les fichiers que
/// symphonia ne sait pas lire passent par ffmpeg quand il est fourni.
pub fn measure_file(path: &Path, ffmpeg: Option<&Path>, cancel: &dyn Fn() -> bool) -> Option<Level> {
    match measure_symphonia(path, cancel) {
        Ok(level) => level,
        Err(err) => {
            log::debug!("Mesure du volume par le décodeur intégré impossible ({}) : {err}", path.display());
            let ffmpeg = ffmpeg?;
            match measure_ffmpeg(ffmpeg, path, cancel) {
                Ok(level) => level,
                Err(err) => {
                    log::debug!("Mesure du volume par ffmpeg impossible ({}) : {err}", path.display());
                    None
                }
            }
        }
    }
}

fn measure_symphonia(path: &Path, cancel: &dyn Fn() -> bool) -> Result<Option<Level>> {
    let mut meter: Option<Meter> = None;
    let mut failure: Option<anyhow::Error> = None;
    decode_file(path, Some(MEASURE_SECONDS), |format, samples| {
        if cancel() {
            return false;
        }
        let m = match &mut meter {
            Some(m) => m,
            None => match Meter::new(format.channels, format.rate) {
                Ok(m) => meter.insert(m),
                Err(err) => {
                    failure = Some(err);
                    return false;
                }
            },
        };
        if let Err(err) = m.add(samples) {
            failure = Some(err);
            return false;
        }
        true
    })?;
    if let Some(err) = failure {
        return Err(err);
    }
    if cancel() {
        return Ok(None);
    }
    Ok(meter.and_then(|m| m.level()))
}

/// Mesure par le filtre `ebur128` de ffmpeg (mêmes options que la version d'origine).
pub(super) fn measure_ffmpeg(ffmpeg: &Path, file: &Path, cancel: &dyn Fn() -> bool) -> Result<Option<Level>> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-nostdin", "-hide_banner", "-nostats", "-t"])
        .arg(format!("{MEASURE_SECONDS}"))
        .arg("-i")
        .arg(file)
        .args(["-vn", "-af", "ebur128=peak=true:framelog=quiet", "-f", "null", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    crate::process::hide_window(&mut cmd);
    let mut child = cmd.spawn()?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("sortie de ffmpeg indisponible"))?;
    // Lue à part : un tuyau plein bloquerait ffmpeg.
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let mut limited = stderr.take(FFMPEG_MAX_OUTPUT);
        let _ = limited.read_to_string(&mut text);
        let _ = std::io::copy(&mut limited.into_inner(), &mut std::io::sink());
        text
    });
    let started = Instant::now();
    let mut stopped = false;
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if cancel() || started.elapsed() > FFMPEG_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            stopped = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let text = reader.join().unwrap_or_default();
    if stopped && cancel() {
        return Ok(None);
    }
    Ok(parse_summary(&text))
}

static LOUDNESS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"I:\s+(-?\d+(?:\.\d+)?)\s+LUFS").expect("regex constante"));
static PEAK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Peak:\s+(-?\d+(?:\.\d+)?)\s+dBFS").expect("regex constante"));

/// Lit le résumé de ffmpeg (stderr) : la dernière valeur de chaque mesure. `None` si le morceau est muet (`-inf`).
pub fn parse_summary(text: &str) -> Option<Level> {
    let last = |re: &Regex| re.captures_iter(text).last().and_then(|c| c[1].parse::<f64>().ok());
    let loudness = last(&LOUDNESS_RE).filter(|l| l.is_finite())?;
    Some(Level { loudness, true_peak: last(&PEAK_RE).filter(|p| p.is_finite()) })
}

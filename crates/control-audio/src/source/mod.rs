//! Sources audio : un fichier décodé (symphonia, ou ffmpeg en secours), converti en stéréo à la fréquence de la
//! sortie. Tout se passe sur le thread du mélangeur ou d'un chargeur, jamais dans le rappel de la carte son.

mod decode;
mod ffmpeg;
mod resample;

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

/// Résultat d'un décodage élémentaire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Chunk {
    /// Des trames ont été ajoutées.
    Data,
    /// Rien pour l'instant (ffmpeg n'a pas encore écrit) : réessayer plus tard.
    Pending,
    /// Fin du fichier.
    End,
}

/// Décodeur à la fréquence propre du fichier, sortie stéréo entrelacée.
pub(crate) trait Decoder: Send {
    fn next_chunk(&mut self, out: &mut Vec<f32>) -> Result<Chunk>;
    /// Déplacement ; rend l'instant effectivement atteint (s).
    fn seek(&mut self, secs: f64) -> Result<f64>;
    fn sample_rate(&self) -> u32;
    fn duration(&self) -> Option<f64>;
}

/// Réduction de n canaux vers deux : gains (gauche, droite) de chaque canal, normalisés pour ne pas saturer.
#[derive(Debug, Clone, Default)]
pub(crate) struct StereoMap {
    gains: Option<Vec<(f32, f32)>>,
}

impl StereoMap {
    fn matrix(gains: Vec<(f32, f32)>) -> StereoMap {
        let l: f32 = gains.iter().map(|g| g.0).sum();
        let r: f32 = gains.iter().map(|g| g.1).sum();
        let norm = 1.0 / l.max(r).max(1.0);
        StereoMap { gains: Some(gains.into_iter().map(|(a, b)| (a * norm, b * norm)).collect()) }
    }

    /// Ajoute à `out` les trames de `samples` (entrelacés sur `channels` canaux) en stéréo.
    fn convert(&self, samples: &[f32], channels: usize, out: &mut Vec<f32>) {
        match (channels, &self.gains) {
            (1, _) => samples.iter().for_each(|&s| out.extend_from_slice(&[s, s])),
            (2, _) | (_, None) => {
                for f in samples.chunks_exact(channels) {
                    out.push(f[0]);
                    out.push(*f.get(1).unwrap_or(&f[0]));
                }
            }
            (_, Some(gains)) => {
                for f in samples.chunks_exact(channels) {
                    let (mut l, mut r) = (0.0, 0.0);
                    for (s, g) in f.iter().zip(gains) {
                        l += s * g.0;
                        r += s * g.1;
                    }
                    out.push(l);
                    out.push(r);
                }
            }
        }
    }
}

/// Lecture d'une source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Read {
    /// Trames écrites (au moins une).
    Frames(usize),
    /// Pas de données pour l'instant.
    Pending,
    /// Fin.
    End,
}

/// Comment ouvrir les fichiers.
#[derive(Debug, Clone)]
pub(crate) struct OpenOptions {
    pub ffmpeg: Option<PathBuf>,
    pub sample_rate: u32,
}

/// Extensions que symphonia ne sait pas lire (ou dont l'audio est souvent dans un codec qu'il ignore) : ffmpeg d'abord.
const FFMPEG_FIRST: &[&str] = &[
    "opus", "webm", "weba", "wma", "wmv", "asf", "avi", "flv", "ape", "ac3", "eac3", "dts", "mpg", "mpeg", "ts",
    "m2ts", "mts", "3gp", "amr", "mka",
];

/// Un fichier ouvert, prêt à lire en stéréo à la fréquence de la sortie.
pub struct Source {
    dec: Box<dyn Decoder>,
    resampler: Option<resample::Resampler>,
    fifo: Vec<f32>,
    pos: usize,
    scratch: Vec<f32>,
    ended: bool,
    backend: &'static str,
}

impl Source {
    pub(crate) fn open(path: &Path, opts: &OpenOptions) -> Result<Source> {
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        if !path.is_file() {
            bail!("Fichier introuvable : {}", path.display());
        }
        let ffmpeg_first = FFMPEG_FIRST.contains(&ext.as_str());
        let mut first_error = None;
        if !ffmpeg_first {
            match decode::SymphoniaDecoder::open(path) {
                Ok(dec) => return Source::new(Box::new(dec), opts.sample_rate, "symphonia"),
                Err(e) => first_error = Some(e),
            }
        }
        if let Some(exe) = &opts.ffmpeg {
            match ffmpeg::FfmpegDecoder::open(exe, path, opts.sample_rate) {
                Ok(dec) => return Source::new(Box::new(dec), opts.sample_rate, "ffmpeg"),
                Err(e) => {
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                }
            }
        } else if ffmpeg_first {
            // Sans ffmpeg, on tente quand même symphonia (un .mka en flac, par exemple).
            match decode::SymphoniaDecoder::open(path) {
                Ok(dec) => return Source::new(Box::new(dec), opts.sample_rate, "symphonia"),
                Err(e) => first_error = Some(e),
            }
        }
        match first_error {
            Some(e) => bail!("Lecture impossible : {e:#}"),
            None => bail!("Lecture impossible"),
        }
    }

    /// Force ffmpeg (tests).
    #[cfg(test)]
    pub(crate) fn open_ffmpeg(path: &Path, exe: &Path, sample_rate: u32) -> Result<Source> {
        Source::new(Box::new(ffmpeg::FfmpegDecoder::open(exe, path, sample_rate)?), sample_rate, "ffmpeg")
    }

    fn new(dec: Box<dyn Decoder>, rate: u32, backend: &'static str) -> Result<Source> {
        let native = dec.sample_rate();
        let resampler = if native != rate { Some(resample::Resampler::new(native, rate)?) } else { None };
        Ok(Source {
            dec,
            resampler,
            fifo: Vec::with_capacity(16384),
            pos: 0,
            scratch: Vec::with_capacity(16384),
            ended: false,
            backend,
        })
    }

    /// Décodeur utilisé (« symphonia » ou « ffmpeg »).
    pub fn backend(&self) -> &'static str {
        self.backend
    }

    pub fn duration(&self) -> Option<f64> {
        self.dec.duration()
    }

    /// Lit des trames stéréo entrelacées dans `out`.
    pub(crate) fn read(&mut self, out: &mut [f32]) -> Result<Read> {
        let want = out.len();
        let mut done = 0;
        loop {
            let avail = self.fifo.len() - self.pos;
            if avail > 0 {
                let n = avail.min(want - done);
                out[done..done + n].copy_from_slice(&self.fifo[self.pos..self.pos + n]);
                self.pos += n;
                done += n;
                if self.pos == self.fifo.len() {
                    self.fifo.clear();
                    self.pos = 0;
                }
            }
            if done == want {
                return Ok(Read::Frames(done / 2));
            }
            if self.ended {
                return Ok(if done > 0 { Read::Frames(done / 2) } else { Read::End });
            }
            if self.pos > 0 {
                self.fifo.drain(..self.pos);
                self.pos = 0;
            }
            match self.fill()? {
                Chunk::Data | Chunk::End => {}
                Chunk::Pending => return Ok(if done > 0 { Read::Frames(done / 2) } else { Read::Pending }),
            }
        }
    }

    fn fill(&mut self) -> Result<Chunk> {
        let Some(rs) = self.resampler.as_mut() else {
            let c = self.dec.next_chunk(&mut self.fifo)?;
            if c == Chunk::End {
                self.ended = true;
            }
            return Ok(c);
        };
        self.scratch.clear();
        let c = self.dec.next_chunk(&mut self.scratch)?;
        match c {
            Chunk::Data => rs.push(&self.scratch, &mut self.fifo)?,
            Chunk::End => {
                rs.finish(&mut self.fifo)?;
                self.ended = true;
            }
            Chunk::Pending => {}
        }
        Ok(c)
    }

    /// Déplacement (s) ; rend l'instant atteint.
    pub(crate) fn seek(&mut self, secs: f64) -> Result<f64> {
        let at = self.dec.seek(secs)?;
        self.fifo.clear();
        self.pos = 0;
        self.ended = false;
        if let Some(rs) = self.resampler.as_mut() {
            rs.reset();
        }
        Ok(at)
    }
}

#[cfg(test)]
mod tests;

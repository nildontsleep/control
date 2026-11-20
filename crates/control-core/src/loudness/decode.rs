//! Décodage d'un fichier audio en PCM f32 entrelacé (symphonia), bloc par bloc, pour les analyses en arrière-plan
//! (niveau sonore, AutoMix). Rien n'est gardé en mémoire ici : chaque bloc part chez l'appelant.

use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Format du PCM rendu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    pub channels: usize,
    pub rate: u32,
}

/// Décode `path` et passe chaque bloc entrelacé à `sink`, jusqu'à la fin du fichier, `max_seconds` de son, ou un
/// `false` rendu par `sink` (arrêt demandé). Rend le format du son et le nombre d'images décodées.
///
/// Une erreur au milieu du fichier (fin abîmée) arrête le décodage sans échec si du son a déjà été lu ; un fichier
/// dont le conteneur ou le codec n'est pas pris en charge (Opus, par exemple) est une erreur.
pub fn decode_file(
    path: &Path,
    max_seconds: Option<f64>,
    mut sink: impl FnMut(Format, &[f32]) -> bool,
) -> Result<(Format, u64)> {
    let file = File::open(path).with_context(|| format!("ouverture de {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| anyhow!("format non reconnu : {e}"))?;
    let mut reader = probed.format;
    let track = reader
        .default_track()
        .filter(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .or_else(|| reader.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL))
        .ok_or_else(|| anyhow!("aucune piste audio"))?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| anyhow!("codec non pris en charge : {e}"))?;

    let mut format: Option<Format> = None;
    let mut frames: u64 = 0;
    let mut buf: Option<SampleBuffer<f32>> = None;
    let mut last_err: Option<String> = None;
    loop {
        let packet = match reader.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) if format.is_some() => {
                log::debug!("décodage interrompu ({}) : {e}", path.display());
                break;
            }
            Err(e) => return Err(anyhow!("lecture impossible : {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // Un paquet abîmé : on passe au suivant, comme un lecteur.
            Err(SymError::DecodeError(e)) => {
                last_err = Some(e.to_string());
                continue;
            }
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) if format.is_some() => {
                log::debug!("décodage interrompu ({}) : {e}", path.display());
                break;
            }
            Err(e) => return Err(anyhow!("décodage impossible : {e}")),
        };
        let spec = *decoded.spec();
        let here = Format { channels: spec.channels.count(), rate: spec.rate };
        if here.channels == 0 || here.rate == 0 {
            continue;
        }
        match format {
            None => format = Some(here),
            // Un changement de format en cours de route (rare, flux concaténés) : on s'arrête à ce qui est lu.
            Some(f) if f != here => break,
            Some(_) => {}
        }
        let n = decoded.frames();
        if n == 0 {
            continue;
        }
        let sb = match &mut buf {
            Some(b) if b.capacity() >= decoded.capacity() => b,
            _ => buf.insert(SampleBuffer::new(decoded.capacity() as u64, spec)),
        };
        sb.copy_interleaved_ref(decoded);
        let mut samples = sb.samples();
        let mut stop = false;
        if let Some(max) = max_seconds {
            let limit = (max * here.rate as f64) as u64;
            let left = limit.saturating_sub(frames) as usize;
            if left <= n {
                samples = &samples[..left * here.channels];
                stop = true;
            }
        }
        frames += (samples.len() / here.channels) as u64;
        if !samples.is_empty() && !sink(here, samples) {
            break;
        }
        if stop {
            break;
        }
    }
    match format {
        Some(f) => Ok((f, frames)),
        None => match last_err {
            Some(e) => Err(anyhow!("décodage impossible : {e}")),
            None => Err(anyhow!("aucun son dans le fichier")),
        },
    }
}

/// Écrit un WAV PCM 16 bits (tests : de quoi faire passer un signal synthétique par le vrai décodeur).
#[cfg(test)]
pub(crate) fn write_wav(path: &Path, channels: u16, rate: u32, interleaved: &[f32]) {
    let data_len = (interleaved.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in interleaved {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, out).expect("écriture du WAV de test");
}

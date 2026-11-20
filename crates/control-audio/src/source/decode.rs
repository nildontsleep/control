//! Décodage avec symphonia (mp3, flac, aac/m4a, alac, ogg vorbis, wav, et l'audio de mkv quand le codec est connu).

use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use symphonia::core::audio::{Channels, SampleBuffer, SignalSpec};
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::{Time, TimeBase};

use super::{Chunk, Decoder, StereoMap};

pub(crate) struct SymphoniaDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track: u32,
    time_base: Option<TimeBase>,
    rate: u32,
    duration: Option<f64>,
    spec: Option<SignalSpec>,
    map: StereoMap,
    buf: Option<SampleBuffer<f32>>,
    /// Après un déplacement : instant (s) avant lequel les échantillons décodés sont jetés (déplacement exact).
    skip_until: Option<f64>,
    /// Premier paquet, décodé à l'ouverture pour vérifier que le codec est vraiment pris en charge.
    primed: Vec<f32>,
}

impl SymphoniaDecoder {
    pub fn open(path: &Path) -> Result<SymphoniaDecoder> {
        let file = File::open(path).with_context(|| format!("Impossible d'ouvrir {}", path.display()))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(&ext.to_ascii_lowercase());
        }
        let opts = FormatOptions { enable_gapless: true, ..Default::default() };
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &opts, &MetadataOptions::default())
            .map_err(|e| anyhow!("format non reconnu : {e}"))?;
        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some())
            .or_else(|| format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL))
            .ok_or_else(|| anyhow!("aucune piste audio"))?;
        let params = track.codec_params.clone();
        let track_id = track.id;
        let decoder = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())
            .map_err(|e| anyhow!("codec non pris en charge : {e}"))?;
        let duration = match (params.n_frames, params.sample_rate, params.time_base) {
            (Some(n), Some(sr), _) if sr > 0 => Some(n as f64 / sr as f64),
            (Some(n), None, Some(tb)) => Some(time_secs(tb.calc_time(n))),
            _ => None,
        };
        let mut dec = SymphoniaDecoder {
            format,
            decoder,
            track: track_id,
            time_base: params.time_base,
            rate: params.sample_rate.unwrap_or(0),
            duration,
            spec: None,
            map: StereoMap::default(),
            buf: None,
            skip_until: None,
            primed: Vec::new(),
        };
        let mut first = Vec::new();
        match dec.decode_packet(&mut first)? {
            Chunk::End if first.is_empty() => bail!("fichier sans audio"),
            _ => {}
        }
        if dec.rate == 0 {
            bail!("fréquence d'échantillonnage inconnue");
        }
        dec.primed = first;
        Ok(dec)
    }

    /// Décode le prochain paquet de la piste ; ajoute des trames stéréo entrelacées à `out`.
    fn decode_packet(&mut self, out: &mut Vec<f32>) -> Result<Chunk> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(Chunk::End),
                Err(SymError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(e) => return Err(anyhow!("lecture du fichier : {e}")),
            };
            if packet.track_id() != self.track {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                // Un paquet abîmé : on le saute, comme le ferait un lecteur ordinaire.
                Err(SymError::DecodeError(e)) => {
                    log::debug!("paquet illisible ignoré : {e}");
                    continue;
                }
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(Chunk::End),
                Err(e) => return Err(anyhow!("décodage : {e}")),
            };
            let spec = *decoded.spec();
            let frames = decoded.frames();
            if frames == 0 {
                continue;
            }
            if self.spec != Some(spec) {
                self.spec = Some(spec);
                self.rate = spec.rate;
                self.map = stereo_map(spec.channels);
                self.buf = None;
            }
            if self.buf.as_ref().is_none_or(|b| b.capacity() < frames) {
                self.buf = Some(SampleBuffer::new(decoded.capacity().max(frames) as u64, spec));
            }
            let Some(buf) = self.buf.as_mut() else { continue };
            buf.copy_interleaved_ref(decoded);
            let samples = buf.samples();
            let channels = spec.channels.count().max(1);
            // Déplacement exact : on jette ce qui précède l'instant demandé.
            let mut skip = 0usize;
            if let Some(until) = self.skip_until {
                let at = self
                    .time_base
                    .map_or(packet.ts() as f64 / self.rate as f64, |tb| time_secs(tb.calc_time(packet.ts())));
                let behind = ((until - at) * self.rate as f64).round();
                if behind >= frames as f64 {
                    continue;
                }
                skip = behind.max(0.0) as usize;
                self.skip_until = None;
            }
            self.map.convert(&samples[skip * channels..frames * channels], channels, out);
            return Ok(Chunk::Data);
        }
    }
}

fn time_secs(t: Time) -> f64 {
    t.seconds as f64 + t.frac
}

/// Matrice de réduction vers la stéréo d'après la disposition des canaux.
fn stereo_map(channels: Channels) -> StereoMap {
    let n = channels.count();
    if n <= 2 {
        return StereoMap::default();
    }
    const H: f32 = std::f32::consts::FRAC_1_SQRT_2;
    let left = Channels::REAR_LEFT
        | Channels::SIDE_LEFT
        | Channels::FRONT_LEFT_CENTRE
        | Channels::REAR_LEFT_CENTRE
        | Channels::FRONT_LEFT_WIDE
        | Channels::FRONT_LEFT_HIGH
        | Channels::TOP_FRONT_LEFT
        | Channels::TOP_REAR_LEFT;
    let right = Channels::REAR_RIGHT
        | Channels::SIDE_RIGHT
        | Channels::FRONT_RIGHT_CENTRE
        | Channels::REAR_RIGHT_CENTRE
        | Channels::FRONT_RIGHT_WIDE
        | Channels::FRONT_RIGHT_HIGH
        | Channels::TOP_FRONT_RIGHT
        | Channels::TOP_REAR_RIGHT;
    let gains: Vec<(f32, f32)> = channels
        .iter()
        .map(|c| {
            if c == Channels::FRONT_LEFT {
                (1.0, 0.0)
            } else if c == Channels::FRONT_RIGHT {
                (0.0, 1.0)
            } else if c == Channels::LFE1 || c == Channels::LFE2 {
                (0.0, 0.0)
            } else if left.contains(c) {
                (H, 0.0)
            } else if right.contains(c) {
                (0.0, H)
            } else {
                (H, H)
            }
        })
        .collect();
    StereoMap::matrix(gains)
}

impl Decoder for SymphoniaDecoder {
    fn next_chunk(&mut self, out: &mut Vec<f32>) -> Result<Chunk> {
        if !self.primed.is_empty() {
            out.append(&mut self.primed);
            return Ok(Chunk::Data);
        }
        self.decode_packet(out)
    }

    fn seek(&mut self, secs: f64) -> Result<f64> {
        self.primed.clear();
        let secs = secs.max(0.0);
        let to = SeekTo::Time { time: Time::from(secs), track_id: Some(self.track) };
        match self.format.seek(SeekMode::Accurate, to) {
            Ok(_) => {
                self.decoder.reset();
                self.skip_until = Some(secs);
                Ok(secs)
            }
            // Au-delà de la fin : la lecture s'arrêtera aussitôt.
            Err(SymError::SeekError(_)) if self.duration.is_some_and(|d| secs >= d) => {
                self.skip_until = Some(f64::MAX);
                Ok(secs)
            }
            Err(e) => Err(anyhow!("déplacement impossible : {e}")),
        }
    }

    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn duration(&self) -> Option<f64> {
        self.duration
    }
}

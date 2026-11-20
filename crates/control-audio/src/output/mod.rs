//! Sortie : la carte son (cpal) ou, sans carte (intégration continue, serveur), une sortie muette cadencée par
//! une horloge, pour que les positions avancent comme en vrai.

pub(crate) mod ring;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

use ring::{Consumer, Producer};

/// Capacité du tampon : un peu plus d'une demi-seconde de stéréo.
fn ring_capacity(sample_rate: u32) -> usize {
    sample_rate as usize
}

/// Sortie en marche ; l'arrêter (drop) libère la carte son.
pub(crate) struct Output {
    pub sample_rate: u32,
    /// Nom de la carte son, `None` pour la sortie muette.
    pub device: Option<String>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Output {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

/// Ouvre la carte son par défaut. Le flux cpal n'est pas `Send` partout : il vit sur son propre thread.
pub(crate) fn open_device() -> Result<(Output, Producer)> {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = crossbeam_channel::bounded::<Result<(u32, String, Producer)>>(1);
    let stop2 = Arc::clone(&stop);
    let thread = std::thread::Builder::new().name("control-audio-output".into()).spawn(move || {
        let stream = match build_stream() {
            Ok((stream, rate, name, producer)) => {
                let _ = tx.send(Ok((rate, name, producer)));
                stream
            }
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        while !stop2.load(Ordering::Acquire) {
            std::thread::park_timeout(Duration::from_millis(250));
        }
        drop(stream);
    })?;
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok((sample_rate, name, producer))) => {
            Ok((Output { sample_rate, device: Some(name), stop, thread: Some(thread) }, producer))
        }
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => {
            // Pilote bloqué : on l'abandonne (le thread s'arrêtera de lui-même s'il finit par répondre).
            stop.store(true, Ordering::Release);
            Err(anyhow!("la carte son ne répond pas"))
        }
    }
}

fn build_stream() -> Result<(cpal::Stream, u32, String, Producer)> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or_else(|| anyhow!("aucune sortie audio"))?;
    let name = device.name().unwrap_or_else(|_| "sortie par défaut".into());
    let supported = device.default_output_config().map_err(|e| anyhow!("sortie audio inutilisable : {e}"))?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate.0;
    let (producer, consumer) = ring::ring(ring_capacity(rate));
    let stream = match format {
        cpal::SampleFormat::F32 => stream::<f32>(&device, &config, consumer),
        cpal::SampleFormat::I16 => stream::<i16>(&device, &config, consumer),
        cpal::SampleFormat::U16 => stream::<u16>(&device, &config, consumer),
        cpal::SampleFormat::I32 => stream::<i32>(&device, &config, consumer),
        cpal::SampleFormat::U32 => stream::<u32>(&device, &config, consumer),
        cpal::SampleFormat::F64 => stream::<f64>(&device, &config, consumer),
        cpal::SampleFormat::I8 => stream::<i8>(&device, &config, consumer),
        cpal::SampleFormat::U8 => stream::<u8>(&device, &config, consumer),
        other => Err(anyhow!("format d'échantillons non pris en charge : {other}")),
    }?;
    stream.play().map_err(|e| anyhow!("démarrage de la sortie audio : {e}"))?;
    Ok((stream, rate, name, producer))
}

/// Rappel de la carte son : il ne fait que lire le tampon (ni verrou, ni allocation, ni appel système).
fn stream<T>(device: &cpal::Device, config: &cpal::StreamConfig, mut consumer: Consumer) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels.max(1) as usize;
    let mut tmp = [0f32; 1024];
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                for chunk in data.chunks_mut(channels * 512) {
                    let frames = chunk.len() / channels;
                    consumer.pop(&mut tmp[..frames * 2]);
                    for (f, out) in chunk.chunks_mut(channels).enumerate() {
                        let (l, r) = (tmp[f * 2], tmp[f * 2 + 1]);
                        if channels == 1 {
                            out[0] = T::from_sample((l + r) * 0.5);
                        } else {
                            out[0] = T::from_sample(l);
                            out[1] = T::from_sample(r);
                            out[2..].iter_mut().for_each(|o| *o = T::EQUILIBRIUM);
                        }
                    }
                }
            },
            |err| log::warn!("sortie audio : {err}"),
            None,
        )
        .map_err(|e| anyhow!("ouverture de la sortie audio : {e}"))
}

/// Sortie muette : un thread consomme le tampon au rythme de l'horloge, et jette le son.
pub(crate) fn open_null(sample_rate: u32) -> Result<(Output, Producer)> {
    let stop = Arc::new(AtomicBool::new(false));
    let (producer, mut consumer) = ring::ring(ring_capacity(sample_rate));
    let stop2 = Arc::clone(&stop);
    let thread = std::thread::Builder::new().name("control-audio-null".into()).spawn(move || {
        let mut scratch = vec![0f32; 8192];
        let start = Instant::now();
        let mut consumed: u64 = 0;
        while !stop2.load(Ordering::Acquire) {
            std::thread::park_timeout(Duration::from_millis(5));
            let due = (start.elapsed().as_secs_f64() * sample_rate as f64) as u64;
            while consumed < due {
                let frames = ((due - consumed) as usize).min(scratch.len() / 2);
                consumer.pop(&mut scratch[..frames * 2]);
                consumed += frames as u64;
            }
        }
    })?;
    Ok((Output { sample_rate, device: None, stop, thread: Some(thread) }, producer))
}

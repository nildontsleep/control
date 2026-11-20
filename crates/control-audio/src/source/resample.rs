//! Rééchantillonnage vers la fréquence de la sortie (rubato, par FFT : rapport fixe, très bonne qualité).

use anyhow::{Result, anyhow};
use rubato::{FftFixedIn, Resampler as _};

const CHUNK: usize = 1024;

pub(crate) struct Resampler {
    inner: FftFixedIn<f32>,
    input: [Vec<f32>; 2],
    output: Vec<Vec<f32>>,
    /// Trames de sortie encore à jeter (retard propre au filtre, après une remise à zéro).
    skip: usize,
    in_total: u64,
    out_total: u64,
    ratio: f64,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Result<Resampler> {
        let inner = FftFixedIn::<f32>::new(from as usize, to as usize, CHUNK, 2, 2)
            .map_err(|e| anyhow!("rééchantillonnage {from} → {to} Hz impossible : {e}"))?;
        let output = inner.output_buffer_allocate(true);
        let skip = inner.output_delay();
        Ok(Resampler {
            inner,
            input: [Vec::with_capacity(CHUNK), Vec::with_capacity(CHUNK)],
            output,
            skip,
            in_total: 0,
            out_total: 0,
            ratio: to as f64 / from as f64,
        })
    }

    pub fn reset(&mut self) {
        self.inner.reset();
        self.input.iter_mut().for_each(Vec::clear);
        self.skip = self.inner.output_delay();
        self.in_total = 0;
        self.out_total = 0;
    }

    /// Ajoute de l'entrée stéréo entrelacée ; la sortie disponible est ajoutée à `out`.
    pub fn push(&mut self, interleaved: &[f32], out: &mut Vec<f32>) -> Result<()> {
        let mut frames = interleaved.as_chunks::<2>().0.iter();
        loop {
            let need = self.inner.input_frames_next() - self.input[0].len();
            let mut taken = 0;
            for f in frames.by_ref().take(need) {
                self.input[0].push(f[0]);
                self.input[1].push(f[1]);
                taken += 1;
            }
            self.in_total += taken as u64;
            if self.input[0].len() < self.inner.input_frames_next() {
                return Ok(());
            }
            self.run(out, None)?;
        }
    }

    fn run(&mut self, out: &mut Vec<f32>, limit: Option<u64>) -> Result<()> {
        let (_, produced) = self
            .inner
            .process_into_buffer(&self.input, &mut self.output, None)
            .map_err(|e| anyhow!("rééchantillonnage : {e}"))?;
        self.input.iter_mut().for_each(Vec::clear);
        let start = self.skip.min(produced);
        self.skip -= start;
        let mut end = produced;
        if let Some(limit) = limit {
            let room = limit.saturating_sub(self.out_total) as usize;
            end = end.min(start + room);
        }
        for i in start..end {
            out.push(self.output[0][i]);
            out.push(self.output[1][i]);
        }
        self.out_total += (end - start) as u64;
        Ok(())
    }

    /// Fin de l'entrée : vide le filtre, en s'arrêtant exactement à la durée attendue.
    pub fn finish(&mut self, out: &mut Vec<f32>) -> Result<()> {
        let expected = (self.in_total as f64 * self.ratio).round() as u64;
        let mut guard = 0;
        while self.out_total < expected && guard < 64 {
            let need = self.inner.input_frames_next();
            for ch in self.input.iter_mut() {
                ch.resize(need, 0.0);
            }
            self.run(out, Some(expected))?;
            guard += 1;
        }
        Ok(())
    }
}

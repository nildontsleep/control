//! Une platine : un morceau ouvert, sa vitesse (hauteur conservée), son gain de normalisation, son filtre de graves
//! (échange des basses d'AutoMix) et son gain de fondu.

use control_core::model::TrackId;

use crate::automix::BASS_FREQ;
use crate::dsp::biquad::{Biquad, Coeffs, FilterKind};
use crate::dsp::gain::{Smoothed, db_to_linear};
use crate::dsp::stretch::TimeStretch;
use crate::source::{Read, Source};

/// Trames lues à la fois dans la source.
const READ_FRAMES: usize = 2048;
/// Sous-blocs du filtre de graves pendant une rampe.
const BASS_STEP: usize = 64;

/// Un morceau ouvert.
pub(crate) struct Loaded {
    pub id: TrackId,
    pub source: Source,
}

/// Ce qu'a donné le rendu d'un bloc.
#[derive(Debug, Default)]
pub(crate) struct Rendered {
    /// Trames de son produites (le reste du bloc est du silence).
    pub frames: usize,
    /// La source est épuisée et tout a été rendu.
    pub ended: bool,
    pub error: Option<String>,
}

pub(crate) struct Deck {
    pub track: Option<Loaded>,
    /// La platine joue (sinon elle attend, par exemple calée avant un mixage).
    pub rendering: bool,
    pub ended: bool,
    /// Vitesse absolue (1 = normale).
    pub speed: f64,
    /// Gain de fondu au début et à la fin du prochain bloc (interpolé entre les deux).
    pub fade: f32,
    pub fade_end: f32,
    /// Gain des graves visé à la fin du prochain bloc (dB).
    pub bass_target: f64,
    bass_db: f64,
    bass: Biquad,
    norm: Smoothed,
    stretch: TimeStretch,
    /// Instant (s) de la source à la dernière remise à zéro de l'étireur.
    base: f64,
    duration: Option<f64>,
    sr: f64,
    input: Vec<f32>,
    out: Vec<f32>,
}

impl Deck {
    pub fn new(sample_rate: u32) -> Deck {
        Deck {
            track: None,
            rendering: false,
            ended: false,
            speed: 1.0,
            fade: 1.0,
            fade_end: 1.0,
            bass_target: 0.0,
            bass_db: 0.0,
            bass: Biquad::new(Coeffs::IDENTITY),
            norm: Smoothed::new(1.0, (sample_rate / 20).max(1)),
            stretch: TimeStretch::new(sample_rate),
            base: 0.0,
            duration: None,
            sr: sample_rate as f64,
            input: vec![0.0; READ_FRAMES * 2],
            out: vec![0.0; super::BLOCK * 2],
        }
    }

    pub fn id(&self) -> Option<TrackId> {
        self.track.as_ref().map(|t| t.id)
    }

    pub fn duration(&self) -> Option<f64> {
        self.duration
    }

    /// Position (s) dans le morceau du prochain échantillon rendu.
    pub fn position(&self) -> f64 {
        let p = self.base + self.stretch.position() / self.sr;
        match self.duration {
            Some(d) => p.min(d),
            None => p,
        }
    }

    /// Prend un morceau ; `at` : instant où sa source est déjà positionnée.
    pub fn attach(&mut self, loaded: Loaded, at: f64, speed: f64, gain_db: f32) {
        self.duration = loaded.source.duration();
        self.track = Some(loaded);
        self.speed = speed;
        self.base = at;
        self.stretch.reset(speed);
        self.ended = false;
        self.rendering = false;
        self.fade = 1.0;
        self.fade_end = 1.0;
        self.set_bass_now(0.0);
        self.norm.set_now(db_to_linear(gain_db));
    }

    /// Rend le morceau (pour le remettre de côté).
    pub fn take(&mut self) -> Option<Loaded> {
        self.rendering = false;
        self.ended = false;
        self.duration = None;
        self.track.take()
    }

    pub fn unload(&mut self) {
        drop(self.take());
        self.fade = 1.0;
        self.fade_end = 1.0;
        self.set_bass_now(0.0);
    }

    pub fn set_gain_db(&mut self, db: f32) {
        self.norm.set_target(db_to_linear(db));
    }

    pub fn set_bass_now(&mut self, db: f64) {
        self.bass_db = db;
        self.bass_target = db;
        self.bass.set(Coeffs::new(FilterKind::LowShelf, self.sr, BASS_FREQ, 0.707, db));
        if db == 0.0 {
            self.bass.reset();
        }
    }

    /// Déplacement (s) dans le morceau.
    pub fn seek(&mut self, secs: f64) -> Result<(), String> {
        let Some(t) = self.track.as_mut() else { return Ok(()) };
        let secs = match self.duration {
            Some(d) => secs.clamp(0.0, d),
            None => secs.max(0.0),
        };
        let at = t.source.seek(secs).map_err(|e| format!("{e:#}"))?;
        self.base = at;
        self.stretch.reset(self.speed);
        self.ended = false;
        Ok(())
    }

    /// Rend jusqu'à `out.len() / 2` trames et les *ajoute* à `out` (stéréo entrelacé).
    pub fn render(&mut self, out: &mut [f32]) -> Rendered {
        let n = out.len() / 2;
        let mut r = Rendered::default();
        let Some(track) = self.track.as_mut() else {
            r.ended = true;
            return r;
        };
        if self.ended {
            r.ended = true;
            return r;
        }
        let buf = &mut self.out[..n * 2];
        let mut got = 0;
        while got < n {
            got += self.stretch.pull(&mut buf[got * 2..], self.speed);
            if got == n {
                break;
            }
            if self.stretch.is_done() {
                self.ended = true;
                r.ended = true;
                break;
            }
            match track.source.read(&mut self.input) {
                Ok(Read::Frames(m)) => self.stretch.push(&self.input[..m * 2]),
                Ok(Read::End) => self.stretch.finish(),
                // La source n'a rien pour l'instant : le reste du bloc est silencieux, la position n'avance pas.
                Ok(Read::Pending) => break,
                Err(e) => {
                    r.error = Some(format!("{e:#}"));
                    self.ended = true;
                    r.ended = true;
                    break;
                }
            }
        }
        r.frames = got;
        let buf = &mut self.out[..got * 2];
        self.norm.apply(buf);
        // Échange des basses : coefficients mis à jour par petits pas pendant une rampe.
        if self.bass_db != 0.0 || self.bass_target != 0.0 {
            let (from, to) = (self.bass_db, self.bass_target);
            let steps = got.div_ceil(BASS_STEP).max(1);
            for (k, chunk) in buf.chunks_mut(BASS_STEP * 2).enumerate() {
                if from != to {
                    let db = from + (to - from) * (k + 1) as f64 / steps as f64;
                    self.bass.set(Coeffs::new(FilterKind::LowShelf, self.sr, BASS_FREQ, 0.707, db));
                }
                self.bass.process_block(chunk);
            }
            self.bass_db = to;
            if to == 0.0 {
                self.bass.reset();
            }
        }
        // Fondu : rampe linéaire sur le bloc demandé (les gains suivent la courbe calculée par le mélangeur).
        let (g0, g1) = (self.fade, self.fade_end);
        let dg = (g1 - g0) / n.max(1) as f32;
        for (i, (o, s)) in out.as_chunks_mut::<2>().0.iter_mut().zip(buf.as_chunks::<2>().0).enumerate() {
            let g = g0 + dg * i as f32;
            o[0] += s[0] * g;
            o[1] += s[1] * g;
        }
        self.fade = g1;
        r
    }
}

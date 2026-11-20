//! Le vrai moteur de lecture (`control_audio::Engine`) derrière le [`Transport`] du lecteur.
//!
//! L'identifiant donné au moteur est celui de l'entrée de la file (unique même si un morceau y figure deux fois).
//! Pour AutoMix, les analyses de tempo sont faites sur un thread (et gardées en cache) puis passées au moteur.

use std::path::PathBuf;
use std::sync::Arc;

use crossbeam_channel::Receiver;

use control_audio::{BeatInfo, Engine, EngineConfig, EngineEvent};
use control_core::automix::Analysis;
use control_core::model::Track;

use crate::backend::Backend;
use crate::player::{SoundSettings, Transport, TransportEvent};

fn beat_info(a: &Analysis) -> BeatInfo {
    BeatInfo {
        duration: a.dur,
        bpm: a.bpm,
        confidence: a.conf,
        audible_start: a.start,
        audible_end: a.end,
        grid_start: a.grid_start,
        grid_end: a.grid_end,
    }
}

pub struct EngineTransport {
    engine: Engine,
    events: Receiver<EngineEvent>,
    backend: Arc<Backend>,
    automix: bool,
    current: Option<(u64, PathBuf)>,
    next: Option<(u64, PathBuf)>,
    /// Résultat d'analyse arrivé d'un thread : (entrée courante, entrée suivante, analyses).
    beats: crossbeam_channel::Sender<(u64, u64, Option<BeatInfo>, Option<BeatInfo>)>,
    beats_rx: Receiver<(u64, u64, Option<BeatInfo>, Option<BeatInfo>)>,
}

impl EngineTransport {
    /// Ouvre la sortie audio (une sortie muette s'il n'y a pas de carte son).
    pub fn new(backend: Arc<Backend>) -> anyhow::Result<EngineTransport> {
        let config = EngineConfig { ffmpeg: backend.ffmpeg(), ..EngineConfig::default() };
        let engine = Engine::new(config)?;
        let events = engine.events();
        let (beats, beats_rx) = crossbeam_channel::unbounded();
        Ok(EngineTransport { engine, events, backend, automix: false, current: None, next: None, beats, beats_rx })
    }

    /// AutoMix : analyse (ou relit du cache) les deux morceaux, puis donne leurs grilles au moteur.
    fn analyze_pair(&self) {
        let (Some((cur_uid, cur_path)), Some((next_uid, next_path))) = (self.current.clone(), self.next.clone()) else {
            return;
        };
        if !self.automix {
            return;
        }
        let backend = self.backend.clone();
        let tx = self.beats.clone();
        let spawned = std::thread::Builder::new().name("control-automix".into()).spawn(move || {
            let analyze = |p: &PathBuf| {
                let cache = backend.automix.lock();
                match cache.peek(p) {
                    Some(known) => known,
                    None => cache.analyze(p),
                }
            };
            let cur = analyze(&cur_path).map(|a| beat_info(&a));
            let next = analyze(&next_path).map(|a| beat_info(&a));
            let _ = tx.send((cur_uid, next_uid, cur, next));
        });
        if let Err(err) = spawned {
            log::warn!("Analyse AutoMix impossible : {err}");
        }
    }
}

impl Transport for EngineTransport {
    fn load(&mut self, uid: u64, track: &Track, start: f64, autoplay: bool, gain_db: f32) {
        let id = uid as i64;
        self.engine.set_track_gain_db(id, gain_db);
        if autoplay {
            self.engine.load(id, &track.path, start);
        } else {
            self.engine.load_paused(id, &track.path, start);
        }
        self.current = Some((uid, PathBuf::from(&track.path)));
        self.next = None;
    }

    fn preload(&mut self, next: Option<(u64, &Track, f32)>) {
        match next {
            Some((uid, track, gain)) => {
                if self.next.as_ref().is_some_and(|(u, _)| *u == uid) {
                    return;
                }
                self.engine.set_track_gain_db(uid as i64, gain);
                self.engine.preload_next(uid as i64, &track.path);
                self.next = Some((uid, PathBuf::from(&track.path)));
                self.analyze_pair();
            }
            None => {
                if self.next.take().is_some() {
                    self.engine.clear_next();
                }
            }
        }
    }

    fn play(&mut self) {
        self.engine.play();
    }

    fn pause(&mut self) {
        self.engine.pause();
    }

    fn stop(&mut self) {
        self.engine.stop();
        self.current = None;
        self.next = None;
    }

    fn seek(&mut self, seconds: f64) {
        self.engine.seek(seconds);
    }

    fn position(&self) -> f64 {
        self.engine.position()
    }

    fn duration(&self) -> Option<f64> {
        self.engine.duration()
    }

    fn is_playing(&self) -> bool {
        self.engine.is_playing()
    }

    fn apply(&mut self, sound: &SoundSettings) {
        self.engine.set_volume(sound.volume);
        self.engine.set_muted(sound.muted);
        self.engine.set_rate(sound.rate as f64);
        self.engine.set_eq(sound.eq_enabled, sound.eq_gains);
        self.engine.set_crossfade(sound.crossfade as f64);
        if sound.automix != self.automix {
            self.automix = sound.automix;
            if self.automix {
                self.analyze_pair();
            } else {
                self.engine.set_automix(false, None, None);
            }
        }
    }

    fn events(&mut self) -> Vec<TransportEvent> {
        // Analyses AutoMix arrivées : seulement si la paire est toujours celle en cours.
        while let Ok((cur, next, a, b)) = self.beats_rx.try_recv() {
            let same = self.current.as_ref().is_some_and(|(u, _)| *u == cur) && self.next.as_ref().is_some_and(|(u, _)| *u == next);
            if same && self.automix {
                self.engine.set_automix(true, a, b);
            }
        }
        let mut out = Vec::new();
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                EngineEvent::Advanced { to, .. } => {
                    let uid = to as u64;
                    self.current = self.next.take().filter(|(u, _)| *u == uid).or(Some((uid, PathBuf::new())));
                    out.push(TransportEvent::Advanced { to: uid });
                }
                EngineEvent::Ended { id } => out.push(TransportEvent::Ended { uid: id as u64 }),
                EngineEvent::Error { id, message } => out.push(TransportEvent::Error { uid: id as u64, message }),
                EngineEvent::Started { .. } | EngineEvent::NearEnd { .. } => {}
            }
        }
        out
    }
}

//! Le mélangeur : deux platines, la chaîne de sortie (égaliseur, volume, limiteur) et les enchaînements
//! (sans blanc, fondu enchaîné, AutoMix). Il tourne sur le thread du moteur (ou dans le rendu hors ligne des tests),
//! jamais dans le rappel de la carte son.

mod deck;
mod transition;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use control_core::model::TrackId;
use crossbeam_channel::Sender;

use crate::automix::BeatInfo;
use crate::dsp::eq::Equalizer;
use crate::dsp::gain::{Limiter, Smoothed, volume_curve};
use crate::engine::{EngineEvent, Status, StatusCell, TransitionKind};
use crate::source::{OpenOptions, Source};
use deck::{Deck, Loaded};
pub(crate) use transition::Transition;

/// Taille maximale d'un bloc de rendu (trames).
pub(crate) const BLOCK: usize = 512;
/// Rampe de démarrage et d'arrêt (s) : ni pause ni reprise ne claquent.
const PLAY_RAMP: f64 = 0.008;
/// Rampe du volume (s).
const VOLUME_RAMP: f64 = 0.03;

/// Ordres reçus par le mélangeur.
pub(crate) enum Command {
    Load {
        id: TrackId,
        path: PathBuf,
        start_at: f64,
        autoplay: bool,
    },
    Preload {
        id: TrackId,
        path: PathBuf,
    },
    ClearNext,
    Play,
    Pause,
    Stop,
    Seek(f64),
    Volume(f32),
    Muted(bool),
    Rate(f64),
    Eq {
        enabled: bool,
        gains: [f32; 10],
    },
    TrackGain {
        id: TrackId,
        db: f32,
    },
    Crossfade(f64),
    Automix {
        enabled: bool,
        current: Option<BeatInfo>,
        next: Option<BeatInfo>,
    },
    /// Fin d'une ouverture faite par un thread de chargement.
    Opened {
        token: u64,
        result: Box<Result<(Source, f64), String>>,
    },
    Quit,
}

/// Ouverture des fichiers : tout de suite (rendu hors ligne, déterministe) ou sur un thread de chargement.
pub(crate) enum Loader {
    Sync(OpenOptions),
    Threaded(OpenOptions, Sender<Command>),
}

fn open_at(path: &std::path::Path, start_at: f64, opts: &OpenOptions) -> Result<(Source, f64), String> {
    let mut src = Source::open(path, opts).map_err(|e| format!("{e:#}"))?;
    log::debug!("{} ouvert ({})", path.display(), src.backend());
    let at = if start_at > 0.0 { src.seek(start_at).map_err(|e| format!("{e:#}"))? } else { 0.0 };
    Ok((src, at))
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Purpose {
    Current { start_at: f64 },
    Next,
}

struct Ticket {
    token: u64,
    id: TrackId,
    purpose: Purpose,
}

/// Morceau suivant, ouvert et prêt.
pub(crate) struct Prepared {
    loaded: Loaded,
    at: f64,
}

pub(crate) struct Mixer {
    sr: u32,
    decks: [Deck; 2],
    cur: usize,
    playing: bool,
    play_gain: Smoothed,
    volume: f32,
    muted: bool,
    vol_gain: Smoothed,
    rate: f64,
    eq: Equalizer,
    limiter: Limiter,
    crossfade: f64,
    automix: bool,
    beat_cur: Option<BeatInfo>,
    beat_next: Option<BeatInfo>,
    next: Option<Prepared>,
    tickets: Vec<Ticket>,
    token: u64,
    transition: Option<Transition>,
    mix_skip: Option<TrackId>,
    ease: Option<transition::Ease>,
    gains: HashMap<TrackId, f32>,
    near_end_sent: bool,
    announce: bool,
    waiting_next: bool,
    near_end_secs: f64,
    events: Sender<EngineEvent>,
    status: Arc<StatusCell>,
    loader: Loader,
    /// Trames déjà rendues mais pas encore entendues (tampon de sortie), pour des positions exactes.
    pub latency: usize,
    flush: bool,
    pending: Option<(TrackId, f64)>,
}

impl Mixer {
    pub fn new(
        sr: u32,
        loader: Loader,
        events: Sender<EngineEvent>,
        status: Arc<StatusCell>,
        near_end_secs: f64,
    ) -> Mixer {
        let ramp = |secs: f64| ((sr as f64 * secs) as u32).max(1);
        Mixer {
            sr,
            decks: [Deck::new(sr), Deck::new(sr)],
            cur: 0,
            playing: false,
            play_gain: Smoothed::new(0.0, ramp(PLAY_RAMP)),
            volume: 1.0,
            muted: false,
            vol_gain: Smoothed::new(1.0, ramp(VOLUME_RAMP)),
            rate: 1.0,
            eq: Equalizer::new(sr),
            limiter: Limiter::new(sr),
            crossfade: 0.0,
            automix: false,
            beat_cur: None,
            beat_next: None,
            next: None,
            tickets: Vec::new(),
            token: 0,
            transition: None,
            mix_skip: None,
            ease: None,
            gains: HashMap::new(),
            near_end_sent: false,
            announce: false,
            waiting_next: false,
            near_end_secs,
            events,
            status,
            loader,
            latency: 0,
            flush: false,
            pending: None,
        }
    }

    fn emit(&self, e: EngineEvent) {
        let _ = self.events.send(e);
    }

    /// Le tampon de sortie doit être vidé (commande à entendre tout de suite).
    pub fn take_flush(&mut self) -> bool {
        std::mem::take(&mut self.flush)
    }

    fn gain_for(&self, id: TrackId) -> f32 {
        self.gains.get(&id).copied().unwrap_or(0.0)
    }

    fn request(&mut self, id: TrackId, path: PathBuf, purpose: Purpose) {
        self.token += 1;
        let token = self.token;
        self.tickets.push(Ticket { token, id, purpose });
        let start_at = match purpose {
            Purpose::Current { start_at } => start_at,
            Purpose::Next => 0.0,
        };
        match &self.loader {
            Loader::Sync(opts) => {
                let result = open_at(&path, start_at, opts);
                self.on_opened(token, result);
            }
            Loader::Threaded(opts, tx) => {
                let (opts, tx) = (opts.clone(), tx.clone());
                let spawned = std::thread::Builder::new().name("control-audio-load".into()).spawn(move || {
                    let result = open_at(&path, start_at, &opts);
                    let _ = tx.send(Command::Opened { token, result: Box::new(result) });
                });
                if let Err(e) = spawned {
                    self.on_opened(token, Err(format!("chargement impossible : {e}")));
                }
            }
        }
    }

    pub fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Load { id, path, start_at, autoplay } => {
                self.abort_transition();
                self.decks.iter_mut().for_each(Deck::unload);
                self.tickets.clear();
                self.next = None;
                self.beat_cur = None;
                self.beat_next = None;
                self.mix_skip = None;
                self.ease = None;
                self.near_end_sent = false;
                self.waiting_next = false;
                self.playing = autoplay;
                self.play_gain.set_now(0.0);
                self.flush = true;
                self.pending = Some((id, start_at.max(0.0)));
                self.request(id, path, Purpose::Current { start_at: start_at.max(0.0) });
            }
            Command::Preload { id, path } => {
                self.tickets.retain(|t| t.purpose != Purpose::Next);
                self.next = None;
                self.beat_next = None;
                self.request(id, path, Purpose::Next);
            }
            Command::ClearNext => {
                self.tickets.retain(|t| t.purpose != Purpose::Next);
                self.next = None;
                self.beat_next = None;
                if self.waiting_next {
                    self.end_current();
                }
            }
            Command::Play => self.play(),
            Command::Pause => self.pause(),
            Command::Stop => {
                self.abort_transition();
                self.decks.iter_mut().for_each(Deck::unload);
                self.tickets.clear();
                self.next = None;
                self.pending = None;
                self.beat_cur = None;
                self.beat_next = None;
                self.playing = false;
                self.waiting_next = false;
                self.play_gain.set_now(0.0);
                self.flush = true;
            }
            Command::Seek(t) => self.seek(t),
            Command::Volume(v) => {
                self.volume = if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 };
                self.apply_volume();
            }
            Command::Muted(m) => {
                self.muted = m;
                self.apply_volume();
            }
            Command::Rate(r) => {
                self.rate = if r.is_finite() { r.clamp(0.5, 2.0) } else { 1.0 };
                self.ease = None;
                // Pendant un mixage, l'asservissement recalcule aussitôt la vitesse de l'entrant.
                self.decks.iter_mut().for_each(|d| d.speed = self.rate);
            }
            Command::Eq { enabled, gains } => self.eq.set(enabled, gains),
            Command::TrackGain { id, db } => {
                let db = if db.is_finite() { db.clamp(-30.0, 24.0) } else { 0.0 };
                self.gains.insert(id, db);
                if self.gains.len() > 64 {
                    // Ne garder que les morceaux en cours ou attendus.
                    let keep: Vec<TrackId> = self.known_ids();
                    self.gains.retain(|k, _| keep.contains(k) || *k == id);
                }
                for d in self.decks.iter_mut().filter(|d| d.id() == Some(id)) {
                    d.set_gain_db(db);
                }
            }
            Command::Crossfade(s) => self.crossfade = if s.is_finite() { s.clamp(0.0, 12.0) } else { 0.0 },
            Command::Automix { enabled, current, next } => {
                self.automix = enabled;
                self.beat_cur = current;
                self.beat_next = next;
            }
            Command::Opened { token, result } => self.on_opened(token, *result),
            Command::Quit => {}
        }
    }

    fn known_ids(&self) -> Vec<TrackId> {
        let mut ids: Vec<TrackId> = self.decks.iter().filter_map(Deck::id).collect();
        ids.extend(self.next.as_ref().map(|n| n.loaded.id));
        ids.extend(self.tickets.iter().map(|t| t.id));
        ids
    }

    fn apply_volume(&mut self) {
        let g = if self.muted { 0.0 } else { volume_curve(self.volume) };
        self.vol_gain.set_target(g);
    }

    fn on_opened(&mut self, token: u64, result: Result<(Source, f64), String>) {
        let Some(i) = self.tickets.iter().position(|t| t.token == token) else { return };
        let ticket = self.tickets.remove(i);
        let id = ticket.id;
        match (ticket.purpose, result) {
            (Purpose::Current { .. }, Ok((source, at))) => {
                let gain = self.gain_for(id);
                let (rate, cur) = (self.rate, self.cur);
                self.decks[cur].attach(Loaded { id, source }, at, rate, gain);
                self.decks[cur].rendering = true;
                self.pending = None;
                self.announce = true;
                if self.playing {
                    self.play_gain.set_now(0.0);
                    self.play_gain.set_target(1.0);
                }
            }
            (Purpose::Current { .. }, Err(message)) => {
                self.pending = None;
                self.playing = false;
                self.emit(EngineEvent::Error { id, message });
            }
            (Purpose::Next, Ok((source, at))) => {
                self.next = Some(Prepared { loaded: Loaded { id, source }, at });
                if self.waiting_next {
                    self.advance_gapless();
                }
            }
            (Purpose::Next, Err(message)) => {
                self.emit(EngineEvent::Error { id, message });
                if self.waiting_next {
                    self.end_current();
                }
            }
        }
    }

    fn has_current(&self) -> bool {
        self.decks[self.cur].track.is_some()
    }

    fn play(&mut self) {
        if !self.has_current() && self.pending.is_none() {
            return;
        }
        if !self.playing {
            self.playing = true;
            self.flush = true;
            if self.has_current() {
                self.decks[self.cur].rendering = true;
            }
        }
        self.play_gain.set_target(1.0);
    }

    fn pause(&mut self) {
        if !self.playing {
            return;
        }
        self.abort_transition();
        self.playing = false;
        if self.latency > 0 && self.has_current() {
            // Le son des ~150 ms d'avance est abandonné : on revient à ce qui a vraiment été entendu.
            let d = &mut self.decks[self.cur];
            let heard = d.position() - self.latency as f64 / self.sr as f64 * d.speed;
            if let Err(message) = d.seek(heard.max(0.0)) {
                log::warn!("retour à la position entendue : {message}");
            }
            self.play_gain.set_now(0.0);
            self.flush = true;
        } else {
            self.play_gain.set_target(0.0);
        }
    }

    fn seek(&mut self, t: f64) {
        if !t.is_finite() {
            return;
        }
        if !self.has_current() {
            if let Some(p) = self.pending.as_mut() {
                p.1 = t.max(0.0);
            }
            return;
        }
        self.abort_transition();
        self.mix_skip = None;
        self.near_end_sent = false;
        self.waiting_next = false;
        let (cur, rate) = (self.cur, self.rate);
        self.decks[cur].speed = rate;
        if let Err(message) = self.decks[cur].seek(t) {
            let id = self.decks[cur].id().unwrap_or_default();
            self.emit(EngineEvent::Error { id, message });
        }
        self.flush = true;
        if self.playing {
            self.decks[cur].rendering = true;
            self.play_gain.set_now(0.0);
            self.play_gain.set_target(1.0);
        }
    }

    /// Le morceau en cours est fini et rien ne suit : fin de lecture, retour au début, en pause.
    fn end_current(&mut self) {
        self.waiting_next = false;
        self.playing = false;
        self.play_gain.set_now(0.0);
        let cur = self.cur;
        if let Some(id) = self.decks[cur].id() {
            self.emit(EngineEvent::Ended { id });
            let _ = self.decks[cur].seek(0.0);
        }
        self.near_end_sent = false;
    }

    /// Enchaînement sans blanc : le suivant prend la suite au même échantillon.
    fn advance_gapless(&mut self) {
        let Some(p) = self.next.take() else { return };
        let from = self.decks[self.cur].id();
        let inc = 1 - self.cur;
        let to = p.loaded.id;
        let gain = self.gain_for(to);
        let rate = self.rate;
        self.decks[inc].attach(p.loaded, p.at, rate, gain);
        self.decks[inc].rendering = true;
        self.decks[self.cur].unload();
        self.cur = inc;
        self.beat_cur = self.beat_next.take();
        self.near_end_sent = false;
        self.waiting_next = false;
        self.mix_skip = None;
        self.announce = false;
        if let Some(from) = from {
            self.emit(EngineEvent::Advanced { from, to });
        }
        self.emit(EngineEvent::Started { id: to });
    }

    /// Rend `out` (stéréo entrelacé) : tout le son des platines, mixé et traité.
    pub fn render(&mut self, out: &mut [f32]) {
        for chunk in out.chunks_mut(BLOCK * 2) {
            self.render_block(chunk);
        }
    }

    fn render_block(&mut self, out: &mut [f32]) {
        let n = out.len() / 2;
        out.iter_mut().for_each(|s| *s = 0.0);
        self.before_block(n);
        let audible = self.playing || self.play_gain.current() > 0.0 || !self.play_gain.is_settled();
        let mut ended = false;
        if audible && self.has_current() && self.decks[self.cur].rendering {
            let mut off = 0;
            loop {
                let cur = self.cur;
                let r = self.decks[cur].render(&mut out[off * 2..]);
                if r.frames > 0 && self.announce {
                    self.announce = false;
                    if let Some(id) = self.decks[cur].id() {
                        self.emit(EngineEvent::Started { id });
                    }
                }
                if let Some(message) = r.error {
                    let id = self.decks[cur].id().unwrap_or_default();
                    self.abort_transition();
                    self.decks[self.cur].rendering = false;
                    self.playing = false;
                    self.emit(EngineEvent::Error { id, message });
                    break;
                }
                if r.ended && self.transition.is_none() {
                    off += r.frames;
                    if self.playing && self.next.is_some() {
                        self.advance_gapless();
                        if off < n {
                            continue;
                        }
                    } else {
                        ended = true;
                    }
                }
                break;
            }
            if let Some(other) = self.transition.as_ref().map(|t| t.other_deck(self.cur))
                && self.decks[other].rendering
            {
                let r = self.decks[other].render(out);
                if let Some(message) = r.error {
                    let id = self.decks[other].id().unwrap_or_default();
                    self.abort_transition();
                    self.emit(EngineEvent::Error { id, message });
                }
            }
        }
        // Chaîne de sortie : volume, départ/arrêt, égaliseur, limiteur.
        if !self.vol_gain.is_settled()
            || !self.play_gain.is_settled()
            || self.vol_gain.current() != 1.0
            || self.play_gain.current() != 1.0
        {
            for f in out.as_chunks_mut::<2>().0 {
                let g = self.vol_gain.tick() * self.play_gain.tick();
                f[0] *= g;
                f[1] *= g;
            }
        }
        self.eq.process(out);
        self.limiter.process(out);
        if !self.playing && self.play_gain.is_settled() && self.play_gain.current() == 0.0 && self.has_current() {
            self.decks[self.cur].rendering = false;
        }
        self.after_block(n, ended);
        self.publish();
    }

    fn after_block(&mut self, n: usize, ended: bool) {
        self.progress_transition(n);
        if ended && self.transition.is_none() {
            let loading_next = self.tickets.iter().any(|t| t.purpose == Purpose::Next);
            if loading_next && self.playing {
                // Le suivant n'est pas encore ouvert : on l'attend en silence, puis il enchaîne.
                self.waiting_next = true;
            } else if !self.waiting_next {
                self.end_current();
            }
        }
        if !self.playing || !self.has_current() {
            return;
        }
        let d = &self.decks[self.cur];
        if !self.near_end_sent
            && let (Some(id), Some(dur)) = (d.id(), d.duration())
            && (dur - d.position()) / d.speed.max(0.01) <= self.near_end_secs
        {
            self.near_end_sent = true;
            self.emit(EngineEvent::NearEnd { id });
        }
        self.maybe_transition();
    }

    /// Publie l'état pour l'interface (sans verrou).
    pub fn publish(&self) {
        let d = &self.decks[self.cur];
        let transition = match &self.transition {
            None => TransitionKind::None,
            Some(Transition::Fade(_)) => TransitionKind::Crossfade,
            Some(Transition::Mix(_)) => TransitionKind::Automix,
        };
        let s = if let Some((id, at)) = self.pending {
            Status {
                track: Some(id),
                position: at,
                duration: None,
                playing: self.playing,
                loading: true,
                ..Status::default()
            }
        } else {
            let mut position = d.position();
            if d.rendering && self.playing {
                position -= self.latency as f64 / self.sr as f64 * d.speed;
            }
            Status {
                track: d.id(),
                position: position.max(0.0),
                duration: d.duration(),
                playing: self.playing && d.track.is_some(),
                loading: false,
                ..Status::default()
            }
        };
        self.status.store(&Status {
            deck: self.cur as u8,
            next: self.next.as_ref().map(|n| n.loaded.id),
            transition,
            rate: self.rate,
            ..s
        });
    }

    #[cfg(test)]
    pub(crate) fn deck_position(&self, deck: usize) -> f64 {
        self.decks[deck].position()
    }

    #[cfg(test)]
    pub(crate) fn deck_speed(&self, deck: usize) -> f64 {
        self.decks[deck].speed
    }

    #[cfg(test)]
    pub(crate) fn current_deck(&self) -> usize {
        self.cur
    }

    #[cfg(test)]
    pub(crate) fn transition(&self) -> Option<&Transition> {
        self.transition.as_ref()
    }
}

#[cfg(test)]
mod tests;

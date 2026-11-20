//! Lecteur : la file (`control_core::queue`), le comptage des écoutes et le pilotage du moteur audio.
//!
//! Le moteur est vu à travers [`Transport`] : le vrai (`control_audio`) en application, un moteur simulé en
//! rendu sans fenêtre et dans les tests.

use std::sync::Arc;

use serde_json::json;

use control_core::catalog::Catalog;
use control_core::model::{Kind, Track, TrackId};
use control_core::queue::{Listen, Queue, QueueItem, Repeat, Session, Step};
use control_core::settings::Settings;

use crate::backend::Backend;

/// Ce que le moteur signale à l'interface.
#[derive(Debug, Clone, PartialEq)]
pub enum TransportEvent {
    /// Le moteur est passé de lui-même à l'entrée préchargée (fondu enchaîné, enchaînement sans blanc).
    Advanced { to: u64 },
    /// L'entrée est finie (rien n'était préchargé).
    Ended { uid: u64 },
    Error { uid: u64, message: String },
}

/// Réglages de son appliqués au moteur.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundSettings {
    pub volume: f32,
    pub muted: bool,
    pub rate: f32,
    pub eq_enabled: bool,
    pub eq_gains: [f32; 10],
    pub crossfade: f32,
    pub automix: bool,
}

impl SoundSettings {
    pub fn from(s: &Settings) -> SoundSettings {
        let mut gains = [0.0f32; 10];
        for (g, v) in gains.iter_mut().zip(&s.eq.gains) {
            *g = (*v as f32).clamp(-12.0, 12.0);
        }
        SoundSettings {
            // Courbe du curseur de volume de l'original : v².
            volume: (s.volume.clamp(0.0, 1.0) as f32).powi(2),
            muted: s.muted,
            rate: (s.rate as f32).clamp(0.5, 2.0),
            eq_enabled: s.eq.enabled,
            eq_gains: gains,
            crossfade: (s.crossfade as f32).clamp(0.0, 12.0),
            automix: s.automix,
        }
    }
}

/// Le moteur de lecture, vu par le lecteur.
pub trait Transport {
    fn load(&mut self, uid: u64, track: &Track, start: f64, autoplay: bool, gain_db: f32);
    /// Prépare l'entrée suivante (fondu enchaîné ou enchaînement sans blanc), ou l'oublie (`None`).
    fn preload(&mut self, next: Option<(u64, &Track, f32)>);
    fn play(&mut self);
    fn pause(&mut self);
    fn stop(&mut self);
    fn seek(&mut self, seconds: f64);
    fn position(&self) -> f64;
    fn duration(&self) -> Option<f64>;
    fn is_playing(&self) -> bool;
    fn apply(&mut self, sound: &SoundSettings);
    fn events(&mut self) -> Vec<TransportEvent>;
    /// Avance l'horloge (moteur simulé seulement ; le vrai moteur a la sienne).
    fn tick(&mut self, _clock: f64) {}
}

/// Moteur simulé : le temps avance avec l'horloge de l'interface. Sert sans carte son et en rendu sans fenêtre.
#[derive(Debug, Default)]
pub struct SimTransport {
    uid: Option<u64>,
    duration: f64,
    position: f64,
    playing: bool,
    last_clock: Option<f64>,
    rate: f64,
    pending: Vec<TransportEvent>,
}

impl SimTransport {
    /// Fait avancer la lecture jusqu'à `clock` (secondes).
    pub fn advance(&mut self, clock: f64) {
        if let (true, Some(last)) = (self.playing, self.last_clock) {
            self.position += (clock - last).max(0.0) * if self.rate > 0.0 { self.rate } else { 1.0 };
            if self.duration > 0.0 && self.position >= self.duration {
                self.position = self.duration;
                self.playing = false;
                if let Some(uid) = self.uid {
                    self.pending.push(TransportEvent::Ended { uid });
                }
            }
        }
        self.last_clock = Some(clock);
    }
}

impl Transport for SimTransport {
    fn load(&mut self, uid: u64, track: &Track, start: f64, autoplay: bool, _gain_db: f32) {
        self.uid = Some(uid);
        self.duration = track.duration;
        self.position = start.max(0.0);
        self.playing = autoplay;
    }
    fn preload(&mut self, _next: Option<(u64, &Track, f32)>) {}
    fn play(&mut self) {
        if self.uid.is_some() {
            self.playing = true;
        }
    }
    fn pause(&mut self) {
        self.playing = false;
    }
    fn stop(&mut self) {
        self.playing = false;
        self.uid = None;
        self.position = 0.0;
    }
    fn seek(&mut self, seconds: f64) {
        self.position = seconds.clamp(0.0, self.duration.max(0.0));
    }
    fn position(&self) -> f64 {
        self.position
    }
    fn duration(&self) -> Option<f64> {
        (self.duration > 0.0).then_some(self.duration)
    }
    fn is_playing(&self) -> bool {
        self.playing
    }
    fn apply(&mut self, sound: &SoundSettings) {
        self.rate = sound.rate as f64;
    }
    fn events(&mut self) -> Vec<TransportEvent> {
        std::mem::take(&mut self.pending)
    }
    fn tick(&mut self, clock: f64) {
        self.advance(clock);
    }
}

pub struct Player {
    pub queue: Queue,
    pub transport: Box<dyn Transport>,
    /// Le morceau de l'entrée courante (copie : la bibliothèque peut changer sous nos pieds).
    pub track: Option<Track>,
    pub repeat: Repeat,
    pub shuffle: bool,
    pub sound: SoundSettings,
    /// Normalisation du volume : active, et niveau visé (LUFS).
    normalize: Option<f64>,
    listen: Listen,
    /// Écoute enregistrée dans la base (identifiant renvoyé par `play_start`), en attente ou faite.
    play_id: Option<Arc<parking_lot::Mutex<Option<i64>>>>,
    /// Dernière sauvegarde de la session (horloge de l'interface).
    saved_at: f64,
    /// Message d'erreur du dernier chargement.
    pub error: Option<String>,
    failures: u32,
}

impl Player {
    pub fn new(settings: &Settings) -> Player {
        Player {
            queue: Queue::new(),
            transport: Box::new(SimTransport::default()),
            track: None,
            repeat: Repeat::parse(&settings.repeat),
            shuffle: settings.shuffle,
            sound: SoundSettings::from(settings),
            normalize: settings.normalize.then_some(settings.normalize_target),
            listen: Listen::default(),
            play_id: None,
            saved_at: 0.0,
            error: None,
            failures: 0,
        }
    }

    pub fn set_transport(&mut self, mut transport: Box<dyn Transport>) {
        transport.apply(&self.sound);
        self.transport = transport;
    }

    pub fn current_id(&self) -> Option<TrackId> {
        self.track.as_ref().map(|t| t.id)
    }

    pub fn is_playing(&self) -> bool {
        self.transport.is_playing()
    }

    pub fn position(&self) -> f64 {
        self.transport.position()
    }

    pub fn duration(&self) -> f64 {
        self.transport.duration().filter(|d| *d > 0.0).or(self.track.as_ref().map(|t| t.duration)).unwrap_or(0.0)
    }

    pub fn apply_settings(&mut self, s: &Settings) {
        self.repeat = Repeat::parse(&s.repeat);
        if s.shuffle != self.shuffle {
            self.shuffle = s.shuffle;
            self.queue.set_shuffle(s.shuffle);
        }
        let sound = SoundSettings::from(s);
        let normalize = s.normalize.then_some(s.normalize_target);
        if sound != self.sound {
            self.sound = sound;
            self.transport.apply(&self.sound);
        }
        if normalize != self.normalize {
            self.normalize = normalize;
        }
    }

    /// Gain de normalisation d'un morceau (dB), 0 sans normalisation.
    fn gain_for(&self, t: &Track) -> f32 {
        match self.normalize {
            Some(target) => control_core::loudness::gain_db(t.loudness, t.true_peak, target) as f32,
            None => 0.0,
        }
    }

    fn track_of(cat: &Catalog, item: Option<QueueItem>) -> Option<Track> {
        item.and_then(|it| cat.track(it.track).cloned())
    }

    /// Exécute ce que demande la file.
    fn step(&mut self, step: Step, cat: &Catalog, backend: &Arc<Backend>, start: f64) {
        match step {
            Step::Load { autoplay } => self.load_current(cat, backend, start, autoplay),
            Step::Restart => {
                self.finish_listen(backend, false);
                self.listen.reset(self.current_id());
                self.transport.seek(0.0);
                self.transport.play();
            }
            Step::Stop => {
                self.finish_listen(backend, true);
                self.transport.pause();
                self.transport.seek(0.0);
            }
            Step::Nothing => self.refresh_preload(cat),
        }
    }

    fn load_current(&mut self, cat: &Catalog, backend: &Arc<Backend>, start: f64, autoplay: bool) {
        self.finish_listen(backend, false);
        let item = self.queue.current();
        self.track = Self::track_of(cat, item);
        self.error = None;
        match (item, &self.track) {
            (Some(it), Some(t)) => {
                let gain = self.gain_for(t);
                self.transport.load(it.uid, t, start, autoplay, gain);
                self.listen.reset(Some(t.id));
            }
            _ => {
                self.transport.stop();
                self.listen.reset(None);
            }
        }
        self.refresh_preload(cat);
    }

    /// Prépare l'entrée suivante pour le moteur (s'il y en a une, et pour l'audio seulement).
    fn refresh_preload(&mut self, cat: &Catalog) {
        let next = self.queue.peek_next(self.repeat).and_then(|it| cat.track(it.track).map(|t| (it.uid, t)));
        match next {
            Some((uid, t)) if t.kind == Kind::Audio && self.track.as_ref().is_some_and(|c| c.kind == Kind::Audio) => {
                let gain = self.gain_for(t);
                self.transport.preload(Some((uid, t, gain)));
            }
            _ => self.transport.preload(None),
        }
    }

    // ─── Commandes ───────────────────────────────────────────────────────────

    pub fn play_list(&mut self, ids: &[TrackId], start: usize, shuffle: Option<bool>, cat: &Catalog, backend: &Arc<Backend>) {
        let use_shuffle = shuffle.unwrap_or(self.shuffle);
        if let Some(sh) = shuffle {
            self.shuffle = sh;
        }
        let step = self.queue.play(ids, start, use_shuffle);
        self.step(step, cat, backend, 0.0);
    }

    pub fn toggle(&mut self) {
        if self.track.is_none() {
            return;
        }
        if self.transport.is_playing() {
            self.transport.pause();
        } else {
            self.transport.play();
        }
        self.listen.break_continuity();
    }

    pub fn next(&mut self, cat: &Catalog, backend: &Arc<Backend>) {
        let step = self.queue.next(self.repeat);
        self.step(step, cat, backend, 0.0);
    }

    pub fn prev(&mut self, cat: &Catalog, backend: &Arc<Backend>) {
        let step = self.queue.prev(self.transport.position());
        self.step(step, cat, backend, 0.0);
    }

    pub fn jump_to(&mut self, i: usize, cat: &Catalog, backend: &Arc<Backend>) {
        let step = self.queue.jump_to(i);
        self.step(step, cat, backend, 0.0);
    }

    pub fn seek(&mut self, seconds: f64) {
        if !seconds.is_finite() {
            return;
        }
        let d = self.duration();
        self.transport.seek(if d > 0.0 { seconds.clamp(0.0, d) } else { seconds.max(0.0) });
        self.listen.break_continuity();
    }

    pub fn play_next(&mut self, ids: &[TrackId], cat: &Catalog, backend: &Arc<Backend>) {
        let step = self.queue.play_next(ids, self.shuffle);
        self.step(step, cat, backend, 0.0);
    }

    pub fn enqueue(&mut self, ids: &[TrackId], cat: &Catalog, backend: &Arc<Backend>) {
        let step = self.queue.enqueue(ids, self.shuffle);
        self.step(step, cat, backend, 0.0);
    }

    pub fn remove_at(&mut self, i: usize, cat: &Catalog) {
        self.queue.remove_at(i);
        self.refresh_preload(cat);
    }

    pub fn move_item(&mut self, from: usize, to: usize, cat: &Catalog) {
        self.queue.move_item(from, to);
        self.refresh_preload(cat);
    }

    pub fn clear_upcoming(&mut self, cat: &Catalog) {
        self.queue.clear_upcoming();
        self.refresh_preload(cat);
    }

    // ─── Bibliothèque et session ─────────────────────────────────────────────

    /// La bibliothèque a changé : la file perd les morceaux disparus, le morceau courant est rafraîchi.
    pub fn library_changed(&mut self, cat: &Catalog, backend: &Arc<Backend>) {
        let step = self.queue.prune(|id| cat.by_id.contains_key(&id));
        match step {
            Step::Load { .. } | Step::Stop => self.step(step, cat, backend, 0.0),
            _ => {
                if let Some(id) = self.current_id() {
                    self.track = cat.track(id).cloned().or(self.track.take());
                }
                self.refresh_preload(cat);
            }
        }
    }

    pub fn restore_session(&mut self, settings: &Settings, cat: &Catalog) {
        let Ok(session) = serde_json::from_value::<Session>(settings.session.clone()) else { return };
        let Some(position) = self.queue.restore(&session, |id| cat.by_id.contains_key(&id)) else { return };
        self.track = Self::track_of(cat, self.queue.current());
        if let (Some(it), Some(t)) = (self.queue.current(), &self.track) {
            let gain = self.gain_for(t);
            self.transport.load(it.uid, t, position, false, gain);
            self.listen.reset(Some(t.id));
        }
        self.refresh_preload(cat);
    }

    /// Session à enregistrer (`session` dans les réglages).
    pub fn session_value(&self) -> Option<serde_json::Value> {
        self.queue.session(self.transport.position()).map(|s| json!(s))
    }

    // ─── Horloge ─────────────────────────────────────────────────────────────

    /// Appelé à chaque image tant que quelque chose joue : écoutes, fin de morceau, sauvegarde de la session.
    /// Renvoie `true` si l'interface doit être redessinée pour autre chose que la position.
    pub fn tick(&mut self, clock: f64, cat: &Catalog, backend: &Arc<Backend>) -> bool {
        self.transport.tick(clock);
        let mut changed = false;
        for ev in self.transport.events() {
            changed = true;
            match ev {
                TransportEvent::Advanced { to } => {
                    self.finish_listen(backend, true);
                    if self.queue.advance_to(to) {
                        self.track = Self::track_of(cat, self.queue.current());
                        self.listen.reset(self.current_id());
                        self.failures = 0;
                    }
                    self.refresh_preload(cat);
                }
                TransportEvent::Ended { uid } => {
                    if self.queue.current().map(|c| c.uid) == Some(uid) {
                        self.failures = 0;
                        let step = self.queue.ended(self.repeat);
                        if step == Step::Stop {
                            self.finish_listen(backend, true);
                            self.transport.seek(0.0);
                        } else {
                            if matches!(step, Step::Load { .. }) {
                                self.finish_listen(backend, true);
                            }
                            self.step(step, cat, backend, 0.0);
                        }
                    }
                }
                TransportEvent::Error { uid, message } => {
                    if self.queue.current().map(|c| c.uid) == Some(uid) {
                        log::warn!("Lecture impossible : {message}");
                        self.error = Some(message);
                        self.failures += 1;
                        // Un fichier illisible : on passe au suivant, sans boucler sur une file entière d'erreurs.
                        if self.failures < 5 && self.queue.upcoming().len() > 0 {
                            let step = self.queue.next(Repeat::Off);
                            self.step(step, cat, backend, 0.0);
                        }
                    }
                }
            }
        }
        if self.transport.is_playing() {
            let pos = self.transport.position();
            let dur = self.duration();
            if self.listen.tick(pos, dur)
                && let Some(t) = &self.track
            {
                let slot = Arc::new(parking_lot::Mutex::new(None));
                self.play_id = Some(slot.clone());
                let (lib, id) = (backend.library.clone(), t.id);
                std::thread::spawn(move || {
                    *slot.lock() = lib.play_start(id, dur);
                });
            }
            if clock - self.saved_at > 15.0 {
                self.saved_at = clock;
                if let Some(v) = self.session_value() {
                    backend.store.patch(&json!({ "session": v }));
                }
            }
        }
        changed
    }

    /// Termine l'écoute en cours (enregistre la durée écoutée si l'écoute a été comptée).
    fn finish_listen(&mut self, backend: &Arc<Backend>, completed: bool) {
        if let Some(slot) = self.play_id.take() {
            let seconds = self.listen.listened.round();
            let lib = backend.library.clone();
            std::thread::spawn(move || {
                // L'enregistrement du début peut être encore en cours : on l'attend un peu.
                for _ in 0..50 {
                    if let Some(id) = *slot.lock() {
                        lib.play_end(id, seconds, completed);
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            });
        }
        self.listen.reset(None);
    }

    /// Sauvegarde la session tout de suite (fermeture de la fenêtre).
    pub fn save_session(&self, backend: &Backend) {
        if let Some(v) = self.session_value() {
            backend.store.patch(&json!({ "session": v }));
        }
    }
}

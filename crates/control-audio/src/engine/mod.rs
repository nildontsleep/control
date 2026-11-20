//! Le moteur, vu de l'application : une poignée [`Engine`] (clonable, partageable entre threads) qui envoie des
//! ordres au thread du mélangeur, un état lisible sans verrou et un canal d'événements.
//!
//! Threads :
//! - `control-audio-mixer` : décode, étire, mixe et traite le son par blocs de 512 trames, et garde ~150 ms
//!   d'avance dans un tampon circulaire sans verrou ;
//! - `control-audio-output` : porte le flux cpal ; son rappel ne fait que lire le tampon (ni verrou ni allocation) ;
//!   sans carte son, `control-audio-null` consomme le tampon au rythme de l'horloge et jette le son ;
//! - `control-audio-load` : un par ouverture de fichier (sonde, premier paquet, lancement de ffmpeg), pour ne jamais
//!   faire attendre le mélangeur ;
//! - `control-audio-ffmpeg` : lecture de la sortie de ffmpeg, pour les fichiers qu'il décode.

mod status;

use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::Result;
use control_core::model::TrackId;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TryRecvError, unbounded};
use parking_lot::Mutex;

use crate::automix::BeatInfo;
use crate::mixer::{BLOCK, Command, Loader, Mixer};
use crate::output::{self, Output, ring::Producer};
use crate::source::OpenOptions;
pub(crate) use status::StatusCell;
pub use status::{Status, TransitionKind};

/// Événements du moteur, dans l'ordre où ils se produisent.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    /// Un morceau commence à se faire entendre (chargé et lancé, ou entrant d'un enchaînement).
    Started { id: TrackId },
    /// Le morceau en cours approche de sa fin : c'est le moment de précharger le suivant.
    NearEnd { id: TrackId },
    /// Le morceau en cours est désormais `to` (enchaînement sans blanc, fondu ou mixage AutoMix).
    Advanced { from: TrackId, to: TrackId },
    /// Le morceau est fini et rien ne suit : lecture arrêtée, revenue au début du morceau.
    Ended { id: TrackId },
    /// Ouverture ou décodage impossible (message en français, pour l'utilisateur).
    Error { id: TrackId, message: String },
}

/// Où va le son.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputMode {
    /// La carte son par défaut ; à défaut, la sortie muette.
    #[default]
    Auto,
    /// Sortie muette cadencée par l'horloge (tests, serveur).
    Null,
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// ffmpeg, pour les formats que symphonia ne lit pas (Opus, audio des vidéos).
    pub ffmpeg: Option<PathBuf>,
    pub output: OutputMode,
    /// Avance du rendu sur la carte son (ms).
    pub buffer_ms: u32,
    /// `NearEnd` est émis quand il reste moins que cela (s, temps réel).
    pub near_end_secs: f64,
    /// Fréquence de la sortie muette.
    pub null_sample_rate: u32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            ffmpeg: None,
            output: OutputMode::Auto,
            buffer_ms: 150,
            near_end_secs: 30.0,
            null_sample_rate: 48_000,
        }
    }
}

struct Inner {
    tx: Sender<Command>,
    events: Receiver<EngineEvent>,
    status: Arc<StatusCell>,
    sample_rate: u32,
    device: Option<String>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Quit);
        if let Some(t) = self.thread.lock().take() {
            let _ = t.join();
        }
    }
}

/// Poignée du moteur. Les commandes ne bloquent jamais : elles sont exécutées dans l'ordre par le mélangeur.
#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

impl Engine {
    /// Démarre le moteur sur la carte son par défaut, ou sur la sortie muette s'il n'y en a pas (ou si
    /// `config.output` le demande). N'échoue que si un thread ne peut pas être créé.
    pub fn new(config: EngineConfig) -> Result<Engine> {
        let (output, producer) = match config.output {
            OutputMode::Auto => match output::open_device() {
                Ok(o) => o,
                Err(e) => {
                    log::info!("pas de sortie audio ({e:#}) : sortie muette");
                    output::open_null(config.null_sample_rate)?
                }
            },
            OutputMode::Null => output::open_null(config.null_sample_rate)?,
        };
        let sample_rate = output.sample_rate;
        let device = output.device.clone();
        let (tx, rx) = unbounded();
        let (events_tx, events) = unbounded();
        let status = Arc::new(StatusCell::new());
        let opts = OpenOptions { ffmpeg: config.ffmpeg.clone(), sample_rate };
        let mixer = Mixer::new(
            sample_rate,
            Loader::Threaded(opts, tx.clone()),
            events_tx,
            Arc::clone(&status),
            config.near_end_secs,
        );
        let target = (sample_rate as usize * config.buffer_ms.clamp(20, 1000) as usize / 1000).max(BLOCK * 2);
        let thread = std::thread::Builder::new()
            .name("control-audio-mixer".into())
            .spawn(move || run(mixer, rx, producer, output, target))?;
        Ok(Engine {
            inner: Arc::new(Inner { tx, events, status, sample_rate, device, thread: Mutex::new(Some(thread)) }),
        })
    }

    /// Moteur sans thread ni carte son : le son est rendu à la demande par le [`OfflineRenderer`], de façon
    /// déterministe (les fichiers sont ouverts au moment où le rendu traite la commande).
    pub fn offline(config: EngineConfig, sample_rate: u32) -> (Engine, OfflineRenderer) {
        let (tx, rx) = unbounded();
        let (events_tx, events) = unbounded();
        let status = Arc::new(StatusCell::new());
        let opts = OpenOptions { ffmpeg: config.ffmpeg.clone(), sample_rate };
        let mixer = Mixer::new(sample_rate, Loader::Sync(opts), events_tx, Arc::clone(&status), config.near_end_secs);
        let inner = Inner { tx, events, status, sample_rate, device: None, thread: Mutex::new(None) };
        (Engine { inner: Arc::new(inner) }, OfflineRenderer { mixer, rx, sample_rate })
    }

    fn send(&self, c: Command) {
        let _ = self.inner.tx.send(c);
    }

    /// Fréquence de sortie (Hz).
    pub fn sample_rate(&self) -> u32 {
        self.inner.sample_rate
    }

    /// Nom de la carte son utilisée ; `None` : sortie muette ou rendu hors ligne.
    pub fn output_device(&self) -> Option<&str> {
        self.inner.device.as_deref()
    }

    /// Événements (un récepteur partagé : chaque événement n'est reçu qu'une fois, par l'un des récepteurs).
    pub fn events(&self) -> Receiver<EngineEvent> {
        self.inner.events.clone()
    }

    /// État courant, sans verrou.
    pub fn status(&self) -> Status {
        self.inner.status.load()
    }

    pub fn position(&self) -> f64 {
        self.status().position
    }

    pub fn duration(&self) -> Option<f64> {
        self.status().duration
    }

    pub fn is_playing(&self) -> bool {
        self.status().playing
    }

    pub fn current(&self) -> Option<TrackId> {
        self.status().track
    }

    /// Ouvre un morceau et le lance à `start_at` (s). Le morceau préchargé est oublié.
    pub fn load(&self, id: TrackId, path: impl Into<PathBuf>, start_at: f64) {
        self.send(Command::Load { id, path: path.into(), start_at, autoplay: true });
    }

    /// Ouvre un morceau sans le lancer (reprise d'une session).
    pub fn load_paused(&self, id: TrackId, path: impl Into<PathBuf>, start_at: f64) {
        self.send(Command::Load { id, path: path.into(), start_at, autoplay: false });
    }

    /// Précharge le morceau suivant : il enchaînera sans blanc, en fondu ou en mixage AutoMix selon les réglages.
    /// Appeler [`Engine::set_automix`] *après* (ses informations de tempo pour « suivant » s'appliquent à lui).
    pub fn preload_next(&self, id: TrackId, path: impl Into<PathBuf>) {
        self.send(Command::Preload { id, path: path.into() });
    }

    /// Oublie le morceau préchargé (la file a changé).
    pub fn clear_next(&self) {
        self.send(Command::ClearNext);
    }

    pub fn play(&self) {
        self.send(Command::Play);
    }

    /// Pause (une transition en cours est abandonnée : seul le morceau en cours reste).
    pub fn pause(&self) {
        self.send(Command::Pause);
    }

    /// Arrêt complet : plus aucun morceau ouvert.
    pub fn stop(&self) {
        self.send(Command::Stop);
    }

    pub fn seek(&self, seconds: f64) {
        self.send(Command::Seek(seconds));
    }

    /// Volume de l'interface (0..1), appliqué avec la même courbe quadratique que la version Electron.
    pub fn set_volume(&self, volume: f32) {
        self.send(Command::Volume(volume));
    }

    pub fn set_muted(&self, muted: bool) {
        self.send(Command::Muted(muted));
    }

    /// Vitesse de lecture (0,5..2), hauteur conservée.
    pub fn set_rate(&self, rate: f64) {
        self.send(Command::Rate(rate));
    }

    /// Égaliseur 10 bandes (gains en dB, bandes de [`crate::dsp::eq::EQ_BANDS`]).
    pub fn set_eq(&self, enabled: bool, gains: [f32; 10]) {
        self.send(Command::Eq { enabled, gains });
    }

    /// Gain de normalisation d'un morceau (dB, voir [`crate::dsp::gain::normalization_gain_db`]) : il s'applique à
    /// la platine qui le joue, ou au morceau quand il arrivera (préchargé ou en cours d'ouverture). Le limiteur de
    /// sortie empêche toute saturation.
    pub fn set_track_gain_db(&self, id: TrackId, db: f32) {
        self.send(Command::TrackGain { id, db });
    }

    /// Durée du fondu enchaîné (s, 0 : pas de fondu ; au plus 12).
    pub fn set_crossfade(&self, seconds: f64) {
        self.send(Command::Crossfade(seconds));
    }

    /// AutoMix, avec l'analyse du morceau en cours et du morceau préchargé (`None` : inconnue, d'où un fondu de
    /// secours calé sur la fin).
    pub fn set_automix(&self, enabled: bool, current: Option<BeatInfo>, next: Option<BeatInfo>) {
        self.send(Command::Automix { enabled, current, next });
    }
}

/// Rendu hors ligne (tests) : chaque appel exécute les commandes reçues puis rend le son demandé.
pub struct OfflineRenderer {
    mixer: Mixer,
    rx: Receiver<Command>,
    sample_rate: u32,
}

impl OfflineRenderer {
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn drain(&mut self) {
        while let Ok(c) = self.rx.try_recv() {
            self.mixer.handle(c);
        }
        self.mixer.take_flush();
    }

    /// Rend `out` (stéréo entrelacé).
    pub fn render(&mut self, out: &mut [f32]) {
        self.drain();
        self.mixer.render(out);
        if out.is_empty() {
            self.mixer.publish();
        }
    }

    /// Rend `seconds` secondes et les rend sous forme de tampon stéréo entrelacé.
    pub fn render_seconds(&mut self, seconds: f64) -> Vec<f32> {
        let frames = (seconds * self.sample_rate as f64).round() as usize;
        let mut out = vec![0.0; frames * 2];
        self.render(&mut out);
        out
    }

    #[cfg(test)]
    pub(crate) fn mixer(&self) -> &Mixer {
        &self.mixer
    }
}

/// Boucle du thread du mélangeur.
fn run(mut mixer: Mixer, rx: Receiver<Command>, mut producer: Producer, output: Output, target_frames: usize) {
    let mut block = vec![0f32; BLOCK * 2];
    let handle = |mixer: &mut Mixer, producer: &Producer, c: Command| -> bool {
        if matches!(c, Command::Quit) {
            return false;
        }
        mixer.latency = producer.fill() / 2;
        mixer.handle(c);
        true
    };
    'outer: loop {
        loop {
            match rx.try_recv() {
                Ok(c) => {
                    if !handle(&mut mixer, &producer, c) {
                        break 'outer;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break 'outer,
            }
        }
        if mixer.take_flush() {
            producer.request_flush();
        }
        while producer.fill() / 2 + BLOCK <= target_frames && producer.free() >= BLOCK * 2 {
            mixer.latency = producer.fill() / 2;
            mixer.render(&mut block);
            producer.push(&block);
        }
        match rx.recv_timeout(Duration::from_millis(4)) {
            Ok(c) => {
                if !handle(&mut mixer, &producer, c) {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    drop(output);
}

#[cfg(test)]
mod tests;

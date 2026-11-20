//! État du moteur lisible sans verrou (verrou de séquence fait d'atomiques) : l'interface le lit à ~30 Hz pendant
//! que le mélangeur l'écrit après chaque bloc.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU8, AtomicU64, Ordering, fence};

use control_core::model::TrackId;

/// Transition en cours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransitionKind {
    #[default]
    None,
    /// Fondu enchaîné simple (y compris le fondu de secours d'AutoMix).
    Crossfade,
    /// Mixage AutoMix calé sur le tempo.
    Automix,
}

/// Instantané de l'état du moteur.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Status {
    /// Morceau en cours (celui que l'interface affiche).
    pub track: Option<TrackId>,
    /// Position entendue dans ce morceau (s).
    pub position: f64,
    pub duration: Option<f64>,
    pub playing: bool,
    /// Le morceau en cours est en train de s'ouvrir.
    pub loading: bool,
    /// Platine du morceau en cours (0 ou 1).
    pub deck: u8,
    /// Morceau suivant préchargé et prêt.
    pub next: Option<TrackId>,
    pub transition: TransitionKind,
    /// Vitesse générale.
    pub rate: f64,
}

const NONE: i64 = i64::MIN;

#[derive(Default)]
pub(crate) struct StatusCell {
    seq: AtomicU64,
    track: AtomicI64,
    position: AtomicU64,
    duration: AtomicU64,
    playing: AtomicBool,
    loading: AtomicBool,
    deck: AtomicU8,
    next: AtomicI64,
    transition: AtomicU8,
    rate: AtomicU64,
}

fn opt_id(v: i64) -> Option<TrackId> {
    (v != NONE).then_some(v)
}

impl StatusCell {
    pub fn new() -> StatusCell {
        let c = StatusCell::default();
        c.store(&Status { rate: 1.0, ..Status::default() });
        c
    }

    /// Un seul écrivain : le mélangeur.
    pub fn store(&self, s: &Status) {
        let seq = self.seq.load(Ordering::Relaxed);
        self.seq.store(seq.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        self.track.store(s.track.unwrap_or(NONE), Ordering::Relaxed);
        self.position.store(s.position.to_bits(), Ordering::Relaxed);
        self.duration.store(s.duration.unwrap_or(f64::NAN).to_bits(), Ordering::Relaxed);
        self.playing.store(s.playing, Ordering::Relaxed);
        self.loading.store(s.loading, Ordering::Relaxed);
        self.deck.store(s.deck, Ordering::Relaxed);
        self.next.store(s.next.unwrap_or(NONE), Ordering::Relaxed);
        let t = match s.transition {
            TransitionKind::None => 0,
            TransitionKind::Crossfade => 1,
            TransitionKind::Automix => 2,
        };
        self.transition.store(t, Ordering::Relaxed);
        self.rate.store(s.rate.to_bits(), Ordering::Relaxed);
        self.seq.store(seq.wrapping_add(2), Ordering::Release);
    }

    pub fn load(&self) -> Status {
        loop {
            let s1 = self.seq.load(Ordering::Acquire);
            if s1 & 1 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let duration = f64::from_bits(self.duration.load(Ordering::Relaxed));
            let s = Status {
                track: opt_id(self.track.load(Ordering::Relaxed)),
                position: f64::from_bits(self.position.load(Ordering::Relaxed)),
                duration: (!duration.is_nan()).then_some(duration),
                playing: self.playing.load(Ordering::Relaxed),
                loading: self.loading.load(Ordering::Relaxed),
                deck: self.deck.load(Ordering::Relaxed),
                next: opt_id(self.next.load(Ordering::Relaxed)),
                transition: match self.transition.load(Ordering::Relaxed) {
                    1 => TransitionKind::Crossfade,
                    2 => TransitionKind::Automix,
                    _ => TransitionKind::None,
                },
                rate: f64::from_bits(self.rate.load(Ordering::Relaxed)),
            };
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == s1 {
                return s;
            }
        }
    }
}

//! Moteur de lecture de control : décodage (symphonia, ffmpeg en secours), deux platines pour les enchaînements
//! (sans blanc, fondu enchaîné à puissance constante, AutoMix calé sur le tempo), égaliseur 10 bandes,
//! normalisation avec limiteur, vitesse variable à hauteur constante, sortie cpal.
//!
//! La file d'attente reste à l'interface : elle charge un morceau ([`Engine::load`]), précharge le suivant quand le
//! moteur l'annonce ([`EngineEvent::NearEnd`]) et suit les changements de morceau ([`EngineEvent::Advanced`]).

pub mod automix;
pub mod dsp;
mod engine;
mod mixer;
mod output;
mod source;

pub use automix::{BeatInfo, MixPlan, PlanOptions, plan_mix};
pub use control_core::model::TrackId;
pub use engine::{Engine, EngineConfig, EngineEvent, OfflineRenderer, OutputMode, Status, TransitionKind};

#[cfg(test)]
pub(crate) mod testutil;

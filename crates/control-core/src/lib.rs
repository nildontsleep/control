#![recursion_limit = "256"]
//! Noyau de control : tout ce qui ne dépend ni d'une fenêtre ni d'un navigateur.
//!
//! L'application de bureau crée un seul noyau ; le serveur web, un par compte.

pub mod artists;
pub mod automix;
pub mod catalog;
pub mod core;
pub mod covers;
pub mod db;
pub mod discord;
pub mod downloads;
pub mod library;
pub mod loudness;
pub mod lrc;
pub mod lyrics;
pub mod media;
pub mod model;
pub mod probe;
pub mod process;
pub mod queue;
pub mod scrobble;
pub mod settings;
pub mod smart;
pub mod source_url;
pub mod stats;
pub mod store;
pub mod tagpattern;
pub mod tags;
pub mod tools;
pub mod updater;
pub mod urlguard;
pub mod util;

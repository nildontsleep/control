//! Traitement du signal : filtres et égaliseur, gains lissés et limiteur, courbes de fondu, changement de vitesse
//! sans changement de hauteur.

pub mod biquad;
pub mod eq;
pub mod fade;
pub mod gain;
pub mod stretch;

#[cfg(test)]
mod tests;

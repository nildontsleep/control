//! Tampon circulaire un producteur / un consommateur, sans verrou ni allocation côté consommateur.
//!
//! Le thread du mélangeur y écrit le son rendu ; le rappel de la carte son le lit. Les échantillons sont rangés dans
//! des `AtomicU32` (bits des f32) : du code sûr, sans `unsafe`, et sans attente.
//!
//! « Vider » (pause, déplacement, changement de morceau) : le producteur note la position d'écriture du moment ; le
//! consommateur termine en ~3 ms de fondu ce qu'il était en train de jouer, puis saute directement au son produit
//! après la demande. La commande est ainsi entendue tout de suite, sans claquement, malgré ~150 ms d'avance.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

const NO_FLUSH: usize = usize::MAX;
/// Fondu de sortie lors d'un vidage (échantillons stéréo : 128 trames).
const FLUSH_FADE: usize = 256;

struct Shared {
    buf: Box<[AtomicU32]>,
    mask: usize,
    /// Compteurs d'échantillons, croissants (jamais remis à zéro) : `write − read` est le remplissage.
    write: AtomicUsize,
    read: AtomicUsize,
    flush: AtomicUsize,
}

pub(crate) struct Producer {
    s: Arc<Shared>,
    /// Position d'écriture lors du dernier vidage : ce qui la précède ne sera pas entendu.
    flushed_at: usize,
}

pub(crate) struct Consumer {
    s: Arc<Shared>,
    skip_to: Option<usize>,
    fade_left: usize,
}

/// Crée un tampon d'au moins `capacity` échantillons (arrondi à la puissance de deux supérieure).
pub(crate) fn ring(capacity: usize) -> (Producer, Consumer) {
    let cap = capacity.max(64).next_power_of_two();
    let buf: Box<[AtomicU32]> = (0..cap).map(|_| AtomicU32::new(0)).collect();
    let s = Arc::new(Shared {
        buf,
        mask: cap - 1,
        write: AtomicUsize::new(0),
        read: AtomicUsize::new(0),
        flush: AtomicUsize::new(NO_FLUSH),
    });
    (Producer { s: Arc::clone(&s), flushed_at: 0 }, Consumer { s, skip_to: None, fade_left: 0 })
}

impl Producer {
    pub fn capacity(&self) -> usize {
        self.s.buf.len()
    }

    /// Échantillons qui seront encore entendus avant ce qui sera écrit maintenant (un vidage demandé en retire
    /// d'emblée l'ancien son).
    pub fn fill(&self) -> usize {
        let w = self.s.write.load(Ordering::Relaxed);
        let r = self.s.read.load(Ordering::Acquire);
        w.saturating_sub(r.max(self.flushed_at))
    }

    /// Place libre (l'ancien son en attente de vidage occupe encore le tampon).
    pub fn free(&self) -> usize {
        let w = self.s.write.load(Ordering::Relaxed);
        let r = self.s.read.load(Ordering::Acquire);
        self.capacity() - w.saturating_sub(r)
    }

    /// Écrit autant d'échantillons que la place le permet ; rend le nombre écrit.
    pub fn push(&self, data: &[f32]) -> usize {
        let w = self.s.write.load(Ordering::Relaxed);
        let n = data.len().min(self.free());
        for (i, v) in data[..n].iter().enumerate() {
            self.s.buf[(w + i) & self.s.mask].store(v.to_bits(), Ordering::Relaxed);
        }
        self.s.write.store(w + n, Ordering::Release);
        n
    }

    /// Demande au consommateur d'abandonner (en fondu) ce qui a été écrit jusqu'ici.
    pub fn request_flush(&mut self) {
        let w = self.s.write.load(Ordering::Relaxed);
        self.flushed_at = w;
        self.s.flush.store(w, Ordering::Release);
    }
}

impl Consumer {
    /// Remplit `out` ; ce qui manque (sous-alimentation) est mis à zéro. Rend le nombre d'échantillons lus.
    pub fn pop(&mut self, out: &mut [f32]) -> usize {
        let f = self.s.flush.swap(NO_FLUSH, Ordering::Acquire);
        if f != NO_FLUSH {
            self.skip_to = Some(f);
            self.fade_left = FLUSH_FADE;
        }
        let mut read = self.s.read.load(Ordering::Relaxed);
        let write = self.s.write.load(Ordering::Acquire);
        let mut i = 0;
        while i < out.len() {
            if let Some(skip) = self.skip_to {
                if self.fade_left == 0 || read >= skip {
                    if read < skip {
                        read = skip.min(write);
                    }
                    self.skip_to = None;
                    continue;
                }
                if read >= write {
                    break;
                }
                let g = (self.fade_left / 2) as f32 / (FLUSH_FADE / 2) as f32;
                out[i] = f32::from_bits(self.s.buf[read & self.s.mask].load(Ordering::Relaxed)) * g;
                self.fade_left -= 1;
            } else {
                if read >= write {
                    break;
                }
                out[i] = f32::from_bits(self.s.buf[read & self.s.mask].load(Ordering::Relaxed));
            }
            read += 1;
            i += 1;
        }
        self.s.read.store(read, Ordering::Release);
        out[i..].iter_mut().for_each(|s| *s = 0.0);
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_pop_wraps_and_underruns_to_silence() {
        let (p, mut c) = ring(64);
        let data: Vec<f32> = (0..40).map(|i| i as f32).collect();
        assert_eq!(p.push(&data), 40);
        let mut out = [9.0f32; 30];
        assert_eq!(c.pop(&mut out), 30);
        assert_eq!(out[29], 29.0);
        // Rebouclage.
        assert_eq!(p.push(&data), 40);
        assert_eq!(p.fill(), 50);
        let mut out = [9.0f32; 60];
        assert_eq!(c.pop(&mut out), 50);
        assert_eq!(out[10], 0.0);
        assert_eq!(out[49], 39.0);
        assert!(out[50..].iter().all(|s| *s == 0.0));
        // Plein : on n'écrase pas ce qui n'est pas lu.
        assert_eq!(p.push(&[1.0; 100]), 64);
    }

    #[test]
    fn flush_fades_old_audio_then_jumps_to_new() {
        let (mut p, mut c) = ring(4096);
        p.push(&[1.0; 2000]);
        p.request_flush();
        assert_eq!(p.fill(), 0);
        p.push(&[0.5; 100]);
        assert_eq!(p.fill(), 100);
        let mut out = [0.0f32; 600];
        c.pop(&mut out);
        // Fondu décroissant sur l'ancien son, puis le nouveau directement.
        assert!(out[0] <= 1.0 && out[0] > 0.9);
        assert!(out.windows(2).take(FLUSH_FADE - 1).all(|w| w[1] <= w[0]));
        assert!(out[FLUSH_FADE - 1] < 0.02);
        assert_eq!(out[FLUSH_FADE], 0.5);
        assert_eq!(out[FLUSH_FADE + 99], 0.5);
        assert_eq!(out[FLUSH_FADE + 100], 0.0);
        assert_eq!(p.fill(), 0);
    }
}

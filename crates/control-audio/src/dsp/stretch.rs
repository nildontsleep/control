//! Changement de vitesse sans changement de hauteur : WSOLA (Waveform Similarity Overlap-Add).
//!
//! Le signal est découpé en trames de ~46 ms (fenêtre de Hann, recouvrement de moitié). Pour une vitesse `r`, la
//! trame `k` est prise vers la position `k · Hs · r` du signal d'entrée et posée à `k · Hs` dans la sortie ; autour de
//! la position nominale, on cherche (± ~9 ms) l'endroit qui ressemble le plus à la suite naturelle de la trame
//! précédente : les formes d'onde se raccordent en phase, sans écho ni hauteur changée. À vitesse 1, la suite
//! naturelle est toujours retenue et la reconstruction est exacte (les fenêtres de Hann se somment à 1).
//!
//! La vitesse peut changer à chaque trame : c'est ce qui permet à AutoMix d'asservir finement le morceau entrant.

/// Longueur de trame (s).
const FRAME_SECS: f64 = 0.046;

pub struct TimeStretch {
    /// Longueur de trame et pas de synthèse (trames audio).
    n: usize,
    hs: usize,
    /// Tolérance de recherche autour de la position nominale.
    tol: usize,
    window: Vec<f32>,
    /// Entrée stéréo entrelacée ; `base` est l'indice (avec le remplissage initial de `hs` trames nulles) de sa
    /// première trame.
    input: Vec<f32>,
    base: i64,
    /// Trames réelles reçues (sans le remplissage).
    real: i64,
    finished: bool,
    /// Position nominale (indice rembourré) de la prochaine trame d'analyse.
    analysis: f64,
    /// Début effectif de la trame précédente.
    prev: Option<i64>,
    /// Accumulateur du recouvrement-addition (n trames stéréo).
    ola: Vec<f32>,
    /// Trames de sortie terminées, pas encore lues.
    ready: Vec<f32>,
    ready_len: usize,
    ready_pos: usize,
    /// Position nominale (entrée, sans remplissage) de `ready[0]`, et vitesse de ce pas.
    ready_nominal: f64,
    ready_rate: f64,
    /// Sortie initiale à jeter (la première trame n'a pas de prédécesseur avec qui se recouvrir).
    discard: usize,
    mono_ref: Vec<f32>,
    mono_cand: Vec<f32>,
}

impl TimeStretch {
    pub fn new(sample_rate: u32) -> TimeStretch {
        let n = (((sample_rate as f64 * FRAME_SECS) as usize) & !1).max(64);
        let hs = n / 2;
        let window =
            (0..n).map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()) as f32).collect();
        let mut s = TimeStretch {
            n,
            hs,
            tol: n / 5,
            window,
            input: Vec::with_capacity(n * 16),
            base: 0,
            real: 0,
            finished: false,
            analysis: 0.0,
            prev: None,
            ola: vec![0.0; n * 2],
            ready: vec![0.0; hs * 2],
            ready_len: 0,
            ready_pos: 0,
            ready_nominal: 0.0,
            ready_rate: 1.0,
            discard: 0,
            mono_ref: Vec::with_capacity(hs),
            mono_cand: Vec::with_capacity(hs + 2 * n),
        };
        s.reset(1.0);
        s
    }

    /// Repart de zéro (nouveau morceau, déplacement) ; `rate` sert à placer la toute première trame.
    pub fn reset(&mut self, rate: f64) {
        self.input.clear();
        self.input.resize(self.hs * 2, 0.0);
        self.base = 0;
        self.real = 0;
        self.finished = false;
        // Trame k posée en sortie à (k − 1)·Hs (la trame 0 est jetée) et prise en entrée à Hs + (k − 1)·Hs·r.
        self.analysis = self.hs as f64 * (1.0 - rate);
        self.prev = None;
        self.ola.iter_mut().for_each(|s| *s = 0.0);
        self.ready_len = 0;
        self.ready_pos = 0;
        self.ready_nominal = 0.0;
        self.ready_rate = rate;
        self.discard = self.hs;
    }

    /// Trames d'entrée (sans remplissage) qu'il faudrait encore pour produire la prochaine trame de sortie.
    pub fn wants_input(&self) -> bool {
        !self.finished && self.ready_pos >= self.ready_len
    }

    /// Ajoute de l'entrée (stéréo entrelacée).
    pub fn push(&mut self, frames: &[f32]) {
        self.compact();
        self.input.extend_from_slice(frames);
        self.real += (frames.len() / 2) as i64;
    }

    /// Plus d'entrée : la fin est complétée par du silence et la sortie s'arrête où l'entrée s'arrêtait.
    pub fn finish(&mut self) {
        self.finished = true;
    }

    /// Toute l'entrée a été rendue.
    pub fn is_done(&self) -> bool {
        self.finished && self.ready_pos >= self.ready_len && self.next_nominal() >= self.real as f64
    }

    /// Position nominale (trames d'entrée depuis la remise à zéro) de la prochaine trame de sortie.
    pub fn position(&self) -> f64 {
        if self.discard > 0 {
            return 0.0;
        }
        let p = self.next_nominal().max(0.0);
        if self.finished { p.min(self.real as f64) } else { p }
    }

    fn next_nominal(&self) -> f64 {
        if self.ready_pos < self.ready_len {
            self.ready_nominal + self.ready_pos as f64 * self.ready_rate
        } else {
            self.analysis - self.hs as f64
        }
    }

    fn end(&self) -> i64 {
        self.base + (self.input.len() / 2) as i64
    }

    /// Retire le début de l'entrée devenu inutile (pas d'allocation : simple décalage dans le tampon).
    fn compact(&mut self) {
        let mut keep = self.analysis.floor() as i64 - self.tol as i64 - 1;
        if let Some(p) = self.prev {
            keep = keep.min(p + self.hs as i64);
        }
        let drop = keep - self.base;
        if drop > (self.n * 4) as i64 {
            let frames = (drop as usize).min(self.input.len() / 2);
            self.input.drain(..frames * 2);
            self.base += frames as i64;
        }
    }

    #[inline]
    fn sample(&self, frame: i64, ch: usize) -> f32 {
        let i = frame - self.base;
        if i < 0 {
            return 0.0;
        }
        self.input.get(i as usize * 2 + ch).copied().unwrap_or(0.0)
    }

    #[inline]
    fn mono(&self, frame: i64) -> f32 {
        self.sample(frame, 0) + self.sample(frame, 1)
    }

    /// Lit des trames de sortie à la vitesse `rate` ; rend le nombre de trames écrites (0 : il faut de l'entrée, ou
    /// c'est fini, voir `is_done`).
    pub fn pull(&mut self, out: &mut [f32], rate: f64) -> usize {
        let rate = rate.clamp(0.25, 4.0);
        let want = out.len() / 2;
        let mut done = 0;
        while done < want {
            if self.ready_pos >= self.ready_len && !self.synthesize(rate) {
                break;
            }
            // Fin du morceau : on s'arrête à la trame de sortie qui correspond à la fin de l'entrée.
            let mut avail = self.ready_len - self.ready_pos;
            if self.finished {
                let left = (self.real as f64 - (self.ready_nominal + self.ready_pos as f64 * self.ready_rate))
                    / self.ready_rate;
                let left = left.ceil().max(0.0) as usize;
                if left < avail {
                    avail = left;
                    self.ready_len = self.ready_pos + left;
                }
                if avail == 0 {
                    break;
                }
            }
            let take = avail.min(want - done);
            let from = self.ready_pos * 2;
            out[done * 2..(done + take) * 2].copy_from_slice(&self.ready[from..from + take * 2]);
            self.ready_pos += take;
            done += take;
        }
        done
    }

    /// Calcule la trame suivante ; false s'il manque de l'entrée.
    fn synthesize(&mut self, rate: f64) -> bool {
        let (n, hs, tol) = (self.n as i64, self.hs as i64, self.tol as i64);
        loop {
            if self.finished && self.discard == 0 && self.analysis - hs as f64 >= self.real as f64 {
                return false;
            }
            let center = self.analysis.round() as i64;
            let natural = self.prev.map(|p| p + hs);
            let search = natural.is_some_and(|nat| (nat - center).abs() > tol);
            let mut need = center + n;
            if search {
                need = need.max(center + tol + n);
            }
            if let Some(nat) = natural {
                need = need.max(nat + n);
            }
            if need > self.end() && !self.finished {
                return false;
            }
            let p = match natural {
                Some(nat) if search => self.best_match(center, nat),
                Some(nat) => nat,
                None => center,
            };
            // Recouvrement-addition de la trame retenue.
            for i in 0..self.n {
                let w = self.window[i];
                let f = p + i as i64;
                self.ola[i * 2] += w * self.sample(f, 0);
                self.ola[i * 2 + 1] += w * self.sample(f, 1);
            }
            let h2 = self.hs * 2;
            self.ready[..h2].copy_from_slice(&self.ola[..h2]);
            self.ola.copy_within(h2.., 0);
            let len = self.ola.len();
            self.ola[len - h2..].iter_mut().for_each(|s| *s = 0.0);
            self.ready_nominal = self.analysis - hs as f64;
            self.ready_rate = rate;
            self.ready_pos = 0;
            self.ready_len = self.hs;
            self.prev = Some(p);
            self.analysis += hs as f64 * rate;
            if self.discard > 0 {
                self.discard = 0;
                self.ready_len = 0;
                continue;
            }
            return true;
        }
    }

    /// Début, dans [centre − tol, centre + tol], de l'extrait qui prolonge le mieux la trame précédente : corrélation
    /// normalisée sur la zone de recouvrement, grossière (un point sur 4) puis affinée.
    fn best_match(&mut self, center: i64, natural: i64) -> i64 {
        let (hs, tol) = (self.hs, self.tol as i64);
        let lo = center - tol;
        let span = (2 * tol) as usize + hs + 1;
        self.mono_ref.clear();
        for i in 0..hs {
            let v = self.mono(natural + i as i64);
            self.mono_ref.push(v);
        }
        self.mono_cand.clear();
        for i in 0..span {
            let v = self.mono(lo + i as i64);
            self.mono_cand.push(v);
        }
        let score = |off: usize, step: usize, r: &[f32], c: &[f32]| -> f32 {
            let (mut xy, mut yy) = (0.0f32, 1e-9f32);
            let mut i = 0;
            while i < hs {
                let y = c[off + i];
                xy += r[i] * y;
                yy += y * y;
                i += step;
            }
            xy / yy.sqrt()
        };
        let (r, c) = (&self.mono_ref, &self.mono_cand);
        let candidates = (2 * tol) as usize;
        let mut best = 0usize;
        let mut top = f32::MIN;
        let mut off = 0;
        while off <= candidates {
            let s = score(off, 4, r, c);
            if s > top {
                top = s;
                best = off;
            }
            off += 4;
        }
        let from = best.saturating_sub(3);
        let to = (best + 3).min(candidates);
        top = f32::MIN;
        let mut fine = best;
        for off in from..=to {
            let s = score(off, 2, r, c);
            if s > top {
                top = s;
                fine = off;
            }
        }
        lo + fine as i64
    }
}

//! File de lecture : ordre de lecture, aléatoire, répétition, insertions et déplacements, session.
//!
//! Logique pure (aucune lecture audio ici) : l'interface la pilote et demande au moteur de jouer `current()`.
//! Chaque élément porte un identifiant d'entrée unique, si bien qu'un même morceau peut figurer plusieurs fois sans
//! ambiguïté (l'original comparait des références d'objets).

use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};

use crate::model::TrackId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

impl Repeat {
    pub fn parse(s: &str) -> Repeat {
        match s {
            "all" => Repeat::All,
            "one" => Repeat::One,
            _ => Repeat::Off,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Repeat::Off => "off",
            Repeat::All => "all",
            Repeat::One => "one",
        }
    }

    /// off → all → one → off.
    pub fn cycle(self) -> Repeat {
        match self {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        }
    }
}

/// Une entrée de la file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueItem {
    /// Identifiant propre à l'entrée (stable tant qu'elle reste dans la file).
    pub uid: u64,
    pub track: TrackId,
}

/// Ce qu'il faut faire après une commande.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Charger l'entrée courante et la jouer (ou pas).
    Load { autoplay: bool },
    /// Revenir au début du morceau courant.
    Restart,
    /// La file est finie : arrêter la lecture.
    Stop,
    /// Rien à faire.
    Nothing,
}

/// Session enregistrée dans les réglages (`session`), au format d'origine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub queue: Vec<TrackId>,
    pub index: i64,
    pub position: f64,
}

#[derive(Debug, Clone, Default)]
pub struct Queue {
    items: Vec<QueueItem>,
    /// Ordre d'origine (sans aléatoire), pour revenir à l'ordre quand on coupe l'aléatoire.
    original: Vec<QueueItem>,
    index: Option<usize>,
    next_uid: u64,
}

impl Queue {
    pub fn new() -> Queue {
        Queue::default()
    }

    fn make(&mut self, tracks: &[TrackId]) -> Vec<QueueItem> {
        tracks
            .iter()
            .map(|&track| {
                self.next_uid += 1;
                QueueItem { uid: self.next_uid, track }
            })
            .collect()
    }

    pub fn items(&self) -> &[QueueItem] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn index(&self) -> Option<usize> {
        self.index
    }

    pub fn current(&self) -> Option<QueueItem> {
        self.index.and_then(|i| self.items.get(i).copied())
    }

    /// Les entrées après la courante.
    pub fn upcoming(&self) -> &[QueueItem] {
        match self.index {
            Some(i) if i < self.items.len() => &self.items[i + 1..],
            _ => &self.items,
        }
    }

    /// Remplace la file. Avec l'aléatoire, le morceau choisi passe en premier et le reste est mélangé.
    pub fn play(&mut self, tracks: &[TrackId], start: usize, shuffle: bool) -> Step {
        if tracks.is_empty() {
            return Step::Nothing;
        }
        let start = start.min(tracks.len() - 1);
        let items = self.make(tracks);
        self.original = items.clone();
        if shuffle {
            let mut rest = items;
            let first = rest.remove(start);
            rest.shuffle(&mut rand::rng());
            rest.insert(0, first);
            self.items = rest;
            self.index = Some(0);
        } else {
            self.items = items;
            self.index = Some(start);
        }
        Step::Load { autoplay: true }
    }

    /// Le morceau est fini : suivant, répétition, ou fin de file.
    pub fn ended(&mut self, repeat: Repeat) -> Step {
        let Some(i) = self.index else { return Step::Nothing };
        if repeat == Repeat::One {
            return Step::Restart;
        }
        if i + 1 < self.items.len() {
            self.index = Some(i + 1);
            Step::Load { autoplay: true }
        } else if repeat == Repeat::All && !self.items.is_empty() {
            self.index = Some(0);
            Step::Load { autoplay: true }
        } else {
            Step::Stop
        }
    }

    /// Bouton « suivant » : en fin de file sans répétition, on recharge le dernier sans le jouer.
    pub fn next(&mut self, repeat: Repeat) -> Step {
        let Some(i) = self.index else { return Step::Nothing };
        if i + 1 < self.items.len() {
            self.index = Some(i + 1);
            Step::Load { autoplay: true }
        } else if repeat == Repeat::All {
            self.index = Some(0);
            Step::Load { autoplay: true }
        } else {
            Step::Load { autoplay: false }
        }
    }

    /// Bouton « précédent » : revient au début du morceau après 3 s, sinon au morceau d'avant.
    pub fn prev(&mut self, position: f64) -> Step {
        let Some(i) = self.index else { return Step::Nothing };
        if position > 3.0 || i == 0 {
            return Step::Restart;
        }
        self.index = Some(i - 1);
        Step::Load { autoplay: true }
    }

    pub fn jump_to(&mut self, i: usize) -> Step {
        if i < self.items.len() {
            self.index = Some(i);
            Step::Load { autoplay: true }
        } else {
            Step::Nothing
        }
    }

    /// L'entrée qui suivra (pour le fondu enchaîné ou le préchargement), compte tenu de la répétition.
    pub fn peek_next(&self, repeat: Repeat) -> Option<QueueItem> {
        let i = self.index?;
        match repeat {
            Repeat::One => None,
            _ if i + 1 < self.items.len() => self.items.get(i + 1).copied(),
            Repeat::All => self.items.first().copied(),
            Repeat::Off => None,
        }
    }

    /// Passe à l'entrée qui suit, déjà jouée par le moteur (fondu enchaîné ou enchaînement sans blanc).
    pub fn advance_to(&mut self, uid: u64) -> bool {
        match self.items.iter().position(|it| it.uid == uid) {
            Some(i) => {
                self.index = Some(i);
                true
            }
            None => false,
        }
    }

    /// Active ou coupe l'aléatoire en gardant le morceau courant.
    pub fn set_shuffle(&mut self, on: bool) {
        let Some(current) = self.current() else { return };
        if on {
            let mut rest: Vec<QueueItem> = self.items.iter().copied().filter(|it| it.uid != current.uid).collect();
            rest.shuffle(&mut rand::rng());
            rest.insert(0, current);
            self.items = rest;
            self.index = Some(0);
        } else {
            if !self.original.is_empty() {
                self.items = self.original.clone();
            }
            self.index = Some(self.items.iter().position(|it| it.uid == current.uid).unwrap_or(0));
        }
    }

    /// Insère juste après le morceau courant (« lire ensuite »).
    pub fn play_next(&mut self, tracks: &[TrackId], shuffle: bool) -> Step {
        if self.items.is_empty() {
            return self.play(tracks, 0, shuffle);
        }
        let new = self.make(tracks);
        let at = self.index.map_or(0, |i| i + 1);
        self.items.splice(at..at, new.iter().copied());
        let cur = self.current().and_then(|c| self.original.iter().position(|it| it.uid == c.uid));
        let at = cur.map_or(self.original.len(), |i| i + 1);
        self.original.splice(at..at, new);
        Step::Nothing
    }

    /// Ajoute à la fin de la file.
    pub fn enqueue(&mut self, tracks: &[TrackId], shuffle: bool) -> Step {
        if self.items.is_empty() {
            return self.play(tracks, 0, shuffle);
        }
        let new = self.make(tracks);
        self.items.extend(new.iter().copied());
        self.original.extend(new);
        Step::Nothing
    }

    /// Retire une entrée (pas la courante).
    pub fn remove_at(&mut self, i: usize) {
        if Some(i) == self.index || i >= self.items.len() {
            return;
        }
        let removed = self.items.remove(i);
        if let Some(cur) = self.index
            && i < cur
        {
            self.index = Some(cur - 1);
        }
        self.original.retain(|it| it.uid != removed.uid);
    }

    /// Déplace une entrée (pas la courante).
    pub fn move_item(&mut self, from: usize, to: usize) {
        if from == to || Some(from) == self.index || from >= self.items.len() {
            return;
        }
        let to = to.min(self.items.len() - 1);
        let item = self.items.remove(from);
        self.items.insert(to, item);
        if let Some(cur) = self.index {
            if from < cur && to >= cur {
                self.index = Some(cur - 1);
            } else if from > cur && to <= cur {
                self.index = Some(cur + 1);
            }
        }
    }

    /// Vide ce qui suit le morceau courant.
    pub fn clear_upcoming(&mut self) {
        if let Some(i) = self.index {
            self.items.truncate(i + 1);
        }
        let keep: Vec<u64> = self.items.iter().map(|it| it.uid).collect();
        self.original.retain(|it| keep.contains(&it.uid));
    }

    /// Vide toute la file.
    pub fn clear(&mut self) {
        self.items.clear();
        self.original.clear();
        self.index = None;
    }

    /// Retire les morceaux qui ne sont plus dans la bibliothèque. Renvoie `Load { autoplay: false }` si le morceau
    /// courant a disparu (le suivant le remplace, sans lecture).
    pub fn prune(&mut self, alive: impl Fn(TrackId) -> bool) -> Step {
        let current = self.current();
        let gone = current.is_some_and(|c| !alive(c.track));
        let old_index = self.index.unwrap_or(0);
        self.items.retain(|it| alive(it.track));
        self.original.retain(|it| alive(it.track));
        if self.items.is_empty() {
            self.index = None;
            return if gone { Step::Stop } else { Step::Nothing };
        }
        if gone {
            self.index = Some(old_index.min(self.items.len() - 1));
            return Step::Load { autoplay: false };
        }
        self.index = current.and_then(|c| self.items.iter().position(|it| it.uid == c.uid));
        Step::Nothing
    }

    pub fn session(&self, position: f64) -> Option<Session> {
        if self.items.is_empty() {
            return None;
        }
        Some(Session {
            queue: self.items.iter().map(|it| it.track).collect(),
            index: self.index.unwrap_or(0) as i64,
            position: position.max(0.0).floor(),
        })
    }

    /// Reprend une session (sans lancer la lecture). Renvoie la position où reprendre.
    pub fn restore(&mut self, session: &Session, alive: impl Fn(TrackId) -> bool) -> Option<f64> {
        let tracks: Vec<TrackId> = session.queue.iter().copied().filter(|t| alive(*t)).collect();
        if tracks.is_empty() {
            return None;
        }
        let items = self.make(&tracks);
        self.original = items.clone();
        self.items = items;
        self.index = Some((session.index.max(0) as usize).min(self.items.len() - 1));
        Some(session.position.max(0.0))
    }
}

/// Comptage d'une écoute : une lecture compte après 30 s (ou la moitié d'un morceau plus court).
#[derive(Debug, Clone, Default)]
pub struct Listen {
    pub track: Option<TrackId>,
    pub listened: f64,
    last: Option<f64>,
    pub counted: bool,
}

impl Listen {
    pub fn reset(&mut self, track: Option<TrackId>) {
        *self = Listen { track, ..Listen::default() };
    }

    /// Avance avec la position de lecture ; renvoie `true` au moment où l'écoute doit être comptée.
    pub fn tick(&mut self, position: f64, duration: f64) -> bool {
        if let Some(last) = self.last {
            let delta = position - last;
            if delta > 0.0 && delta < 1.5 {
                self.listened += delta;
            }
        }
        self.last = Some(position);
        if self.counted || self.track.is_none() {
            return false;
        }
        let threshold = if duration > 0.0 { 30f64.min(duration * 0.5) } else { 30.0 };
        if self.listened >= threshold && duration > 2.0 {
            self.counted = true;
            return true;
        }
        false
    }

    /// Une pause ou un saut : la prochaine position ne compte pas comme écoutée.
    pub fn break_continuity(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests;

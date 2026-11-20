//! Enchaînements entre platines : fondu enchaîné simple et mixage AutoMix (`maybeCrossfade`, `startCrossfade`,
//! `maybeAutomix`, `startAutomix`, `tickMix`, `finishMix`, `killFade` de `player.js`).
//!
//! Les gains suivent les mêmes courbes, mais calculées à chaque bloc de rendu (~11 ms) à partir des positions
//! réelles des platines, au lieu de minuteries : le mixage reste calé même si le rendu prend du retard.

use control_core::model::TrackId;

use super::{EngineEvent, Mixer, Prepared};
use crate::automix::{
    BASS_CUT_DB, FALLBACK_FADE, MAX_STRETCH, MIX_LEAD, MixPlan, PREPARE_AHEAD, PlanOptions, plan_mix,
};
use crate::dsp::fade::{equal_power, ramp_between, smoothstep};

/// Détente de la vitesse de l'entrant vers la vitesse générale après un mixage (s).
const EASE_SECS: f64 = 8.0;

pub(crate) enum Transition {
    Fade(Fade),
    Mix(Mix),
}

impl Transition {
    /// La platine qui joue en plus de la platine en cours.
    pub fn other_deck(&self, cur: usize) -> usize {
        match self {
            Transition::Fade(f) => f.out,
            Transition::Mix(m) => {
                if cur == m.out {
                    m.inc
                } else {
                    m.out
                }
            }
        }
    }
}

/// Fondu simple : la platine entrante est déjà la platine en cours.
pub(crate) struct Fade {
    pub out: usize,
    elapsed: u64,
    total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    /// L'entrant est positionné, à l'arrêt.
    Cued,
    /// Il tourne, muet, pour se caler.
    Rolling,
    /// Les deux morceaux sont fondus.
    Mixing,
}

/// Mixage AutoMix : la platine sortante reste la platine en cours jusqu'au milieu du mixage.
pub(crate) struct Mix {
    pub out: usize,
    pub inc: usize,
    pub plan: MixPlan,
    pub stage: Stage,
    pub switched: bool,
    lead: f64,
    /// Écart de calage lissé (s de l'entrant ; positif : l'entrant est en avance).
    pub err: f64,
    rolled: bool,
    out_id: TrackId,
    in_id: TrackId,
}

pub(crate) struct Ease {
    deck: usize,
    from: f64,
    elapsed: u64,
    total: u64,
}

enum Step {
    Continue,
    Finish,
    Abort,
}

/// Graves du sortant : coupés entre 30 et 55 % du mixage ; ceux de l'entrant reviennent entre 45 et 70 %.
fn out_bass(x: f64) -> f64 {
    ramp_between(x, 0.3, 0.0, 0.55, BASS_CUT_DB)
}

fn in_bass(x: f64) -> f64 {
    ramp_between(x, 0.45, BASS_CUT_DB, 0.7, 0.0)
}

impl Mixer {
    /// Avant le rendu d'un bloc : vitesses et gains de chaque platine pour ce bloc.
    pub(super) fn before_block(&mut self, n: usize) {
        if let Some(e) = self.ease.as_mut() {
            let k = e.elapsed as f64 / e.total as f64;
            self.decks[e.deck].speed = e.from + (self.rate - e.from) * smoothstep(k);
            e.elapsed += n as u64;
            if k >= 1.0 {
                let d = e.deck;
                self.decks[d].speed = self.rate;
                self.ease = None;
            }
        }
        let Some(mut t) = self.transition.take() else { return };
        let keep = match &mut t {
            Transition::Fade(f) => {
                let x0 = f.elapsed as f64 / f.total as f64;
                let x1 = (f.elapsed + n as u64) as f64 / f.total as f64;
                let ((o0, i0), (o1, i1)) = (equal_power(x0), equal_power(x1));
                let cur = self.cur;
                (self.decks[f.out].fade, self.decks[f.out].fade_end) = (o0, o1);
                (self.decks[cur].fade, self.decks[cur].fade_end) = (i0, i1);
                true
            }
            Transition::Mix(m) => match self.tick_mix(m, n) {
                Step::Continue => true,
                Step::Finish => {
                    self.finish_mix(m);
                    false
                }
                Step::Abort => {
                    self.transition = Some(t);
                    self.abort_transition();
                    return;
                }
            },
        };
        if keep {
            self.transition = Some(t);
        }
    }

    fn tick_mix(&mut self, m: &mut Mix, n: usize) -> Step {
        let p = m.plan;
        let rate = self.rate;
        let p_out = self.decks[m.out].position();
        let x = (p_out - p.start_out) / p.length;
        let target = p.start_in + (p_out - p.start_out) * p.ratio;
        let until_start = (p.start_out - p_out) / rate;
        if m.stage == Stage::Cued {
            if until_start > m.lead {
                self.decks[m.inc].fade = 0.0;
                self.decks[m.inc].fade_end = 0.0;
                return Step::Continue;
            }
            if let Err(message) = self.decks[m.inc].seek(target.max(0.0)) {
                self.emit(EngineEvent::Error { id: m.in_id, message });
                return Step::Abort;
            }
            self.decks[m.inc].rendering = true;
            m.stage = Stage::Rolling;
        }
        // Asservissement : l'écart lissé corrige la vitesse de ±4 % au plus, sans que cela s'entende.
        let raw = self.decks[m.inc].position() - target;
        m.err = if m.rolled { m.err * 0.6 + raw * 0.4 } else { raw };
        m.rolled = true;
        if raw.abs() > 0.25 && x < 0.0 {
            if let Err(message) = self.decks[m.inc].seek(target.max(0.0)) {
                self.emit(EngineEvent::Error { id: m.in_id, message });
                return Step::Abort;
            }
            m.err = 0.0;
        }
        self.decks[m.inc].speed = p.ratio * rate * (1.0 + (-m.err * 1.5).clamp(-0.04, 0.04));
        if m.stage == Stage::Rolling && x >= 0.0 {
            m.stage = Stage::Mixing;
            self.emit(EngineEvent::Started { id: m.in_id });
        }
        if m.stage == Stage::Mixing {
            let x1 = x + n as f64 * self.decks[m.out].speed / self.sr as f64 / p.length;
            let ((o0, i0), (o1, i1)) = (equal_power(x), equal_power(x1));
            let (out, inc) = (&mut self.decks[m.out], m.inc);
            (out.fade, out.fade_end) = (o0, o1);
            out.bass_target = out_bass(x1);
            let inc = &mut self.decks[inc];
            (inc.fade, inc.fade_end) = (i0, i1);
            inc.bass_target = in_bass(x1);
        } else {
            (self.decks[m.inc].fade, self.decks[m.inc].fade_end) = (0.0, 0.0);
            (self.decks[m.out].fade, self.decks[m.out].fade_end) = (1.0, 1.0);
        }
        if m.stage == Stage::Mixing && !m.switched && x >= 0.5 {
            self.switch_to_incoming(m);
        }
        if x >= 1.0 { Step::Finish } else { Step::Continue }
    }

    /// Au milieu du mixage, le morceau entrant devient le morceau en cours.
    fn switch_to_incoming(&mut self, m: &mut Mix) {
        m.switched = true;
        self.cur = m.inc;
        self.beat_cur = self.beat_next.take();
        self.near_end_sent = false;
        self.emit(EngineEvent::Advanced { from: m.out_id, to: m.in_id });
    }

    fn finish_mix(&mut self, m: &mut Mix) {
        if !m.switched {
            self.switch_to_incoming(m);
        }
        self.decks[m.out].unload();
        let inc = &mut self.decks[m.inc];
        inc.fade = 1.0;
        inc.fade_end = 1.0;
        inc.set_bass_now(0.0);
        let from = inc.speed;
        self.ease = Some(Ease { deck: m.inc, from, elapsed: 0, total: ((EASE_SECS * self.sr as f64) as u64).max(1) });
    }

    /// Après le rendu d'un bloc.
    pub(super) fn progress_transition(&mut self, n: usize) {
        match self.transition.as_mut() {
            Some(Transition::Fade(f)) => {
                f.elapsed += n as u64;
                if f.elapsed >= f.total {
                    let out = f.out;
                    self.transition = None;
                    self.decks[out].unload();
                    let cur = self.cur;
                    self.decks[cur].fade = 1.0;
                    self.decks[cur].fade_end = 1.0;
                }
            }
            Some(Transition::Mix(m)) => {
                // Le sortant s'est arrêté avant la fin prévue : le mixage se termine aussitôt.
                let out = m.out;
                if self.decks[out].ended
                    && let Some(Transition::Mix(mut m)) = self.transition.take()
                {
                    self.finish_mix(&mut m);
                }
            }
            None => {}
        }
    }

    /// Arrête la transition en cours (pause, déplacement, autre morceau) : seule la platine en cours reste.
    pub(super) fn abort_transition(&mut self) {
        if let Some(t) = self.transition.take() {
            match t {
                Transition::Fade(f) => self.decks[f.out].unload(),
                Transition::Mix(m) if m.switched => self.decks[m.out].unload(),
                Transition::Mix(m) => {
                    // Le mixage n'a pas abouti : on ne le retente pas pour ce morceau, mais le suivant reste prêt
                    // pour un enchaînement ordinaire.
                    if let Some(mut loaded) = self.decks[m.inc].take()
                        && self.next.is_none()
                        && let Ok(at) = loaded.source.seek(0.0)
                    {
                        self.next = Some(Prepared { loaded, at });
                    }
                    self.decks[m.inc].unload();
                    self.mix_skip = Some(m.out_id);
                }
            }
        }
        let rate = self.rate;
        for d in self.decks.iter_mut() {
            d.fade = 1.0;
            d.fade_end = 1.0;
            d.set_bass_now(0.0);
            d.speed = rate;
        }
        self.ease = None;
    }

    /// Après un bloc : faut-il commencer une transition ?
    pub(super) fn maybe_transition(&mut self) {
        if !self.playing || self.play_gain.target() < 1.0 {
            return;
        }
        let d = &self.decks[self.cur];
        if !d.rendering || d.ended || d.track.is_none() {
            return;
        }
        if !self.maybe_automix() {
            self.maybe_crossfade();
        }
    }

    /// true tant qu'AutoMix s'occupe de la fin du morceau (il attend son moment ou mixe) : pas de fondu simple.
    fn maybe_automix(&mut self) -> bool {
        match self.transition {
            Some(Transition::Mix(_)) => return true,
            Some(Transition::Fade(_)) => return false,
            None => {}
        }
        if !self.automix || self.next.is_none() {
            return false;
        }
        let d = &self.decks[self.cur];
        if d.id().is_some_and(|id| self.mix_skip == Some(id)) {
            return false;
        }
        let (Some(out), Some(inc)) = (self.beat_cur, self.beat_next) else { return false };
        let pos = d.position();
        let opts = PlanOptions { rate: self.rate, max_stretch: MAX_STRETCH, from: pos + MIX_LEAD + 1.5 };
        let Some(plan) = plan_mix(&out, &inc, opts) else { return false };
        if (plan.start_out - pos) / self.rate > PREPARE_AHEAD {
            return true; // pas encore l'heure de préparer
        }
        self.start_automix(plan);
        true
    }

    fn maybe_crossfade(&mut self) {
        if self.transition.is_some() || self.next.is_none() {
            return;
        }
        let length = if self.crossfade > 0.0 {
            self.crossfade
        } else if self.automix {
            FALLBACK_FADE
        } else {
            0.0
        };
        if length <= 0.0 {
            return;
        }
        let d = &self.decks[self.cur];
        let Some(duration) = d.duration() else { return };
        if duration < length * 2.0 + 2.0 {
            return;
        }
        // Avec AutoMix, on enchaîne sur la fin audible (sans le silence final) et on saute le silence du début du suivant.
        let end = match (self.automix, self.beat_cur) {
            (true, Some(b)) => duration.min(b.audible_end),
            _ => duration,
        };
        let remaining = (end - d.position()) / d.speed.max(0.01);
        if remaining > length + 0.05 || remaining < 0.4 {
            return;
        }
        let start_at = match (self.automix, self.beat_next) {
            (true, Some(b)) => (b.audible_start - 0.05).max(0.0),
            _ => 0.0,
        };
        self.start_crossfade(length.min(remaining), start_at);
    }

    fn start_crossfade(&mut self, seconds: f64, start_at: f64) {
        let Some(p) = self.next.take() else { return };
        let (out, inc) = (self.cur, 1 - self.cur);
        let from = self.decks[out].id();
        let to = p.loaded.id;
        let (gain, rate) = (self.gain_for(to), self.rate);
        self.decks[inc].attach(p.loaded, p.at, rate, gain);
        if (start_at - p.at).abs() > 0.01
            && let Err(message) = self.decks[inc].seek(start_at)
        {
            self.decks[inc].unload();
            self.emit(EngineEvent::Error { id: to, message });
            return;
        }
        let d = &mut self.decks[inc];
        d.rendering = true;
        d.fade = 0.0;
        d.fade_end = 0.0;
        self.cur = inc;
        self.transition =
            Some(Transition::Fade(Fade { out, elapsed: 0, total: ((seconds * self.sr as f64) as u64).max(1) }));
        self.beat_cur = self.beat_next.take();
        self.near_end_sent = false;
        self.mix_skip = None;
        self.announce = false;
        if let Some(from) = from {
            self.emit(EngineEvent::Advanced { from, to });
        }
        self.emit(EngineEvent::Started { id: to });
    }

    fn start_automix(&mut self, plan: MixPlan) {
        let Some(p) = self.next.take() else { return };
        let (out, inc) = (self.cur, 1 - self.cur);
        let Some(out_id) = self.decks[out].id() else { return };
        let in_id = p.loaded.id;
        let gain = self.gain_for(in_id);
        self.decks[inc].attach(p.loaded, p.at, plan.rate_in, gain);
        // Où démarre l'entrant, muet, pour être calé sur le début du mixage : un peu avant, sans passer sous zéro.
        let p0 = (plan.start_in - MIX_LEAD * plan.rate_in).max(0.0);
        if let Err(message) = self.decks[inc].seek(p0) {
            self.decks[inc].unload();
            self.mix_skip = Some(out_id);
            self.emit(EngineEvent::Error { id: in_id, message });
            return;
        }
        let d = &mut self.decks[inc];
        d.rendering = false;
        d.fade = 0.0;
        d.fade_end = 0.0;
        d.set_bass_now(BASS_CUT_DB);
        self.decks[out].set_bass_now(0.0);
        self.transition = Some(Transition::Mix(Mix {
            out,
            inc,
            plan,
            stage: Stage::Cued,
            switched: false,
            lead: (plan.start_in - p0) / plan.rate_in,
            err: 0.0,
            rolled: false,
            out_id,
            in_id,
        }));
    }
}

// Moteur de lecture : file d’attente, aléatoire, répétition, égaliseur, MediaSession, comptage d’écoutes.

import { settings, update, subscribe as onSettings } from './settings.js';
import { lib, bumpPlay } from './data.js';
import { planMix } from './automix/analysis.js';
import { peekAnalysis, warm } from './automix/analyzer.js';
import { mediaUrl, coverFile } from './urls.js';
import { isWeb } from './web.js';

const api = window.control?.library;
// Deux « platines » : l'élément vidéo (aussi utilisé pour l'audio et pour toutes les vidéos) et un élément audio
// de renfort. Le fondu enchaîné fait jouer la fin d'un morceau sur l'une pendant que le suivant démarre sur l'autre.
const videoEl = document.getElementById('media');
const audioEl = new Audio();
audioEl.preload = 'auto';
const decks = [videoEl, audioEl];
let media = videoEl;

export const EQ_BANDS = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
export const EQ_PRESETS = {
  flat: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  bass: [6, 5, 4, 2, 0, 0, 0, 0, 0, 0],
  treble: [0, 0, 0, 0, 0, 1, 3, 4, 5, 6],
  vocal: [-2, -2, -1, 1, 3, 3, 2, 0, -1, -2],
  rock: [4, 3, 2, -1, -2, -1, 2, 3, 4, 4],
  electronic: [5, 4, 1, 0, -2, 2, 1, 2, 4, 5],
  classical: [3, 2, 1, 1, -1, -1, 0, 2, 3, 4],
  acoustic: [4, 3, 2, 1, 1, 1, 2, 3, 3, 2],
  jazz: [3, 2, 1, 2, -2, -2, 0, 1, 2, 3],
  loudness: [6, 4, 0, 0, -1, 0, -1, 0, 4, 2],
};

const handlers = new Map();
const emit = (type, payload) => (handlers.get(type) || []).forEach((fn) => fn(payload));
export const on = (type, fn) => {
  if (!handlers.has(type)) handlers.set(type, []);
  handlers.get(type).push(fn);
  return () => handlers.set(type, handlers.get(type).filter((f) => f !== fn));
};

const state = {
  queue: [],
  original: [],
  index: -1,
  playing: false,
  loading: false,
  failures: 0,
};

// ─── Graphe audio (égaliseur) ────────────────────────────────────
let audioCtx = null;
let eqFilters = [];
let preamp = null;
const deckGain = new Map();
// Filtre de graves de chaque platine : neutre (0 dB) sauf pendant un mixage, où l'on échange les basses des deux morceaux.
const deckBass = new Map();
const BASS_CUT = -30;

// Sur un téléphone, passer par Web Audio peut couper la musique quand l'écran s'éteint (iOS surtout) : tant que ni
// l'égaliseur, ni le fondu, ni AutoMix n'en ont besoin, les éléments média jouent directement.
const nativeFirst = isWeb && typeof matchMedia === 'function' && matchMedia('(pointer: coarse)').matches;
const graphWanted = () => !nativeFirst || !!settings.eq?.enabled || Number(settings.crossfade) > 0 || !!settings.automix;

function ensureAudioGraph() {
  if (audioCtx) {
    if (audioCtx.state === 'suspended') audioCtx.resume();
    return;
  }
  if (!graphWanted()) return;
  try {
    audioCtx = new AudioContext();
    preamp = audioCtx.createGain();
    for (const el of decks) {
      const gain = audioCtx.createGain();
      const bass = audioCtx.createBiquadFilter();
      bass.type = 'lowshelf';
      bass.frequency.value = 200;
      bass.gain.value = 0;
      audioCtx.createMediaElementSource(el).connect(bass).connect(gain).connect(preamp);
      deckGain.set(el, gain);
      deckBass.set(el, bass);
    }
    eqFilters = EQ_BANDS.map((freq, i) => {
      const f = audioCtx.createBiquadFilter();
      f.type = i === 0 ? 'lowshelf' : i === EQ_BANDS.length - 1 ? 'highshelf' : 'peaking';
      f.frequency.value = freq;
      f.Q.value = 1.1;
      return f;
    });
    let node = preamp;
    for (const f of eqFilters) node = node.connect(f);
    node.connect(audioCtx.destination);
    applyEq();
  } catch (err) {
    console.warn('Graphe audio indisponible :', err);
    audioCtx = null;
  }
}

export function applyEq() {
  if (!audioCtx) return;
  const { enabled, gains } = settings.eq;
  eqFilters.forEach((f, i) => { f.gain.value = enabled ? Number(gains[i]) || 0 : 0; });
  // Compense le gain positif maximal pour éviter la saturation.
  const peak = enabled ? Math.max(0, ...gains.map(Number)) : 0;
  preamp.gain.value = 10 ** (-peak * 0.6 / 20);
}

// ─── État ────────────────────────────────────────────────────────────────────
export const player = {
  get state() {
    return state;
  },
  get track() {
    return state.queue[state.index] || null;
  },
  get time() {
    return media.currentTime || 0;
  },
  get duration() {
    return Number.isFinite(media.duration) && media.duration > 0 ? media.duration : (player.track?.duration || 0);
  },
  get playing() {
    return state.playing;
  },
  /** L'élément vidéo : c'est lui que « À l'écoute » affiche pour les vidéos. */
  get media() {
    return videoEl;
  },
  get upcoming() {
    return state.queue.slice(state.index + 1);
  },
};

const volumeCurve = (v) => v * v;
function applyVolume() {
  for (const el of decks) {
    el.volume = volumeCurve(Math.min(1, Math.max(0, settings.volume)));
    el.muted = !!settings.muted;
  }
}

function shuffled(list, keepFirst) {
  const rest = [...list];
  const first = keepFirst === undefined ? null : rest.splice(keepFirst, 1)[0];
  for (let i = rest.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [rest[i], rest[j]] = [rest[j], rest[i]];
  }
  return first ? [first, ...rest] : rest;
}

// ─── Comptage des écoutes ────────────────────────────────────────────────────
const listen = { id: null, listened: 0, last: null, playId: null, counting: null };

function resetListen(trackId = null) {
  listen.id = trackId;
  listen.listened = 0;
  listen.last = null;
  listen.playId = null;
  listen.counting = null;
}

function finishListen(completed) {
  if (listen.playId) api?.playEnd(listen.playId, Math.round(listen.listened), completed);
  resetListen();
}

function trackListen() {
  const t = media.currentTime;
  if (listen.last !== null) {
    const delta = t - listen.last;
    if (delta > 0 && delta < 1.5) listen.listened += delta;
  }
  listen.last = t;
  const track = player.track;
  if (!track || listen.playId || listen.counting) return;
  const duration = player.duration;
  const threshold = Math.min(30, duration ? duration * 0.5 : 30);
  if (listen.listened >= threshold && duration > 2) {
    const id = track.id;
    listen.counting = api.playStart(id, duration).then((playId) => {
      if (listen.id === id) listen.playId = playId;
      else api.playEnd(playId, Math.round(listen.listened), false);
      bumpPlay(id);
      emit('played', track);
    });
  }
}

// ─── Chargement et lecture ───────────────────────────────────────────────────
function load(index, { autoplay = true, position = 0 } = {}) {
  finishListen(false);
  killFade();
  cancelEase();
  mixSkip = null;
  state.index = index;
  const track = state.queue[index];
  if (!track) {
    media.removeAttribute('src');
    media.load();
    state.playing = false;
    emit('track', null);
    emit('state');
    return;
  }
  // Une vidéo s'affiche toujours dans l'élément vidéo ; l'audio reste sur la platine en cours.
  if (track.kind === 'video' && media !== videoEl) {
    media.pause();
    media.removeAttribute('src');
    media.load();
    media = videoEl;
  }
  resetListen(track.id);
  state.loading = true;
  media.src = mediaUrl(track.id);
  media.playbackRate = settings.rate;
  if (position > 0) {
    const seek = () => {
      media.currentTime = position;
      media.removeEventListener('loadedmetadata', seek);
    };
    media.addEventListener('loadedmetadata', seek);
  }
  if (autoplay) {
    ensureAudioGraph();
    media.play().catch(() => {});
  }
  updateMediaSession(track);
  emit('track', track);
  emit('state');
}

function ended() {
  finishListen(true);
  if (settings.repeat === 'one') {
    media.currentTime = 0;
    media.play().catch(() => {});
    resetListen(player.track?.id);
    return;
  }
  if (state.index < state.queue.length - 1) {
    load(state.index + 1);
  } else if (settings.repeat === 'all' && state.queue.length) {
    load(0);
  } else {
    state.playing = false;
    media.currentTime = 0;
    emit('state');
    emit('ended');
  }
}

/** Seule la platine active pilote l'interface : la platine qui s'éteint en fondu est ignorée. */
function bindDeck(el) {
  const own = (fn) => (e) => { if (e.target === media) fn(e); };
  el.addEventListener('playing', own(() => {
    state.playing = true;
    state.loading = false;
    state.failures = 0;
    listen.last = media.currentTime;
    setSessionState('playing');
    emit('state');
  }));
  el.addEventListener('pause', own(() => {
    state.playing = false;
    setSessionState('paused');
    saveSession();
    emit('state');
  }));
  el.addEventListener('waiting', own(() => { state.loading = true; emit('state'); }));
  el.addEventListener('canplay', own(() => { state.loading = false; emit('state'); }));
  el.addEventListener('ended', own(ended));
  el.addEventListener('timeupdate', own(() => {
    trackListen();
    if (!maybeAutomix()) maybeCrossfade();
    emit('time', media.currentTime);
    throttledPosition();
  }));
  el.addEventListener('durationchange', own(() => { emit('state'); throttledPosition(true); }));
  el.addEventListener('error', own(() => {
    if (!player.track) return;
    state.failures += 1;
    emit('error', player.track);
    if (state.failures < 4 && state.index < state.queue.length - 1) setTimeout(() => load(state.index + 1), 500);
    else {
      state.playing = false;
      state.loading = false;
      emit('state');
    }
  }));
}
decks.forEach(bindDeck);

// ─── Fondu enchaîné et AutoMix ───────────────────────────────────────────────
// `fade` décrit une transition en cours entre deux platines. Pour un fondu simple, la platine entrante devient la platine
// active dès le début. Pour un mixage AutoMix (`fade.mix`), la platine sortante reste active jusqu'au milieu du mixage.
let fade = null;
let rateEase = null;
let mixSkip = null; // id du morceau pour lequel un mixage a échoué : on ne réessaie pas
const FALLBACK_FADE = 6; // s : durée du fondu quand AutoMix ne peut pas caler les tempos
const MIX_LEAD = 1.2; // s : l'entrant tourne (muet) avant le début du mixage, le temps de se caler
const clamp = (v, a, b) => Math.min(b, Math.max(a, v));

function setGain(el, value) {
  const g = deckGain.get(el);
  if (!g) return;
  g.gain.cancelScheduledValues(0);
  g.gain.value = value;
}

function setBass(el, db) {
  const f = deckBass.get(el);
  if (!f) return;
  f.gain.cancelScheduledValues(0);
  f.gain.value = db;
}

/** Ramène la vitesse d'une platine à celle des réglages, en douceur (le tempo du morceau suivant se détend). */
function easeRate(el, seconds = 8) {
  clearInterval(rateEase?.timer);
  rateEase = null;
  const from = el.playbackRate;
  if (Math.abs(from - settings.rate) < 0.002) {
    el.playbackRate = settings.rate;
    return;
  }
  const t0 = performance.now();
  const timer = setInterval(() => {
    const k = Math.min(1, (performance.now() - t0) / (seconds * 1000));
    el.playbackRate = from + (settings.rate - from) * (k * k * (3 - 2 * k));
    if (k >= 1) { clearInterval(timer); if (rateEase?.timer === timer) rateEase = null; }
  }, 100);
  rateEase = { el, timer };
}

function cancelEase() {
  clearInterval(rateEase?.timer);
  rateEase = null;
}

/** Arrête une transition en cours (saut, pause, déplacement dans le morceau) : la platine qui n'est pas active se tait. */
function killFade() {
  if (!fade) return;
  clearTimeout(fade.timer);
  clearInterval(fade.tick);
  const { outgoing, incoming, mix } = fade;
  const active = media === incoming ? incoming : outgoing;
  const drop = active === incoming ? outgoing : incoming;
  fade = null;
  drop.pause();
  drop.removeAttribute('src');
  drop.load();
  for (const el of decks) {
    setGain(el, 1);
    setBass(el, 0);
  }
  cancelEase();
  active.playbackRate = settings.rate;
  if (mix?.announced) emit('mix', false);
}

function nextFadeIndex() {
  if (settings.repeat === 'one') return -1;
  if (state.index < state.queue.length - 1) return state.index + 1;
  if (settings.repeat === 'all' && state.queue.length > 1) return 0;
  return -1;
}

/** Positionne une platine à un instant donné dès que le fichier est prêt. */
function seekWhenReady(el, position) {
  if (!(position > 0)) return;
  const seek = () => {
    el.currentTime = position;
    el.removeEventListener('loadedmetadata', seek);
  };
  el.addEventListener('loadedmetadata', seek);
}

function maybeCrossfade() {
  const automix = !!settings.automix;
  const length = (Number(settings.crossfade) || 0) || (automix ? FALLBACK_FADE : 0);
  if (length <= 0 || fade || !audioCtx || !state.playing || media.seeking) return;
  const track = player.track;
  if (!track || track.kind !== 'audio') return;
  const duration = player.duration;
  if (!duration || duration < length * 2 + 2) return;
  // Avec AutoMix, on enchaîne sur la fin audible (sans le silence final) et on saute le silence du début du suivant.
  const outA = automix ? peekAnalysis(track) : null;
  const end = outA ? Math.min(duration, outA.end) : duration;
  const remaining = end - media.currentTime;
  if (remaining > length + 0.05 || remaining < 0.4) return;
  const index = nextFadeIndex();
  const next = state.queue[index];
  if (index < 0 || !next || next.kind !== 'audio') return;
  const inA = automix ? peekAnalysis(next) : null;
  startCrossfade(index, Math.min(length, remaining), inA ? Math.max(0, inA.start - 0.05) : 0);
}

function startCrossfade(index, seconds, startAt = 0) {
  const outgoing = media;
  const incoming = outgoing === videoEl ? audioEl : videoEl;
  const next = state.queue[index];
  finishListen(true);
  incoming.src = mediaUrl(next.id);
  incoming.playbackRate = settings.rate;
  seekWhenReady(incoming, startAt);
  setGain(incoming, 0);
  setGain(outgoing, 1);
  fade = { outgoing, incoming, timer: null };
  state.index = index;
  media = incoming;
  resetListen(next.id);
  state.loading = true;
  incoming.play().catch(() => {});

  // Fondu à puissance constante : la somme des deux volumes reste perçue comme stable.
  const { rise, fall } = powerCurves();
  const at = audioCtx.currentTime;
  deckGain.get(incoming).gain.setValueCurveAtTime(rise, at, seconds);
  deckGain.get(outgoing).gain.setValueCurveAtTime(fall, at, seconds);
  fade.timer = setTimeout(() => killFade(), seconds * 1000 + 200);

  updateMediaSession(next);
  emit('track', next);
  emit('state');
}

function powerCurves(steps = 64) {
  const rise = new Float32Array(steps);
  const fall = new Float32Array(steps);
  for (let i = 0; i < steps; i++) {
    const x = i / (steps - 1);
    rise[i] = Math.sin(x * Math.PI / 2);
    fall[i] = Math.cos(x * Math.PI / 2);
  }
  return { rise, fall };
}

// ─── AutoMix ─────────────────────────────────────────────────────────────────
// Le morceau suivant est accéléré ou ralenti (hauteur conservée) pour prendre le tempo du morceau en cours, entre en
// muet sur un temps de la grille, puis les deux sont fondus 8 à 16 temps en échangeant leurs basses. Pendant tout
// l'enchaînement, la position de l'entrant est asservie à celle du sortant : une petite correction de vitesse
// rattrape tout décalage (démarrage du fichier, minuterie imprécise) sans qu'on l'entende.

/** Lance l'analyse des deux morceaux concernés (sans effet si elle est déjà faite) et rend le plan de mixage s'il existe. */
function automixPlan(track, next) {
  warm(track);
  warm(next);
  const out = peekAnalysis(track);
  const inc = peekAnalysis(next);
  if (!out || !inc) return null;
  return planMix(out, inc, { rate: settings.rate, from: media.currentTime + MIX_LEAD + 1.5 });
}

/** true tant qu'AutoMix s'occupe de la fin du morceau (il attend son moment ou mixe) : pas de fondu simple. */
function maybeAutomix() {
  if (fade) return !!fade.mix;
  if (!settings.automix || !audioCtx || !state.playing || media.seeking) return false;
  const track = player.track;
  if (!track || track.kind !== 'audio' || mixSkip === track.id) return false;
  const index = nextFadeIndex();
  const next = state.queue[index];
  if (index < 0 || !next || next.kind !== 'audio') return false;
  const plan = automixPlan(track, next);
  if (!plan) return false;
  if ((plan.startOut - media.currentTime) / settings.rate > 6) return true; // pas encore l'heure de préparer
  startAutomix(index, plan);
  return true;
}

function startAutomix(index, plan) {
  const outgoing = media;
  const incoming = outgoing === videoEl ? audioEl : videoEl;
  const next = state.queue[index];
  incoming.src = mediaUrl(next.id);
  incoming.playbackRate = plan.rateIn;
  setGain(incoming, 0);
  setGain(outgoing, 1);
  setBass(incoming, BASS_CUT);
  setBass(outgoing, 0);
  // Où démarre l'entrant, muet, pour être calé sur le début du mixage : un peu avant, sans passer sous zéro.
  const p0 = Math.max(0, plan.startIn - MIX_LEAD * plan.rateIn);
  fade = {
    outgoing, incoming, timer: null, switched: false,
    mix: { plan, track: next, stage: 'prep', p0, lead: (plan.startIn - p0) / plan.rateIn, err: 0, seeked: false, announced: false },
  };
  fade.tick = setInterval(tickMix, 30);
}

function abortMix() {
  if (fade && !fade.switched) mixSkip = player.track?.id || null;
  killFade();
}

function tickMix() {
  const f = fade;
  if (!f?.mix) return;
  const m = f.mix;
  const { plan } = m;
  const { outgoing, incoming } = f;
  if (incoming.error || outgoing.error || outgoing.paused) { abortMix(); return; }
  const pO = outgoing.currentTime;
  const x = (pO - plan.startOut) / plan.length; // avancement du mixage : 0 au début, 1 à la fin
  const target = plan.startIn + (pO - plan.startOut) * plan.ratio; // où doit être l'entrant en ce moment
  const untilStart = (plan.startOut - pO) / settings.rate;

  if (m.stage === 'prep') {
    if (!m.seeked && incoming.readyState >= 1) {
      incoming.currentTime = m.p0;
      m.seeked = true;
    }
    if (m.seeked && !incoming.seeking && incoming.readyState >= 3) m.stage = 'ready';
    else if (x >= 0) { abortMix(); return; } // le fichier n'est pas arrivé à temps
  }
  if (m.stage === 'ready' && untilStart <= m.lead) {
    if (Math.abs(incoming.currentTime - target) > 0.2) incoming.currentTime = Math.max(0, target);
    incoming.play().catch(() => {});
    m.stage = 'rolling';
  }
  if (m.stage === 'ready') return;
  if (m.stage === 'prep') return;

  // Asservissement : écart signé (secondes de l'entrant), lissé ; positif = l'entrant est en avance.
  const raw = incoming.currentTime - target;
  m.err = m.rolled ? m.err * 0.6 + raw * 0.4 : raw;
  m.rolled = true;
  if (Math.abs(raw) > 0.25 && x < 0) {
    incoming.currentTime = Math.max(0, target + 0.03);
    m.err = 0;
  }
  incoming.playbackRate = plan.ratio * settings.rate * (1 + clamp(-m.err * 1.5, -0.04, 0.04));

  if (m.stage === 'rolling' && x >= 0) beginMixCurves(f);
  if (m.stage === 'mixing' && !f.switched && x >= 0.5 && !switchToIncoming(f)) return;
  if (x >= 1) finishMix(f);
}

function beginMixCurves(f) {
  const { plan } = f.mix;
  const { outgoing, incoming } = f;
  const seconds = plan.length / settings.rate;
  const at = audioCtx.currentTime;
  const { rise, fall } = powerCurves();
  deckGain.get(incoming).gain.setValueCurveAtTime(rise, at, seconds);
  deckGain.get(outgoing).gain.setValueCurveAtTime(fall, at, seconds);
  // Échange des basses : le sortant les perd au milieu du mixage pendant que l'entrant les retrouve.
  const so = deckBass.get(outgoing).gain;
  const si = deckBass.get(incoming).gain;
  so.cancelScheduledValues(0);
  si.cancelScheduledValues(0);
  so.setValueAtTime(0, at);
  so.setValueAtTime(0, at + seconds * 0.3);
  so.linearRampToValueAtTime(BASS_CUT, at + seconds * 0.55);
  si.setValueAtTime(BASS_CUT, at);
  si.setValueAtTime(BASS_CUT, at + seconds * 0.45);
  si.linearRampToValueAtTime(0, at + seconds * 0.7);
  f.mix.stage = 'mixing';
  f.mix.announced = true;
  emit('mix', true);
}

/** Au milieu du mixage, le morceau entrant devient « le morceau en cours » (pochette, titre, paroles, comptage d'écoutes). */
function switchToIncoming(f) {
  // La file a pu changer pendant le mixage : on retrouve le morceau entrant par son identifiant.
  const { id } = f.mix.track;
  let index = state.queue.findIndex((t, k) => k > state.index && t.id === id);
  if (index < 0) index = state.queue.findIndex((t) => t.id === id);
  if (index < 0) {
    abortMix();
    return false;
  }
  const next = state.queue[index];
  f.switched = true;
  finishListen(true);
  state.index = index;
  media = f.incoming;
  resetListen(next.id);
  state.loading = false;
  updateMediaSession(next);
  emit('track', next);
  emit('state');
  return true;
}

function finishMix(f) {
  if (!f.switched && !switchToIncoming(f)) return;
  clearInterval(f.tick);
  fade = null;
  const { outgoing, incoming } = f;
  outgoing.pause();
  outgoing.removeAttribute('src');
  outgoing.load();
  for (const el of decks) {
    setGain(el, 1);
    setBass(el, 0);
  }
  easeRate(incoming);
  emit('mix', false);
}

// ─── Commandes ───────────────────────────────────────────────────────────────
player.playQueue = (tracks, startIndex = 0, { shuffle } = {}) => {
  if (!tracks.length) return;
  const useShuffle = shuffle ?? settings.shuffle;
  state.original = [...tracks];
  state.queue = useShuffle ? shuffled(tracks, startIndex) : [...tracks];
  if (shuffle !== undefined && shuffle !== settings.shuffle) update({ shuffle });
  emit('queue');
  load(useShuffle ? 0 : startIndex);
};

player.playOne = (track, context) => {
  if (context?.length) player.playQueue(context, Math.max(0, context.indexOf(track)));
  else player.playQueue([track], 0);
};

player.toggle = () => {
  if (!player.track) return;
  ensureAudioGraph();
  if (media.paused) media.play().catch(() => {});
  else {
    killFade();
    media.pause();
  }
};
player.play = () => { if (player.track && media.paused) { ensureAudioGraph(); media.play().catch(() => {}); } };
player.pause = () => {
  killFade();
  media.pause();
};

/** État des platines (utile au diagnostic et aux tests). */
player.decksInfo = () => decks.map((d, i) => ({
  deck: i ? 'audio' : 'video', active: d === media, paused: d.paused, time: Math.round(d.currentTime * 10) / 10, src: d.getAttribute('src') || '',
  gain: deckGain.get(d) ? Math.round(deckGain.get(d).gain.value * 100) / 100 : null,
}));

/** Libère le fichier en cours de lecture (Windows refuse d'écrire dans un fichier ouvert) ; retourne de quoi le reprendre. */
player.suspend = (ids) => {
  const track = player.track;
  if (!track || !ids.includes(track.id)) return () => {};
  killFade();
  const resume = { position: media.currentTime || 0, playing: state.playing };
  media.pause();
  media.removeAttribute('src');
  media.load();
  return () => {
    if (player.track?.id !== track.id) return;
    load(state.index, { autoplay: resume.playing, position: resume.position });
  };
};

player.next = () => {
  if (!state.queue.length) return;
  if (state.index < state.queue.length - 1) load(state.index + 1);
  else if (settings.repeat === 'all') load(0);
  else load(state.index, { autoplay: false });
};

player.prev = () => {
  if (!state.queue.length) return;
  if (media.currentTime > 3 || state.index === 0) {
    killFade();
    media.currentTime = 0;
    return;
  }
  load(state.index - 1);
};

player.seek = (t) => {
  if (!Number.isFinite(t)) return;
  killFade();
  mixSkip = null;
  media.currentTime = Math.max(0, Math.min(player.duration || t, t));
  emit('time', media.currentTime);
};
player.seekBy = (delta) => player.seek(media.currentTime + delta);

player.jumpTo = (i) => { if (i >= 0 && i < state.queue.length) load(i); };

player.setVolume = (v) => {
  update({ volume: Math.min(1, Math.max(0, v)), muted: false });
};
player.toggleMute = () => update({ muted: !settings.muted });

player.toggleShuffle = () => {
  const on = !settings.shuffle;
  const current = player.track;
  update({ shuffle: on });
  if (!state.queue.length) return;
  if (on) {
    const rest = state.queue.filter((_, i) => i !== state.index);
    state.queue = [current, ...shuffled(rest)];
    state.index = 0;
  } else {
    state.queue = state.original.length ? [...state.original] : state.queue;
    state.index = Math.max(0, state.queue.indexOf(current));
  }
  emit('queue');
};

player.toggleAutomix = () => update({ automix: !settings.automix });

/** Mixage en cours (diagnostic et tests) : étape, écart de calage de l'entrant (s), avancement. */
player.mixInfo = () => {
  const m = fade?.mix;
  if (!m) return null;
  const pO = fade.outgoing.currentTime;
  return {
    stage: m.stage, err: Math.round(m.err * 1000) / 1000, x: Math.round(((pO - m.plan.startOut) / m.plan.length) * 1000) / 1000,
    switched: !!fade.switched, plan: m.plan,
  };
};

player.cycleRepeat = () => {
  update({ repeat: { off: 'all', all: 'one', one: 'off' }[settings.repeat] });
};

player.setRate = (r) => update({ rate: r });

// File d'attente
player.playNext = (tracks) => {
  if (!state.queue.length) return player.playQueue(tracks, 0);
  const at = state.index + 1;
  state.queue.splice(at, 0, ...tracks);
  const cur = state.original.indexOf(player.track);
  state.original.splice(cur + 1, 0, ...tracks);
  emit('queue');
};

player.enqueue = (tracks) => {
  if (!state.queue.length) return player.playQueue(tracks, 0);
  state.queue.push(...tracks);
  state.original.push(...tracks);
  emit('queue');
};

player.removeAt = (i) => {
  if (i === state.index) return;
  const [removed] = state.queue.splice(i, 1);
  if (i < state.index) state.index -= 1;
  const k = state.original.indexOf(removed);
  if (k >= 0) state.original.splice(k, 1);
  emit('queue');
};

player.move = (from, to) => {
  if (from === to || from === state.index) return;
  const [item] = state.queue.splice(from, 1);
  state.queue.splice(to, 0, item);
  if (from < state.index && to >= state.index) state.index -= 1;
  else if (from > state.index && to <= state.index) state.index += 1;
  emit('queue');
};

player.clearUpcoming = () => {
  state.queue = state.queue.slice(0, state.index + 1);
  state.original = state.original.filter((t) => state.queue.includes(t));
  emit('queue');
};

/** Retire de la file les morceaux supprimés de la bibliothèque. */
player.prune = () => {
  const alive = (t) => lib.byId.has(t.id);
  const current = player.track;
  if (current && !alive(current)) {
    media.pause();
    state.queue = state.queue.filter(alive);
    state.original = state.original.filter(alive);
    load(Math.min(state.index, state.queue.length - 1), { autoplay: false });
    return;
  }
  state.queue = state.queue.filter(alive);
  state.original = state.original.filter(alive);
  state.index = current ? state.queue.indexOf(current) : -1;
  emit('queue');
};

// Les morceaux sont rechargés à chaque mise à jour de la bibliothèque : on retrouve les nouvelles références.
player.refreshReferences = () => {
  const swap = (t) => lib.byId.get(t.id) || t;
  state.queue = state.queue.map(swap);
  state.original = state.original.map(swap);
  emit('queue');
  emit('track', player.track);
};

// ─── Session ─────────────────────────────────────────────────────────────────
function saveSession() {
  if (!state.queue.length) return;
  update({ session: { queue: state.queue.map((t) => t.id), index: state.index, position: Math.floor(media.currentTime || 0) } });
}
setInterval(() => { if (state.playing) saveSession(); }, 15000);
window.addEventListener('beforeunload', saveSession);

player.restoreSession = () => {
  const s = settings.session;
  if (!s?.queue?.length) return;
  const tracks = s.queue.map((id) => lib.byId.get(id)).filter(Boolean);
  if (!tracks.length) return;
  state.queue = tracks;
  state.original = [...tracks];
  emit('queue');
  load(Math.min(s.index, tracks.length - 1), { autoplay: false, position: s.position || 0 });
};

// ─── Réglages ────────────────────────────────────────────────────────────────
onSettings((patch) => {
  if ('volume' in patch || 'muted' in patch) applyVolume();
  if ('rate' in patch) {
    cancelEase();
    for (const el of decks) el.playbackRate = settings.rate;
  }
  if ('eq' in patch || 'crossfade' in patch || 'automix' in patch) {
    if (state.playing) ensureAudioGraph();
    if ('eq' in patch) applyEq();
  }
  emit('settings', patch);
});
applyVolume();

// ─── MediaSession (touches multimédia, panneau Windows) ──────────────────────
const session = 'mediaSession' in navigator ? navigator.mediaSession : null;

let artworkUrl = null;

function updateMediaSession(track) {
  if (!session || !track) return;
  const base = { title: track.title, artist: track.artist, album: track.album };
  session.metadata = new MediaMetadata(base);
  if (artworkUrl) {
    URL.revokeObjectURL(artworkUrl);
    artworkUrl = null;
  }
  if (!track.cover) return;
  // MediaSession n'accepte que http, data et blob : on convertit la pochette.
  fetch(coverFile(track.cover))
    .then((res) => res.blob())
    .then((blob) => {
      if (player.track?.id !== track.id) return;
      artworkUrl = URL.createObjectURL(blob);
      session.metadata = new MediaMetadata({ ...base, artwork: [{ src: artworkUrl, sizes: '640x640', type: 'image/jpeg' }] });
    })
    .catch(() => {});
}

function setSessionState(s) {
  if (session) session.playbackState = s;
}

let lastPosition = 0;
function throttledPosition(force) {
  if (!session?.setPositionState) return;
  const now = performance.now();
  if (!force && now - lastPosition < 1000) return;
  lastPosition = now;
  const duration = player.duration;
  if (!duration || !Number.isFinite(duration)) return;
  try {
    session.setPositionState({ duration, playbackRate: media.playbackRate || 1, position: Math.min(media.currentTime, duration) });
  } catch { /* valeurs incohérentes pendant un changement de piste */ }
}

if (session) {
  const action = (name, fn) => {
    try {
      session.setActionHandler(name, fn);
    } catch { /* action non prise en charge */ }
  };
  action('play', () => player.play());
  action('pause', () => player.pause());
  action('previoustrack', () => player.prev());
  action('nexttrack', () => player.next());
  action('seekto', (d) => player.seek(d.seekTime));
  action('seekbackward', (d) => player.seekBy(-(d.seekOffset || 10)));
  action('seekforward', (d) => player.seekBy(d.seekOffset || 10));
  action('stop', () => { player.pause(); player.seek(0); });
}

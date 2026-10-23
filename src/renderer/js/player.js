// Moteur de lecture : file d’attente, aléatoire, répétition, égaliseur, MediaSession, comptage d’écoutes.

import { settings, update, subscribe as onSettings } from './settings.js';
import { lib, bumpPlay } from './data.js';

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

function ensureAudioGraph() {
  if (audioCtx) {
    if (audioCtx.state === 'suspended') audioCtx.resume();
    return;
  }
  try {
    audioCtx = new AudioContext();
    preamp = audioCtx.createGain();
    for (const el of decks) {
      const gain = audioCtx.createGain();
      audioCtx.createMediaElementSource(el).connect(gain).connect(preamp);
      deckGain.set(el, gain);
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
  media.src = `app://nilcam/media/${track.id}`;
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
    maybeCrossfade();
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

// ─── Fondu enchaîné ──────────────────────────────────────────────────────────
let fade = null;

function setGain(el, value) {
  const g = deckGain.get(el);
  if (!g) return;
  g.gain.cancelScheduledValues(0);
  g.gain.value = value;
}

/** Arrête un fondu en cours (saut, pause, déplacement dans le morceau) : la platine sortante se tait tout de suite. */
function killFade() {
  if (!fade) return;
  clearTimeout(fade.timer);
  const { outgoing } = fade;
  fade = null;
  outgoing.pause();
  outgoing.removeAttribute('src');
  outgoing.load();
  for (const el of decks) setGain(el, 1);
}

function nextFadeIndex() {
  if (settings.repeat === 'one') return -1;
  if (state.index < state.queue.length - 1) return state.index + 1;
  if (settings.repeat === 'all' && state.queue.length > 1) return 0;
  return -1;
}

function maybeCrossfade() {
  const length = Number(settings.crossfade) || 0;
  if (length <= 0 || fade || !audioCtx || !state.playing || media.seeking) return;
  const track = player.track;
  if (!track || track.kind !== 'audio') return;
  const duration = player.duration;
  if (!duration || duration < length * 2 + 2) return;
  const remaining = duration - media.currentTime;
  if (remaining > length + 0.05 || remaining < 0.4) return;
  const index = nextFadeIndex();
  const next = state.queue[index];
  if (index < 0 || !next || next.kind !== 'audio') return;
  startCrossfade(index, Math.min(length, remaining));
}

function startCrossfade(index, seconds) {
  const outgoing = media;
  const incoming = outgoing === videoEl ? audioEl : videoEl;
  const next = state.queue[index];
  finishListen(true);
  incoming.src = `app://nilcam/media/${next.id}`;
  incoming.playbackRate = settings.rate;
  setGain(incoming, 0);
  setGain(outgoing, 1);
  fade = { outgoing, incoming, timer: null };
  state.index = index;
  media = incoming;
  resetListen(next.id);
  state.loading = true;
  incoming.play().catch(() => {});

  // Fondu à puissance constante : la somme des deux volumes reste perçue comme stable.
  const steps = 64;
  const rise = new Float32Array(steps);
  const fall = new Float32Array(steps);
  for (let i = 0; i < steps; i++) {
    const x = i / (steps - 1);
    rise[i] = Math.sin(x * Math.PI / 2);
    fall[i] = Math.cos(x * Math.PI / 2);
  }
  const at = audioCtx.currentTime;
  deckGain.get(incoming).gain.setValueCurveAtTime(rise, at, seconds);
  deckGain.get(outgoing).gain.setValueCurveAtTime(fall, at, seconds);
  fade.timer = setTimeout(() => killFade(), seconds * 1000 + 200);

  updateMediaSession(next);
  emit('track', next);
  emit('state');
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
  if ('rate' in patch) for (const el of decks) el.playbackRate = settings.rate;
  if ('eq' in patch) applyEq();
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
  fetch(`app://nilcam/cover/${track.cover}.jpg`)
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

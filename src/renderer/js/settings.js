// Réglages : valeurs par défaut, fusion profonde et synchronisation avec le processus principal.

export const DEFAULTS = {
  // Apparence
  theme: 'auto',
  accent: 'pink',
  customAccent: '#7c5cff',
  language: 'auto',
  animations: true,
  ambient: true,
  compact: false,

  // Bibliothèque
  libraryFolders: [],
  downloadDir: '',

  // Lecture
  volume: 0.8,
  muted: false,
  shuffle: false,
  repeat: 'off',
  rate: 1,
  eq: { enabled: false, preset: 'flat', gains: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
  session: null,

  // Vues
  sort: { songs: { key: 'title', dir: 'asc' }, albums: 'title', artists: 'name', videos: 'addedAt' },
  albumSize: 190,
  lastView: { name: 'songs', param: null },

  // Films, séries et animés
  media: {
    tmdbKey: '',
    animeTitles: 'romaji',
    streamBase: 'https://embed.vidrift.net',
    dub: false,
  },

  // Téléchargements
  dl: {
    kind: 'audio',
    audioFormat: 'mp3',
    audioQuality: '320',
    videoQuality: '1080',
    concurrency: 2,
    cookies: '',
    createPlaylist: true,
    autoUpdate: true,
  },
};

const isObj = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);

export function deepMerge(target, patch) {
  for (const [k, v] of Object.entries(patch || {})) {
    if (isObj(v) && isObj(target[k])) deepMerge(target[k], v);
    else target[k] = isObj(v) ? deepMerge({}, v) : v;
  }
  return target;
}

const clone = (v) => JSON.parse(JSON.stringify(v));
const bridge = window.control?.settings;
const LS_KEY = 'control.settings';

function readInitial() {
  if (bridge) return bridge.initial || {};
  try {
    return JSON.parse(localStorage.getItem(LS_KEY) || '{}');
  } catch {
    return {};
  }
}

const listeners = new Set();
export const settings = deepMerge(clone(DEFAULTS), readInitial());

const notify = (patch, source) => listeners.forEach((fn) => fn(patch, source));

export function update(patch, { persist = true } = {}) {
  deepMerge(settings, patch);
  if (persist) {
    if (bridge) bridge.set(patch);
    else {
      try {
        localStorage.setItem(LS_KEY, JSON.stringify(settings));
      } catch { /* stockage indisponible */ }
    }
  }
  notify(patch, 'local');
}

export function subscribe(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

function applyReset() {
  const fresh = clone(DEFAULTS);
  for (const k of Object.keys(settings)) delete settings[k];
  Object.assign(settings, fresh);
  notify(fresh, 'reset');
}

export function resetAll() {
  if (bridge) {
    bridge.reset();
    return;
  }
  try {
    localStorage.removeItem(LS_KEY);
  } catch { /* rien */ }
  applyReset();
}

if (bridge) {
  bridge.onChange((patch) => {
    deepMerge(settings, patch);
    notify(patch, 'remote');
  });
  bridge.onReset(applyReset);
}

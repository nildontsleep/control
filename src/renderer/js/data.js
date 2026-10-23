// Bibliothèque côté interface : morceaux, albums et artistes dérivés, playlists (dont intelligentes).

import { fold } from './format.js';

const api = window.control?.library;
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });
export const cmp = collator.compare;

export const lib = {
  tracks: [],
  audio: [],
  videos: [],
  byId: new Map(),
  albums: [],
  albumByKey: new Map(),
  artists: [],
  artistByKey: new Map(),
  playlists: [],
  playlistItems: new Map(),
  scan: null,
  ready: false,
};

const listeners = new Set();
export const onLibrary = (fn) => {
  listeners.add(fn);
  return () => listeners.delete(fn);
};
const notify = (what) => listeners.forEach((fn) => fn(what));

const dirOf = (p) => p.slice(0, Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\')));
const VARIOUS = Symbol('various');

function albumKeyOf(t) {
  if (!t.album) return null;
  return `${fold(t.album)}\u0000${t.albumArtist ? fold(t.albumArtist) : dirOf(t.path).toLowerCase()}`;
}

const artistKeyOf = (t) => fold(t.albumArtist || t.artist);

function derive() {
  const audio = [];
  const videos = [];
  lib.byId = new Map();
  for (const t of lib.tracks) {
    t.search = fold(`${t.title} ${t.artist} ${t.album} ${t.albumArtist} ${t.genre}`);
    lib.byId.set(t.id, t);
    (t.kind === 'video' ? videos : audio).push(t);
  }
  lib.audio = audio;
  lib.videos = videos;

  const albums = new Map();
  for (const t of audio) {
    const key = albumKeyOf(t);
    t.albumKey = key;
    if (!key) continue;
    let a = albums.get(key);
    if (!a) {
      a = { key, title: t.album, tracks: [], artists: new Set(), albumArtist: t.albumArtist, year: t.year, cover: null, addedAt: 0, duration: 0, playCount: 0 };
      albums.set(key, a);
    }
    a.tracks.push(t);
    a.artists.add(t.artist);
    a.duration += t.duration;
    a.playCount += t.playCount;
    a.addedAt = Math.max(a.addedAt, t.addedAt);
    if (t.year && (!a.year || t.year < a.year)) a.year = t.year;
    if (!a.cover && t.cover) a.cover = t.cover;
  }
  for (const a of albums.values()) {
    a.tracks.sort((x, y) => (x.discNo || 1) - (y.discNo || 1) || (x.trackNo || 0) - (y.trackNo || 0) || cmp(x.title, y.title));
    a.various = !a.albumArtist && a.artists.size > 1;
    a.artist = a.albumArtist || (a.artists.size === 1 ? [...a.artists][0] : '');
    a.artistKey = a.various ? VARIOUS : fold(a.artist);
    a.search = fold(`${a.title} ${a.artist}`);
  }
  lib.albums = [...albums.values()];
  lib.albumByKey = albums;

  const artists = new Map();
  for (const t of audio) {
    const name = t.albumArtist || t.artist;
    if (!name) continue;
    const key = artistKeyOf(t);
    let a = artists.get(key);
    if (!a) {
      a = { key, name, tracks: [], albumKeys: new Set(), cover: null, duration: 0, playCount: 0 };
      artists.set(key, a);
    }
    a.tracks.push(t);
    a.duration += t.duration;
    a.playCount += t.playCount;
    if (t.albumKey) a.albumKeys.add(t.albumKey);
    if (!a.cover && t.cover) a.cover = t.cover;
  }
  for (const a of artists.values()) {
    a.albums = [...a.albumKeys].map((k) => albums.get(k)).filter(Boolean).sort((x, y) => (y.year || 0) - (x.year || 0) || cmp(x.title, y.title));
    a.tracks.sort((x, y) => y.playCount - x.playCount || cmp(x.title, y.title));
    a.search = fold(a.name);
  }
  lib.artists = [...artists.values()];
  lib.artistByKey = artists;
}

function derivePlaylists(rows, items) {
  lib.playlists = rows.map((p) => ({ ...p, rules: p.rules ? JSON.parse(p.rules) : null }));
  lib.playlistItems = new Map();
  for (const item of items) {
    if (!lib.playlistItems.has(item.playlistId)) lib.playlistItems.set(item.playlistId, []);
    lib.playlistItems.get(item.playlistId).push(item);
  }
}

export async function loadTracks() {
  lib.tracks = await api.tracks();
  derive();
  lib.ready = true;
  notify('tracks');
}

export async function loadPlaylists() {
  const { playlists, items } = await api.playlists();
  derivePlaylists(playlists, items);
  notify('playlists');
}

export async function loadAll() {
  const [tracks, pl] = await Promise.all([api.tracks(), api.playlists()]);
  lib.tracks = tracks;
  derive();
  derivePlaylists(pl.playlists, pl.items);
  lib.ready = true;
  notify('tracks');
  notify('playlists');
}

/** Garde la copie locale à jour quand une lecture est comptée (sans tout recharger). */
export function bumpPlay(id) {
  const t = lib.byId.get(id);
  if (!t) return;
  t.playCount += 1;
  t.lastPlayed = Date.now();
}

// ─── Playlists ───────────────────────────────────────────────────────────────
export const SMART_FIELDS = [
  { id: 'playCount', type: 'number' },
  { id: 'lastPlayed', type: 'date' },
  { id: 'addedAt', type: 'date' },
  { id: 'genre', type: 'text' },
  { id: 'artist', type: 'text' },
  { id: 'album', type: 'text' },
  { id: 'title', type: 'text' },
  { id: 'year', type: 'number' },
  { id: 'duration', type: 'number' },
];

export const SMART_OPS = {
  number: ['is', 'isNot', 'gt', 'lt'],
  text: ['contains', 'notContains', 'is', 'isNot'],
  date: ['within', 'notWithin'],
};

export const SMART_PRESETS = {
  mostPlayed: { match: 'all', rules: [{ field: 'playCount', op: 'gt', value: 0 }], sort: { key: 'playCount', dir: 'desc' }, limit: 50 },
  recent: { match: 'all', rules: [{ field: 'addedAt', op: 'within', value: 30 }], sort: { key: 'addedAt', dir: 'desc' }, limit: 0 },
  neverPlayed: { match: 'all', rules: [{ field: 'playCount', op: 'is', value: 0 }], sort: { key: 'addedAt', dir: 'desc' }, limit: 0 },
  recentlyPlayed: { match: 'all', rules: [{ field: 'lastPlayed', op: 'within', value: 14 }], sort: { key: 'lastPlayed', dir: 'desc' }, limit: 0 },
  genre: { match: 'all', rules: [{ field: 'genre', op: 'contains', value: '' }], sort: { key: 'title', dir: 'asc' }, limit: 0 },
};

function testRule(t, r) {
  const field = SMART_FIELDS.find((f) => f.id === r.field);
  if (!field) return true;
  const raw = t[r.field];
  if (field.type === 'text') {
    const a = fold(raw);
    const b = fold(r.value);
    if (r.op === 'contains') return a.includes(b);
    if (r.op === 'notContains') return !a.includes(b);
    if (r.op === 'is') return a === b;
    return a !== b;
  }
  if (field.type === 'date') {
    const within = raw && Date.now() - raw <= Number(r.value || 0) * 86400000;
    return r.op === 'within' ? !!within : !within;
  }
  const v = r.field === 'duration' ? (raw || 0) / 60 : Number(raw || 0);
  const n = Number(r.value || 0);
  if (r.op === 'is') return Math.floor(v) === n;
  if (r.op === 'isNot') return Math.floor(v) !== n;
  if (r.op === 'gt') return v > n;
  return v < n;
}

export function evalSmart(playlist) {
  const spec = playlist.rules || { match: 'all', rules: [], sort: { key: 'title', dir: 'asc' }, limit: 0 };
  const rules = spec.rules.filter((r) => r.field);
  let list = lib.audio.filter((t) => (!rules.length ? true : spec.match === 'any' ? rules.some((r) => testRule(t, r)) : rules.every((r) => testRule(t, r))));
  const { key = 'title', dir = 'asc' } = spec.sort || {};
  const sign = dir === 'desc' ? -1 : 1;
  if (key === 'random') list = list.sort(() => Math.random() - 0.5);
  else if (['playCount', 'addedAt', 'lastPlayed', 'year', 'duration'].includes(key)) list = list.sort((a, b) => sign * ((a[key] || 0) - (b[key] || 0)));
  else list = list.sort((a, b) => sign * cmp(a[key] || '', b[key] || ''));
  if (spec.limit > 0) list = list.slice(0, spec.limit);
  return list;
}

/** Lignes d’une playlist : { track, entryId } (entryId seulement pour les playlists classiques). */
export function playlistRows(playlist) {
  if (playlist.kind === 'smart') return evalSmart(playlist).map((track) => ({ track }));
  return (lib.playlistItems.get(playlist.id) || [])
    .map((item) => ({ track: lib.byId.get(item.trackId), entryId: item.id }))
    .filter((row) => row.track);
}

export const playlistById = (id) => lib.playlists.find((p) => p.id === Number(id)) || null;

// ─── Recherche ───────────────────────────────────────────────────────────────
export function search(query) {
  const tokens = fold(query).split(/\s+/).filter(Boolean);
  if (!tokens.length) return null;
  const all = (text) => tokens.every((tok) => text.includes(tok));
  return {
    tracks: lib.audio.filter((t) => all(t.search)).slice(0, 200),
    albums: lib.albums.filter((a) => all(a.search)).slice(0, 40),
    artists: lib.artists.filter((a) => all(a.search)).slice(0, 40),
    videos: lib.videos.filter((t) => all(t.search)).slice(0, 40),
    playlists: lib.playlists.filter((p) => all(fold(p.name))),
  };
}

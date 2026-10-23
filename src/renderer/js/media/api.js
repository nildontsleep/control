// Films, séries et animés côté interface : appels au catalogue, ma liste et progression.

import { currentLanguage } from '../i18n.js';

const raw = window.control.media;
const watchRaw = window.control.watch;

export class MediaFail extends Error {
  constructor(code, message) {
    super(message);
    this.code = code;
  }
}

async function call(promise) {
  const res = await promise;
  if (!res.ok) throw new MediaFail(res.code, res.message);
  return res.data;
}

const lang = () => currentLanguage();

export const media = {
  home: (kind) => call(raw.home(kind, lang())),
  list: (source, page = 1) => call(raw.list(source, page, lang())),
  details: (type, id) => call(raw.details(type, String(id), lang())),
  season: (id, number) => call(raw.season(String(id), number, lang())),
  search: (query) => call(raw.search(query, lang())),
  testKey: (key) => call(raw.testKey(key)),
  clearCache: () => call(raw.clearCache()),
};

export const watch = {
  save: (rec) => call(watchRaw.save(rec)),
  show: (type, id) => call(watchRaw.show(type, String(id))),
  continue: () => call(watchRaw.continue()),
  history: () => call(watchRaw.history()),
  remove: (type, id) => call(watchRaw.remove(type, String(id))),
  clear: () => call(watchRaw.clear()),
};

// ─── Ma liste (copie locale tenue à jour) ────────────────────────────────────
let cache = null;
const listeners = new Set();

const keyOf = (type, id) => `${type}:${id}`;

export async function loadList() {
  const rows = await call(watchRaw.list());
  cache = new Map(rows.map((r) => [keyOf(r.type, r.id), r]));
  listeners.forEach((fn) => fn());
  return rows;
}

export const inList = (type, id) => !!cache?.has(keyOf(type, id));
export const listItems = () => [...(cache?.values() || [])];
export const onList = (fn) => {
  listeners.add(fn);
  return () => listeners.delete(fn);
};

export async function toggleList(item) {
  const added = await call(watchRaw.toggle({
    type: item.type, id: item.id, title: item.title, poster: item.poster, backdrop: item.backdrop, year: item.year, rating: item.rating,
  }));
  await loadList();
  return added;
}

watchRaw.onChange(({ what }) => {
  if (what === 'list') loadList();
});
loadList().catch(() => {});

// ─── Constantes partagées ────────────────────────────────────────────────────
export const TYPE_ICON = { movie: 'clapperboard', tv: 'tv', anime: 'tv-minimal' };

export const itemKey = keyOf;

/** Paramètre de route de la page de lecture : « type|id|saison|épisode ». */
export const watchParam = (type, id, season = null, episode = null) => [type, id, season ?? '', episode ?? ''].join('|');

export function parseWatchParam(param) {
  const [type, id, season, episode] = String(param || '').split('|');
  return { type, id, season: season === '' || season === undefined ? null : Number(season), episode: episode === '' || episode === undefined ? null : Number(episode) };
}

/** Épisode suivant / précédent d'un titre, d'après ses saisons (séries) ou son nombre d'épisodes (animés). */
export function neighbour(details, season, episode, dir) {
  if (!details || episode == null) return null;
  if (details.type === 'anime') {
    const count = details.episodes?.length || 0;
    const next = episode + dir;
    return next >= 1 && next <= count ? { season: null, episode: next } : null;
  }
  const seasons = details.seasons || [];
  const idx = seasons.findIndex((s) => s.number === season);
  if (idx < 0) return null;
  const next = episode + dir;
  if (next >= 1 && next <= seasons[idx].episodeCount) return { season, episode: next };
  const other = seasons[idx + dir];
  if (!other) return null;
  return { season: other.number, episode: dir > 0 ? 1 : other.episodeCount };
}

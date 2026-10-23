'use strict';

// Service « Films, séries et animés » : catalogues, progression et liste, exposés par IPC.

const { Cache } = require('./cache');
const { Tmdb } = require('./tmdb');
const { AniList } = require('./anilist');
const { Watch } = require('./watch');
const { MediaError } = require('./http');

const OFFICIAL_STREAMS = ['https://embed.vidrift.net', 'https://embed.vidrift.in'];

/** Origines que la page peut intégrer en iframe (CSP) : les deux officielles, plus un serveur de test. */
function frameOrigins() {
  const extra = process.env.CONTROL_STREAM_BASE;
  return [...OFFICIAL_STREAMS, ...(extra ? [new URL(extra).origin] : [])];
}

function imageOrigins() {
  const extra = process.env.CONTROL_TMDB_IMG;
  return extra ? [new URL(extra).origin] : [];
}

class MediaService {
  constructor({ db, store, emit }) {
    this.store = store;
    this.cache = new Cache(db);
    this.tmdb = new Tmdb({ store, cache: this.cache });
    this.anilist = new AniList({ store, cache: this.cache });
    this.watch = new Watch(db, emit);
  }

  // ─── Accueil ───────────────────────────────────────────────────────────────
  async home(kind, lang) {
    if (kind === 'anime') {
      const [home, genres] = await Promise.all([this.anilist.home(), this.anilist.genres().catch(() => [])]);
      return { ...home, genres: genres.map((g) => ({ id: g, name: g })) };
    }
    const defs = {
      all: [['trending', 'trending'], ['moviePopular', 'movie/popular'], ['tvPopular', 'tv/popular'], ['movieTop', 'movie/top_rated'], ['tvTop', 'tv/top_rated']],
      movie: [['moviePopular', 'movie/popular'], ['nowPlaying', 'movie/now_playing'], ['movieTop', 'movie/top_rated'], ['upcoming', 'movie/upcoming']],
      tv: [['tvPopular', 'tv/popular'], ['onTheAir', 'tv/on_the_air'], ['tvTop', 'tv/top_rated']],
    }[kind] || [];
    const results = await Promise.allSettled(defs.map(([, source]) => this.tmdb.list(source, 1, lang)));
    const failed = results.find((r) => r.status === 'rejected');
    if (failed && results.every((r) => r.status === 'rejected')) throw failed.reason;
    const shelves = defs.map(([id, source], i) => ({ id, source: `tmdb:${source}`, items: results[i].status === 'fulfilled' ? results[i].value.items : [] })).filter((s) => s.items.length);
    let genres = [];
    if (kind !== 'all') genres = await this.tmdb.genres(kind, lang).catch(() => []);
    return { shelves, genres };
  }

  async list(source, page, lang) {
    const [provider, ...rest] = source.split(':');
    if (provider === 'anime') return this.anilist.list(rest.join(':'), page);
    if (provider === 'tmdb') return this.tmdb.list(rest.join(':'), page, lang);
    throw new MediaError('http', `Source inconnue : ${source}`);
  }

  async details(type, id, lang) {
    if (type === 'anime') return this.anilist.details(id);
    if (type === 'movie' || type === 'tv') return this.tmdb.details(type, id, lang);
    throw new MediaError('http', `Type inconnu : ${type}`);
  }

  async search(query, lang) {
    const text = String(query || '').trim();
    if (text.length < 2) return { tmdb: [], anime: [], tmdbError: null };
    const hasKey = !!this.tmdb.key;
    const [tmdb, anime] = await Promise.allSettled([hasKey ? this.tmdb.search(text, lang) : Promise.resolve([]), this.anilist.search(text)]);
    return {
      tmdb: tmdb.status === 'fulfilled' ? tmdb.value : [],
      anime: anime.status === 'fulfilled' ? anime.value : [],
      tmdbError: hasKey ? (tmdb.status === 'rejected' ? tmdb.reason.code : null) : 'nokey',
    };
  }

  // ─── IPC ───────────────────────────────────────────────────────────────────
  register(ipcMain) {
    /** Les erreurs traversent l'IPC sous forme d'objet : { ok: false, code, message }. */
    const handle = (channel, fn) => ipcMain.handle(channel, async (event, ...args) => {
      try {
        return { ok: true, data: await fn(...args) };
      } catch (err) {
        if (!(err instanceof MediaError)) console.error(`[${channel}]`, err);
        return { ok: false, code: err.code || 'error', message: err.message || String(err) };
      }
    });

    handle('media:home', (kind, lang) => this.home(kind, lang));
    handle('media:list', (source, page, lang) => this.list(source, page, lang));
    handle('media:details', (type, id, lang) => this.details(type, id, lang));
    handle('media:season', (id, number, lang) => this.tmdb.season(id, number, lang));
    handle('media:search', (query, lang) => this.search(query, lang));
    handle('media:test-key', async (key) => this.tmdb.testKey(key));
    handle('media:clear-cache', () => this.cache.clear());

    handle('watch:save', (rec) => this.watch.save(rec));
    handle('watch:show', (type, id) => this.watch.show(type, id));
    handle('watch:continue', () => this.watch.continueList());
    handle('watch:history', () => this.watch.history());
    handle('watch:remove', (type, id) => this.watch.removeShow(type, id));
    handle('watch:clear', () => this.watch.clearHistory());
    handle('watchlist:list', () => this.watch.list());
    handle('watchlist:toggle', (item) => this.watch.toggle(item));

    // Serveur de lecture imposé par l'environnement (tests) ; sinon le réglage de l'utilisateur s'applique.
    ipcMain.on('app:env', (e) => {
      e.returnValue = { streamOverride: process.env.CONTROL_STREAM_BASE ? process.env.CONTROL_STREAM_BASE.replace(/\/$/, '') : null };
    });
  }
}

module.exports = { MediaService, frameOrigins, imageOrigins, OFFICIAL_STREAMS };

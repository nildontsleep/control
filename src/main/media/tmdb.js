'use strict';

// TMDB : films et séries (catalogue, détails, saisons, recherche). La clé est fournie par l'utilisateur.

const { fetchJson, MediaError } = require('./http');

const BASE = (process.env.CONTROL_TMDB_BASE || 'https://api.themoviedb.org/3').replace(/\/$/, '');
const IMG = (process.env.CONTROL_TMDB_IMG || 'https://image.tmdb.org/t/p').replace(/\/$/, '');

const MIN = 60 * 1000;
const HOUR = 60 * MIN;
const TTL = { list: 30 * MIN, details: 6 * HOUR, season: 6 * HOUR, genres: 7 * 24 * HOUR, search: 10 * MIN };

const LANGUAGES = { fr: 'fr-FR', en: 'en-US' };
const REGIONS = { fr: 'FR', en: 'US' };

const img = (size, p) => (p ? `${IMG}/${size}${p}` : null);
const year = (date) => (date ? Number(String(date).slice(0, 4)) || null : null);
const round1 = (n) => (n ? Math.round(n * 10) / 10 : null);

function mapItem(m, forced) {
  const type = forced || m.media_type;
  if (type !== 'movie' && type !== 'tv') return null;
  const poster = img('w342', m.poster_path);
  const backdrop = img('w1280', m.backdrop_path);
  if (!poster && !backdrop) return null;
  return {
    key: `${type}:${m.id}`,
    type,
    id: String(m.id),
    title: m.title || m.name || '',
    original: m.original_title || m.original_name || '',
    overview: m.overview || '',
    poster,
    backdrop,
    year: year(m.release_date || m.first_air_date),
    rating: round1(m.vote_average),
    genreIds: m.genre_ids || [],
  };
}

const mapList = (results, forced) => (results || []).map((m) => mapItem(m, forced)).filter(Boolean);

function pickTrailer(videos) {
  const list = (videos?.results || []).filter((v) => v.site === 'YouTube' && ['Trailer', 'Teaser'].includes(v.type));
  const score = (v) => (v.type === 'Trailer' ? 2 : 0) + (v.official ? 1 : 0);
  list.sort((a, b) => score(b) - score(a));
  return list[0]?.key || null;
}

function certification(details, type, region) {
  if (type === 'movie') {
    const entry = (details.release_dates?.results || []).find((r) => r.iso_3166_1 === region);
    return (entry?.release_dates || []).map((d) => d.certification).find(Boolean) || null;
  }
  return (details.content_ratings?.results || []).find((r) => r.iso_3166_1 === region)?.rating || null;
}

class Tmdb {
  constructor({ store, cache }) {
    this.store = store;
    this.cache = cache;
  }

  get key() {
    return String(this.store.get('media', {})?.tmdbKey || '').trim();
  }

  async #request(path, params = {}, { key = this.key } = {}) {
    if (!key) throw new MediaError('nokey', 'Aucune clé TMDB');
    const url = new URL(`${BASE}${path}`);
    const headers = { accept: 'application/json' };
    // Un « jeton d'accès » v4 est un JWT : il passe en en-tête, pas en paramètre.
    if (key.startsWith('eyJ')) headers.authorization = `Bearer ${key}`;
    else url.searchParams.set('api_key', key);
    for (const [k, v] of Object.entries(params)) if (v !== undefined && v !== null && v !== '') url.searchParams.set(k, String(v));
    return fetchJson(url.toString(), { headers });
  }

  #memo(path, params, ttl, mapper) {
    const sorted = Object.keys(params).sort().map((k) => `${k}=${params[k]}`).join('&');
    return this.cache.memo(`tmdb:${path}?${sorted}`, ttl, async () => mapper(await this.#request(path, params)));
  }

  /** Teste une clé sans la mémoriser. */
  async testKey(key) {
    await this.#request('/authentication', {}, { key: String(key || '').trim() });
    return true;
  }

  async list(source, page = 1, lang = 'fr') {
    const language = LANGUAGES[lang] || LANGUAGES.fr;
    let path;
    let forced;
    const extra = {};
    const [kind, a, b] = source.split('/');
    if (kind === 'trending') {
      path = '/trending/all/week';
    } else if (kind === 'discover') {
      forced = a;
      path = `/discover/${a}`;
      extra.with_genres = b;
      extra.sort_by = 'popularity.desc';
      extra['vote_count.gte'] = 30;
      extra.include_adult = false;
    } else if (['movie', 'tv'].includes(kind) && /^[a-z_]+$/.test(a || '')) {
      forced = kind;
      path = `/${kind}/${a}`;
    } else {
      throw new MediaError('http', `Source inconnue : ${source}`);
    }
    return this.#memo(path, { language, page, ...extra }, TTL.list, (d) => ({
      items: mapList(d.results, forced),
      page: d.page || page,
      totalPages: Math.min(d.total_pages || 1, 500),
    }));
  }

  async genres(type, lang = 'fr') {
    const language = LANGUAGES[lang] || LANGUAGES.fr;
    return this.#memo(`/genre/${type}/list`, { language }, TTL.genres, (d) => d.genres || []);
  }

  async search(query, lang = 'fr') {
    const language = LANGUAGES[lang] || LANGUAGES.fr;
    return this.#memo('/search/multi', { language, query: query.trim(), include_adult: false, page: 1 }, TTL.search, (d) => mapList(d.results));
  }

  async details(type, id, lang = 'fr') {
    const language = LANGUAGES[lang] || LANGUAGES.fr;
    const region = REGIONS[lang] || 'FR';
    const append = `credits,videos,recommendations,similar,${type === 'movie' ? 'release_dates' : 'content_ratings'}`;
    return this.#memo(`/${type}/${id}`, {
      language, append_to_response: append, include_video_language: `${lang},en,null`,
    }, TTL.details, (d) => {
      const base = mapItem(d, type) || {
        key: `${type}:${d.id}`, type, id: String(d.id), title: d.title || d.name || '', overview: d.overview || '',
        poster: null, backdrop: null, year: year(d.release_date || d.first_air_date), rating: round1(d.vote_average), genreIds: [],
      };
      const crew = d.credits?.crew || [];
      const creators = type === 'movie'
        ? crew.filter((c) => c.job === 'Director').map((c) => c.name)
        : (d.created_by || []).map((c) => c.name);
      return {
        ...base,
        tagline: d.tagline || '',
        runtime: d.runtime || d.episode_run_time?.[0] || null,
        genres: (d.genres || []).map((g) => g.name),
        status: d.status || '',
        releaseDate: d.release_date || d.first_air_date || '',
        numberOfSeasons: d.number_of_seasons || null,
        numberOfEpisodes: d.number_of_episodes || null,
        creators: creators.slice(0, 3),
        certification: certification(d, type, region),
        trailer: pickTrailer(d.videos),
        cast: (d.credits?.cast || []).slice(0, 18).map((c) => ({ id: c.id, name: c.name, role: c.character || '', profile: img('w185', c.profile_path) })),
        seasons: (d.seasons || []).filter((s) => s.season_number > 0).map((s) => ({
          number: s.season_number, name: s.name, overview: s.overview || '', poster: img('w342', s.poster_path),
          airDate: s.air_date || '', episodeCount: s.episode_count || 0,
        })),
        recommendations: mapList(d.recommendations?.results, type),
        similar: mapList(d.similar?.results, type),
      };
    });
  }

  async season(id, number, lang = 'fr') {
    const language = LANGUAGES[lang] || LANGUAGES.fr;
    return this.#memo(`/tv/${id}/season/${number}`, { language }, TTL.season, (d) => ({
      number: d.season_number,
      name: d.name || '',
      overview: d.overview || '',
      episodes: (d.episodes || []).map((e) => ({
        number: e.episode_number,
        name: e.name || '',
        overview: e.overview || '',
        still: img('w300', e.still_path),
        airDate: e.air_date || '',
        runtime: e.runtime || null,
        rating: round1(e.vote_average),
      })),
    }));
  }
}

module.exports = { Tmdb };

'use strict';

// AniList : catalogue d'animés (API GraphQL publique, sans clé).

const { fetchJson, MediaError } = require('./http');

const URL_API = process.env.CONTROL_ANILIST_URL || 'https://graphql.anilist.co';

const MIN = 60 * 1000;
const HOUR = 60 * MIN;
const TTL = { list: 30 * MIN, details: 6 * HOUR, search: 10 * MIN };

const FRAGMENT = `
fragment M on Media {
  id idMal
  title { romaji english native }
  coverImage { extraLarge large color }
  bannerImage
  description(asHtml: false)
  episodes format status season seasonYear averageScore genres
  startDate { year }
  nextAiringEpisode { episode airingAt }
}`;

const entities = { '&amp;': '&', '&lt;': '<', '&gt;': '>', '&quot;': '"', '&#039;': "'", '&nbsp;': ' ' };

function clean(text) {
  return String(text || '')
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/<[^>]+>/g, '')
    .replace(/&(amp|lt|gt|quot|#039|nbsp);/g, (m) => entities[m])
    .replace(/\(Source:[^)]*\)/gi, '')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

function currentSeason(now = new Date()) {
  const m = now.getMonth();
  return { season: m < 3 ? 'WINTER' : m < 6 ? 'SPRING' : m < 9 ? 'SUMMER' : 'FALL', year: now.getFullYear() };
}

function pickTitle(t, pref) {
  const order = pref === 'english' ? [t.english, t.romaji, t.native] : [t.romaji, t.english, t.native];
  return order.find(Boolean) || '';
}

/** Nombre d'épisodes déjà sortis. */
function availableEpisodes(m) {
  const next = m.nextAiringEpisode?.episode;
  if (next) return Math.max(0, next - 1);
  return m.episodes || null;
}

function mapAnime(m, pref) {
  if (!m) return null;
  const t = m.title || {};
  return {
    key: `anime:${m.id}`,
    type: 'anime',
    id: String(m.id),
    title: pickTitle(t, pref),
    original: t.native || '',
    overview: clean(m.description),
    poster: m.coverImage?.extraLarge || m.coverImage?.large || null,
    backdrop: m.bannerImage || null,
    year: m.seasonYear || m.startDate?.year || null,
    rating: m.averageScore ? Math.round(m.averageScore) / 10 : null,
    genres: m.genres || [],
    format: m.format || '',
    status: m.status || '',
    episodes: m.episodes || null,
    available: availableEpisodes(m),
    color: m.coverImage?.color || null,
    idMal: m.idMal || null,
  };
}

const NAMED = {
  trending: 'sort: TRENDING_DESC',
  popular: 'sort: POPULARITY_DESC',
  top: 'sort: SCORE_DESC',
  movies: 'format: MOVIE, sort: POPULARITY_DESC',
};

class AniList {
  constructor({ store, cache }) {
    this.store = store;
    this.cache = cache;
  }

  get pref() {
    return this.store.get('media', {})?.animeTitles === 'english' ? 'english' : 'romaji';
  }

  async #query(query, variables = {}) {
    const data = await fetchJson(URL_API, {
      method: 'POST',
      headers: { 'content-type': 'application/json', accept: 'application/json' },
      body: JSON.stringify({ query, variables }),
    });
    if (data?.errors?.length) throw new MediaError('http', data.errors[0].message);
    return data?.data;
  }

  #memo(key, ttl, fn) {
    return this.cache.memo(`anilist:${this.pref}:${key}`, ttl, fn);
  }

  /** Les rangées de l'accueil en une seule requête (AniList limite à 90 requêtes par minute). */
  async home() {
    const { season, year } = currentSeason();
    return this.#memo(`home:${season}${year}`, TTL.list, async () => {
      const page = (alias, args) => `${alias}: Page(perPage: 20) { media(type: ANIME, isAdult: false, ${args}) { ...M } }`;
      const query = `query Home($season: MediaSeason, $year: Int) {
        ${page('trending', NAMED.trending)}
        ${page('season', 'season: $season, seasonYear: $year, sort: POPULARITY_DESC')}
        ${page('popular', NAMED.popular)}
        ${page('top', NAMED.top)}
        ${page('movies', NAMED.movies)}
      } ${FRAGMENT}`;
      const d = await this.#query(query, { season, year });
      const shelf = (id, source) => ({ id, source: `anime:${source}`, items: (d[id]?.media || []).map((m) => mapAnime(m, this.pref)).filter((a) => a.poster) });
      return { shelves: [shelf('trending', 'trending'), shelf('season', 'season'), shelf('popular', 'popular'), shelf('top', 'top'), shelf('movies', 'movies')] };
    });
  }

  async list(source, page = 1) {
    const [kind, arg] = source.split(':');
    let args;
    const vars = { page };
    if (kind === 'season') {
      const { season, year } = currentSeason();
      args = 'season: $season, seasonYear: $year, sort: POPULARITY_DESC';
      Object.assign(vars, { season, year });
    } else if (kind === 'genre') {
      args = 'genre: $genre, sort: POPULARITY_DESC';
      vars.genre = arg;
    } else if (NAMED[kind]) {
      args = NAMED[kind];
    } else {
      throw new MediaError('http', `Source inconnue : ${source}`);
    }
    const declare = ['$page: Int', kind === 'season' ? '$season: MediaSeason, $year: Int' : '', kind === 'genre' ? '$genre: String' : ''].filter(Boolean).join(', ');
    const query = `query List(${declare}) {
      Page(page: $page, perPage: 24) {
        pageInfo { currentPage lastPage hasNextPage }
        media(type: ANIME, isAdult: false, ${args}) { ...M }
      }
    } ${FRAGMENT}`;
    return this.#memo(`list:${source}:${page}`, TTL.list, async () => {
      const d = (await this.#query(query, vars)).Page;
      return {
        items: (d.media || []).map((m) => mapAnime(m, this.pref)).filter((a) => a.poster),
        page: d.pageInfo?.currentPage || page,
        totalPages: d.pageInfo?.lastPage || 1,
      };
    });
  }

  async genres() {
    return this.#memo('genres', 7 * 24 * HOUR, async () => {
      const d = await this.#query('{ GenreCollection }');
      return (d.GenreCollection || []).filter((g) => g !== 'Hentai');
    });
  }

  async search(query) {
    const text = query.trim();
    return this.#memo(`search:${text.toLowerCase()}`, TTL.search, async () => {
      const q = `query Search($q: String) {
        Page(perPage: 20) { media(type: ANIME, isAdult: false, search: $q, sort: SEARCH_MATCH) { ...M } }
      } ${FRAGMENT}`;
      const d = await this.#query(q, { q: text });
      return (d.Page.media || []).map((m) => mapAnime(m, this.pref)).filter((a) => a.poster);
    });
  }

  async details(id) {
    const numeric = Number(id);
    if (!Number.isInteger(numeric)) throw new MediaError('notfound', 'Identifiant invalide');
    return this.#memo(`details:${numeric}`, TTL.details, async () => {
      const query = `query Details($id: Int) {
        Media(id: $id, type: ANIME) {
          ...M
          duration
          trailer { id site }
          studios(isMain: true) { nodes { name } }
          characters(sort: [ROLE, RELEVANCE], perPage: 14) {
            edges { role node { id name { full } image { medium } } }
          }
          streamingEpisodes { title thumbnail }
          relations { edges { relationType(version: 2) node { ...M type } } }
          recommendations(sort: RATING_DESC, perPage: 14) { nodes { mediaRecommendation { ...M } } }
        }
      } ${FRAGMENT}`;
      const m = (await this.#query(query, { id: numeric })).Media;
      if (!m) throw new MediaError('notfound', 'Animé introuvable');
      const base = mapAnime(m, this.pref);
      const isMovie = m.format === 'MOVIE';
      let count = base.available;
      if (!count && !isMovie) count = m.streamingEpisodes?.length || 12;

      const named = new Map();
      for (const e of m.streamingEpisodes || []) {
        const hit = /^(?:Episode|Épisode)\s+(\d+)\s*[-–:]\s*(.*)$/i.exec(e.title || '');
        if (hit) named.set(Number(hit[1]), { name: hit[2], still: e.thumbnail || null });
      }
      const episodes = isMovie ? [] : Array.from({ length: count }, (_, i) => ({
        number: i + 1,
        name: named.get(i + 1)?.name || '',
        still: named.get(i + 1)?.still || null,
      }));

      // Les séries très longues ont des dizaines de titres liés : on garde les suites et préquelles d'abord.
      const order = ['PREQUEL', 'SEQUEL', 'PARENT', 'SUMMARY', 'SIDE_STORY', 'SPIN_OFF', 'ALTERNATIVE'];
      const related = (m.relations?.edges || [])
        .filter((e) => e.node?.type === 'ANIME' && order.includes(e.relationType))
        .sort((a, b) => order.indexOf(a.relationType) - order.indexOf(b.relationType))
        .map((e) => ({ relation: e.relationType, ...mapAnime(e.node, this.pref) }))
        .filter((a) => a.poster)
        .slice(0, 14);

      return {
        ...base,
        tagline: '',
        runtime: m.duration || null,
        isMovie,
        studios: (m.studios?.nodes || []).map((s) => s.name),
        trailer: m.trailer?.site === 'youtube' ? m.trailer.id : null,
        cast: (m.characters?.edges || []).map((e) => ({ id: e.node.id, name: e.node.name.full, role: e.role === 'MAIN' ? 'Principal' : 'Secondaire', profile: e.node.image?.medium || null })),
        episodes,
        related,
        recommendations: (m.recommendations?.nodes || []).map((n) => mapAnime(n.mediaRecommendation, this.pref)).filter((a) => a && a.poster),
        nextAiring: m.nextAiringEpisode ? { episode: m.nextAiringEpisode.episode, at: m.nextAiringEpisode.airingAt * 1000 } : null,
      };
    });
  }
}

module.exports = { AniList, mapAnime, clean };

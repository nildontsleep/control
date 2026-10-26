'use strict';

// Comparaison de deux récaps (le résultat de Stats.get de chaque compte, sur la même période) : totaux côte à côte, tops,
// goûts communs et un indice de ressemblance. Ne renvoie que des agrégats, jamais l'historique des écoutes.

const fold = (s) => String(s || '').normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase().replace(/\s+/g, ' ').trim();

/** Similarité cosinus de deux « sacs de comptes » (Map clé → poids) : 0 (rien en commun) à 1 (mêmes proportions). */
function cosine(a, b) {
  let dot = 0;
  let na = 0;
  let nb = 0;
  for (const v of a.values()) na += v * v;
  for (const v of b.values()) nb += v * v;
  for (const [k, v] of a) if (b.has(k)) dot += v * b.get(k);
  return na && nb ? dot / Math.sqrt(na * nb) : 0;
}

const bag = (rows, keyOf, weightOf) => {
  const out = new Map();
  for (const r of rows) {
    const key = keyOf(r);
    if (key) out.set(key, (out.get(key) || 0) + weightOf(r));
  }
  return out;
};

/** Ce qu'on garde d'un récap pour l'affichage. */
function summary(stats, top = 10) {
  const m = stats.music || {};
  return {
    range: stats.range,
    plays: m.plays || 0,
    seconds: Math.round(m.seconds || 0),
    tracks: m.tracks || 0,
    artists: m.artists || 0,
    albums: m.albums || 0,
    completed: m.completed || 0,
    streak: stats.streak || { current: 0, longest: 0 },
    video: { seconds: Math.round(stats.video?.seconds || 0), titles: stats.video?.titles || 0, finished: stats.video?.finished || 0 },
    topArtists: (m.topArtists || []).slice(0, top).map(({ name, plays, seconds, cover }) => ({ name, plays, seconds: Math.round(seconds || 0), cover })),
    topTracks: (m.topTracks || []).slice(0, top).map(({ title, artist, album, cover, plays }) => ({ title, artist, album, cover, plays })),
    topAlbums: (m.topAlbums || []).slice(0, 6).map(({ album, artist, cover, plays }) => ({ album, artist, cover, plays })),
    genres: (m.genres || []).slice(0, 6).map(({ name, plays, seconds }) => ({ name, plays, seconds: Math.round(seconds || 0) })),
    byWeekday: m.byWeekday || Array(7).fill(0),
    byHour: m.byHour || Array(24).fill(0),
  };
}

/**
 * a, b : résultats de Stats.get (idéalement avec un `limit` large, pour que les recoupements soient fiables).
 * Renvoie { me, other, shared, score } : score de 0 à 100.
 */
function compare(a, b) {
  const ma = a.music || {};
  const mb = b.music || {};
  const artistKey = (r) => fold(r.name);
  const trackKey = (r) => (r.title ? `${fold(r.artist)}|${fold(r.title)}` : '');
  const genreKey = (r) => fold(r.name);

  const artistsA = bag(ma.topArtists || [], artistKey, (r) => r.plays);
  const artistsB = bag(mb.topArtists || [], artistKey, (r) => r.plays);
  const tracksA = bag(ma.topTracks || [], trackKey, (r) => r.plays);
  const tracksB = bag(mb.topTracks || [], trackKey, (r) => r.plays);
  const genresA = bag(ma.genres || [], genreKey, (r) => r.plays);
  const genresB = bag(mb.genres || [], genreKey, (r) => r.plays);

  const sharedArtists = (ma.topArtists || []).filter((r) => artistsB.has(artistKey(r)))
    .map((r) => ({ name: r.name, cover: r.cover, mine: r.plays, theirs: artistsB.get(artistKey(r)) }))
    .sort((x, y) => (y.mine + y.theirs) - (x.mine + x.theirs)).slice(0, 12);
  const sharedTracks = (ma.topTracks || []).filter((r) => tracksB.has(trackKey(r)))
    .map((r) => ({ title: r.title, artist: r.artist, cover: r.cover, mine: r.plays, theirs: tracksB.get(trackKey(r)) }))
    .sort((x, y) => (y.mine + y.theirs) - (x.mine + x.theirs)).slice(0, 8);
  const sharedGenres = (ma.genres || []).filter((r) => genresB.has(genreKey(r)))
    .map((r) => ({ name: r.name, mine: r.plays, theirs: genresB.get(genreKey(r)) }))
    .sort((x, y) => (y.mine + y.theirs) - (x.mine + x.theirs)).slice(0, 6);

  // Pas d'écoute d'un côté : rien à comparer, l'indice reste à 0.
  const enough = (ma.plays || 0) > 0 && (mb.plays || 0) > 0;
  const score = enough
    ? Math.round(100 * (0.55 * cosine(artistsA, artistsB) + 0.3 * cosine(genresA, genresB) + 0.15 * cosine(tracksA, tracksB)))
    : 0;

  return {
    me: summary(a),
    other: summary(b),
    shared: { artists: sharedArtists, tracks: sharedTracks, genres: sharedGenres },
    score,
  };
}

module.exports = { compare, summary, cosine, fold };

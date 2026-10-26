'use strict';

// Statistiques d'écoute et de visionnage, calculées à la demande depuis la base (plays, watch_time).

const { plain } = require('./db');

const DAY = 24 * 3600 * 1000;
// Une lecture n'est enregistrée qu'après 30 s d'écoute : à défaut de durée exacte, c'est le minimum sûr.
const LISTENED = 'COALESCE(NULLIF(p.seconds, 0), MIN(p.duration, 30))';
const LOCAL = "'unixepoch', 'localtime'";

const dayKey = (date) => {
  const p = (n) => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${p(date.getMonth() + 1)}-${p(date.getDate())}`;
};

function boundsOf(from, to) {
  const parse = (s, fallback) => {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(String(s || ''));
    return m ? new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])) : fallback;
  };
  const start = parse(from, new Date(2000, 0, 1));
  const end = parse(to, new Date());
  const endExclusive = new Date(end.getFullYear(), end.getMonth(), end.getDate() + 1);
  return { start: start.getTime(), end: endExclusive.getTime(), fromDay: dayKey(start), toDay: dayKey(end) };
}

/** Plus longue série de jours consécutifs, et série en cours (aujourd'hui ou hier compte). */
function streaks(days) {
  const sorted = [...new Set(days)].sort();
  if (!sorted.length) return { current: 0, longest: 0 };
  const toTime = (s) => { const [y, m, d] = s.split('-').map(Number); return new Date(y, m - 1, d).getTime(); };
  let longest = 1;
  let run = 1;
  for (let i = 1; i < sorted.length; i++) {
    const gap = Math.round((toTime(sorted[i]) - toTime(sorted[i - 1])) / DAY);
    run = gap === 1 ? run + 1 : 1;
    longest = Math.max(longest, run);
  }
  const today = dayKey(new Date());
  const yesterday = dayKey(new Date(Date.now() - DAY));
  const last = sorted[sorted.length - 1];
  let current = 0;
  if (last === today || last === yesterday) {
    current = 1;
    for (let i = sorted.length - 1; i > 0; i--) {
      if (Math.round((toTime(sorted[i]) - toTime(sorted[i - 1])) / DAY) === 1) current++;
      else break;
    }
  }
  return { current, longest };
}

class Stats {
  constructor(db) {
    this.db = db;
  }

  get({ from, to } = {}) {
    const { start, end, fromDay, toDay } = boundsOf(from, to);
    const q = (sql, ...params) => this.db.prepare(sql).all(...params);
    const one = (sql, ...params) => this.db.prepare(sql).get(...params);
    const audio = "t.kind = 'audio' AND p.played_at >= ? AND p.played_at < ?";
    const artistExpr = "COALESCE(NULLIF(t.album_artist, ''), NULLIF(t.artist, ''))";

    const totals = plain([one(`
      SELECT COUNT(*) AS plays, COALESCE(SUM(${LISTENED}), 0) AS seconds, COUNT(DISTINCT t.id) AS tracks,
             COUNT(DISTINCT ${artistExpr}) AS artists, COUNT(DISTINCT NULLIF(t.album, '')) AS albums,
             COALESCE(SUM(p.completed), 0) AS completed
      FROM plays p JOIN tracks t ON t.id = p.track_id WHERE ${audio}`, start, end)])[0];

    const topTracks = plain(q(`
      SELECT t.id, t.title, t.artist, t.album, t.cover, COUNT(*) AS plays, SUM(${LISTENED}) AS seconds
      FROM plays p JOIN tracks t ON t.id = p.track_id WHERE ${audio}
      GROUP BY t.id ORDER BY plays DESC, seconds DESC, t.title LIMIT 10`, start, end));

    const topArtists = plain(q(`
      SELECT ${artistExpr} AS name, COUNT(*) AS plays, SUM(${LISTENED}) AS seconds, COUNT(DISTINCT t.id) AS tracks,
             (SELECT x.cover FROM tracks x WHERE COALESCE(NULLIF(x.album_artist, ''), NULLIF(x.artist, '')) = ${artistExpr}
                AND x.cover IS NOT NULL ORDER BY x.play_count DESC LIMIT 1) AS cover
      FROM plays p JOIN tracks t ON t.id = p.track_id WHERE ${audio} AND ${artistExpr} IS NOT NULL
      GROUP BY name ORDER BY plays DESC, seconds DESC, name LIMIT 10`, start, end));

    const topAlbums = plain(q(`
      SELECT t.album AS album, ${artistExpr} AS artist, COUNT(*) AS plays, SUM(${LISTENED}) AS seconds,
             MAX(t.cover) AS cover, MIN(t.id) AS trackId
      FROM plays p JOIN tracks t ON t.id = p.track_id WHERE ${audio} AND t.album <> ''
      GROUP BY t.album, artist ORDER BY plays DESC, seconds DESC, album LIMIT 10`, start, end));

    const genreRows = q(`
      SELECT t.genre AS genre, COUNT(*) AS plays, SUM(${LISTENED}) AS seconds
      FROM plays p JOIN tracks t ON t.id = p.track_id WHERE ${audio} AND t.genre <> '' GROUP BY t.genre`, start, end);
    const genres = new Map();
    for (const row of genreRows) {
      for (const g of String(row.genre).split(/[,;/]/).map((x) => x.trim()).filter(Boolean)) {
        const cur = genres.get(g.toLowerCase()) || { name: g, plays: 0, seconds: 0 };
        cur.plays += row.plays;
        cur.seconds += row.seconds;
        genres.set(g.toLowerCase(), cur);
      }
    }

    const byDay = plain(q(`
      SELECT strftime('%Y-%m-%d', p.played_at / 1000, ${LOCAL}) AS day, COUNT(*) AS plays, SUM(${LISTENED}) AS seconds
      FROM plays p JOIN tracks t ON t.id = p.track_id WHERE ${audio} GROUP BY day ORDER BY day`, start, end));

    const byHour = Array(24).fill(0);
    for (const r of q(`
      SELECT CAST(strftime('%H', p.played_at / 1000, ${LOCAL}) AS INTEGER) AS h, SUM(${LISTENED}) AS seconds
      FROM plays p JOIN tracks t ON t.id = p.track_id WHERE ${audio} GROUP BY h`, start, end)) byHour[r.h] = r.seconds;

    const byWeekday = Array(7).fill(0); // lundi → dimanche
    for (const r of q(`
      SELECT CAST(strftime('%w', p.played_at / 1000, ${LOCAL}) AS INTEGER) AS d, SUM(${LISTENED}) AS seconds
      FROM plays p JOIN tracks t ON t.id = p.track_id WHERE ${audio} GROUP BY d`, start, end)) byWeekday[(r.d + 6) % 7] = r.seconds;

    // ─── Films, séries, animés (et vidéos locales) ───────────────────────────
    const wt = one('SELECT COALESCE(SUM(seconds), 0) AS seconds, COUNT(DISTINCT media_type || media_id) AS titles FROM watch_time WHERE day >= ? AND day <= ?', fromDay, toDay);
    const local = one(`
      SELECT COALESCE(SUM(${LISTENED}), 0) AS seconds, COUNT(DISTINCT t.id) AS titles
      FROM plays p JOIN tracks t ON t.id = p.track_id WHERE t.kind = 'video' AND p.played_at >= ? AND p.played_at < ?`, start, end);
    const finished = one('SELECT COUNT(*) AS n FROM watch_progress WHERE finished = 1 AND updated_at >= ? AND updated_at < ?', start, end);

    const topWatched = plain(q(`
      SELECT w.media_type AS type, w.media_id AS id, SUM(w.seconds) AS seconds,
             (SELECT title FROM watch_progress x WHERE x.media_type = w.media_type AND x.media_id = w.media_id ORDER BY updated_at DESC LIMIT 1) AS title,
             (SELECT poster FROM watch_progress x WHERE x.media_type = w.media_type AND x.media_id = w.media_id AND poster IS NOT NULL ORDER BY updated_at DESC LIMIT 1) AS poster,
             (SELECT genres FROM watch_progress x WHERE x.media_type = w.media_type AND x.media_id = w.media_id ORDER BY updated_at DESC LIMIT 1) AS genres
      FROM watch_time w WHERE w.day >= ? AND w.day <= ?
      GROUP BY w.media_type, w.media_id ORDER BY seconds DESC LIMIT 40`, fromDay, toDay));
    const topLocal = plain(q(`
      SELECT 'local' AS type, CAST(t.id AS TEXT) AS id, SUM(${LISTENED}) AS seconds, t.title AS title, NULL AS poster, '[]' AS genres
      FROM plays p JOIN tracks t ON t.id = p.track_id WHERE t.kind = 'video' AND p.played_at >= ? AND p.played_at < ?
      GROUP BY t.id ORDER BY seconds DESC LIMIT 10`, start, end));
    const topVideo = [...topWatched, ...topLocal].sort((a, b) => b.seconds - a.seconds).slice(0, 10);

    const videoGenres = new Map();
    for (const item of topWatched) {
      let list = [];
      try { list = JSON.parse(item.genres || '[]'); } catch { /* genres illisibles */ }
      for (const g of list) {
        const cur = videoGenres.get(String(g).toLowerCase()) || { name: g, seconds: 0, titles: 0 };
        cur.seconds += item.seconds;
        cur.titles += 1;
        videoGenres.set(String(g).toLowerCase(), cur);
      }
    }

    const videoByDay = plain(q(`
      SELECT day, SUM(seconds) AS seconds FROM watch_time WHERE day >= ? AND day <= ? GROUP BY day ORDER BY day`, fromDay, toDay));

    // ─── Séries et années disponibles (sur tout l'historique) ────────────────
    const allDays = [
      ...q(`SELECT DISTINCT strftime('%Y-%m-%d', p.played_at / 1000, ${LOCAL}) AS day FROM plays p`).map((r) => r.day),
      ...q('SELECT DISTINCT day FROM watch_time WHERE seconds >= 30').map((r) => r.day),
    ];
    const years = [...new Set(allDays.map((d) => Number(d.slice(0, 4))))].sort((a, b) => b - a);
    const months = [...new Set(allDays.map((d) => d.slice(0, 7)))].sort().reverse();
    const days = [...new Set(allDays)].sort().reverse();

    return {
      range: { from: fromDay, to: toDay },
      years,
      months,
      days,
      streak: streaks(allDays),
      music: {
        ...totals,
        topTracks,
        topArtists,
        topAlbums,
        genres: [...genres.values()].sort((a, b) => b.plays - a.plays || b.seconds - a.seconds).slice(0, 8),
        byDay,
        byHour,
        byWeekday,
      },
      video: {
        seconds: (wt.seconds || 0) + (local.seconds || 0),
        titles: (wt.titles || 0) + (local.titles || 0),
        finished: finished.n || 0,
        top: topVideo.map(({ genres: _g, ...rest }) => rest),
        genres: [...videoGenres.values()].sort((a, b) => b.seconds - a.seconds).slice(0, 6),
        byDay: videoByDay,
      },
    };
  }
}

module.exports = { Stats, streaks, boundsOf };

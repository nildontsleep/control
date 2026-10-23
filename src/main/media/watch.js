'use strict';

// Suivi du visionnage : progression par épisode, « Ma liste », historique et temps passé par jour.

const { transaction } = require('../db');

const FINISHED_RATIO = 0.92;
const MAX_STEP = 20; // un message de progression arrive toutes les 5 s : au-delà, c'est un saut dans la vidéo
const TYPES = new Set(['movie', 'tv', 'anime']);

const PROGRESS_COLUMNS = `
  media_type AS type, media_id AS id, season, episode, position, duration, finished, title, poster,
  backdrop, year, genres, first_at AS firstAt, updated_at AS updatedAt`;

const row = (r) => (r ? { ...r, season: r.season < 0 ? null : r.season, episode: r.episode < 0 ? null : r.episode, finished: !!r.finished, genres: safeJson(r.genres) } : r);

function safeJson(text) {
  try {
    return JSON.parse(text);
  } catch {
    return [];
  }
}

const localDay = (date = new Date()) => {
  const p = (n) => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${p(date.getMonth() + 1)}-${p(date.getDate())}`;
};

class Watch {
  constructor(db, emit) {
    this.db = db;
    this.emit = emit;
    this.last = new Map();
  }

  // ─── Progression ───────────────────────────────────────────────────────────
  save(rec) {
    if (!rec || !TYPES.has(rec.type) || !rec.id) return null;
    const season = Number.isInteger(rec.season) ? rec.season : -1;
    const episode = Number.isInteger(rec.episode) ? rec.episode : -1;
    const position = Math.max(0, Number(rec.position) || 0);
    const duration = Math.max(0, Number(rec.duration) || 0);
    const finished = rec.finished || (duration > 0 && position / duration >= FINISHED_RATIO) ? 1 : 0;
    const now = Date.now();
    const id = String(rec.id);

    this.db.prepare(`
      INSERT INTO watch_progress (media_type, media_id, season, episode, position, duration, finished, title, poster, backdrop, year, genres, first_at, updated_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(media_type, media_id, season, episode) DO UPDATE SET
        position = excluded.position,
        duration = CASE WHEN excluded.duration > 0 THEN excluded.duration ELSE watch_progress.duration END,
        finished = CASE WHEN excluded.finished = 1 THEN 1 WHEN excluded.position < 30 THEN watch_progress.finished ELSE excluded.finished END,
        title = CASE WHEN excluded.title <> '' THEN excluded.title ELSE watch_progress.title END,
        poster = COALESCE(excluded.poster, watch_progress.poster),
        backdrop = COALESCE(excluded.backdrop, watch_progress.backdrop),
        year = COALESCE(excluded.year, watch_progress.year),
        genres = CASE WHEN excluded.genres <> '[]' THEN excluded.genres ELSE watch_progress.genres END,
        updated_at = excluded.updated_at`)
      .run(rec.type, id, season, episode, position, duration, finished, String(rec.title || ''), rec.poster || null, rec.backdrop || null,
        Number.isInteger(rec.year) ? rec.year : null, JSON.stringify(Array.isArray(rec.genres) ? rec.genres.slice(0, 8) : []), now, now);

    // Temps réellement regardé : on n'ajoute que de petites avancées continues.
    const key = `${rec.type}:${id}:${season}:${episode}`;
    const before = this.last.get(key);
    this.last.set(key, { position, at: now });
    if (before && now - before.at < 60000) {
      const delta = position - before.position;
      if (delta > 0 && delta <= MAX_STEP) {
        this.db.prepare(`
          INSERT INTO watch_time (day, media_type, media_id, season, episode, seconds) VALUES (?, ?, ?, ?, ?, ?)
          ON CONFLICT(day, media_type, media_id, season, episode) DO UPDATE SET seconds = seconds + excluded.seconds`)
          .run(localDay(), rec.type, id, season, episode, delta);
      }
    }
    if (this.last.size > 50) this.last.delete(this.last.keys().next().value);
    return { finished: !!finished };
  }

  show(type, id) {
    return this.db.prepare(`SELECT ${PROGRESS_COLUMNS} FROM watch_progress WHERE media_type = ? AND media_id = ? ORDER BY updated_at DESC`)
      .all(type, String(id)).map(row);
  }

  /** « Continuer à regarder » : le dernier visionnage de chaque titre, s'il n'est pas terminé. */
  continueList(limit = 24) {
    return this.db.prepare(`
      SELECT ${PROGRESS_COLUMNS} FROM watch_progress wp
      WHERE finished = 0 AND position >= 10
        AND updated_at = (SELECT MAX(updated_at) FROM watch_progress WHERE media_type = wp.media_type AND media_id = wp.media_id)
      ORDER BY updated_at DESC LIMIT ?`).all(limit).map(row);
  }

  history(limit = 80) {
    return this.db.prepare(`SELECT ${PROGRESS_COLUMNS} FROM watch_progress WHERE position >= 10 ORDER BY updated_at DESC LIMIT ?`).all(limit).map(row);
  }

  removeShow(type, id) {
    this.db.prepare('DELETE FROM watch_progress WHERE media_type = ? AND media_id = ?').run(type, String(id));
    this.emit('watch:changed', { what: 'progress' });
  }

  clearHistory() {
    transaction(this.db, () => {
      this.db.exec('DELETE FROM watch_progress');
      this.db.exec('DELETE FROM watch_time');
    });
    this.last.clear();
    this.emit('watch:changed', { what: 'progress' });
  }

  // ─── Ma liste ──────────────────────────────────────────────────────────────
  list() {
    return this.db.prepare(`SELECT media_type AS type, media_id AS id, title, poster, backdrop, year, rating, added_at AS addedAt
      FROM watchlist ORDER BY added_at DESC`).all().map((r) => ({ ...r }));
  }

  toggle(item) {
    if (!item || !TYPES.has(item.type) || !item.id) return false;
    const id = String(item.id);
    const existing = this.db.prepare('SELECT 1 FROM watchlist WHERE media_type = ? AND media_id = ?').get(item.type, id);
    if (existing) {
      this.db.prepare('DELETE FROM watchlist WHERE media_type = ? AND media_id = ?').run(item.type, id);
    } else {
      this.db.prepare(`INSERT INTO watchlist (media_type, media_id, title, poster, backdrop, year, rating, added_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)`)
        .run(item.type, id, String(item.title || ''), item.poster || null, item.backdrop || null,
          Number.isInteger(item.year) ? item.year : null, Number.isFinite(item.rating) ? item.rating : null, Date.now());
    }
    this.emit('watch:changed', { what: 'list' });
    return !existing;
  }
}

module.exports = { Watch };

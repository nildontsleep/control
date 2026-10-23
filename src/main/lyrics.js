'use strict';

// Paroles : fichier .lrc voisin, paroles intégrées au fichier, puis LRCLIB (https://lrclib.net, gratuit, sans clé).
// Le résultat est gardé en base : un morceau n'est cherché en ligne qu'une fois.

const fs = require('node:fs');
const fsp = fs.promises;
const path = require('node:path');
const { net } = require('electron');

const API = process.env.CONTROL_LRCLIB_URL || 'https://lrclib.net/api';
const AGENT = 'control/0.3 (https://github.com/nildontsleep/control)';
const RETRY_NONE_AFTER = 3 * 24 * 3600 * 1000;
const STAMP = /\[\d{1,3}:\d{2}(?:[.:]\d{1,3})?\]/g;

/** Au moins trois repères de temps : c'est du LRC, pas du texte brut. */
const looksSynced = (text) => (String(text || '').match(STAMP) || []).length >= 3;

function classify(text) {
  const value = String(text || '').replace(/^﻿/, '').trim();
  if (!value) return { synced: null, plain: null };
  return looksSynced(value) ? { synced: value, plain: null } : { synced: null, plain: value };
}

class Lyrics {
  constructor({ db, library, tags }) {
    this.db = db;
    this.library = library;
    this.tags = tags;
  }

  #row(id) {
    return this.db.prepare('SELECT track_id AS id, synced, plain, source, instrumental, fetched_at AS fetchedAt FROM lyrics WHERE track_id = ?').get(id);
  }

  #store(id, { synced = null, plain = null, source, instrumental = 0 }) {
    this.db.prepare(`
      INSERT INTO lyrics (track_id, synced, plain, source, instrumental, fetched_at) VALUES (?, ?, ?, ?, ?, ?)
      ON CONFLICT(track_id) DO UPDATE SET synced = excluded.synced, plain = excluded.plain, source = excluded.source,
        instrumental = excluded.instrumental, fetched_at = excluded.fetched_at`)
      .run(id, synced, plain, source, instrumental ? 1 : 0, Date.now());
  }

  #result(row) {
    const found = !!(row.synced || row.plain || row.instrumental);
    return {
      status: found ? 'ok' : 'none',
      source: row.source,
      synced: row.synced || null,
      plain: row.plain || null,
      instrumental: !!row.instrumental,
    };
  }

  async #sidecar(file) {
    const base = file.slice(0, file.length - path.extname(file).length);
    for (const ext of ['.lrc', '.txt']) {
      try {
        const text = await fsp.readFile(base + ext, 'utf8');
        const hit = classify(text);
        if (hit.synced || hit.plain) return hit;
      } catch { /* pas de fichier voisin */ }
    }
    return null;
  }

  async #online({ title, artist, album, duration }) {
    // LRCLIB répond parfois « serveur occupé » (503) : on réessaie un peu avant d'abandonner.
    const get = async (url) => {
      let last;
      for (let attempt = 0; attempt < 3; attempt++) {
        try {
          const res = await net.fetch(url, { headers: { 'user-agent': AGENT }, signal: AbortSignal.timeout(10000) });
          if (res.status === 404) return null;
          if (res.status === 429 || res.status >= 500) last = Object.assign(new Error(`LRCLIB ${res.status}`), { code: 'busy' });
          else if (!res.ok) throw Object.assign(new Error(`LRCLIB ${res.status}`), { code: 'http' });
          else return await res.json();
        } catch (err) {
          if (err.code === 'http') throw err;
          last = err.code ? err : Object.assign(err, { code: 'network' });
        }
        await new Promise((resolve) => setTimeout(resolve, 900 * (attempt + 1)));
      }
      throw last;
    };
    const seconds = Math.round(duration || 0);
    if (artist && title) {
      const exact = new URLSearchParams({ track_name: title, artist_name: artist });
      if (album) exact.set('album_name', album);
      if (seconds) exact.set('duration', String(seconds));
      const hit = await get(`${API}/get?${exact}`);
      if (hit && (hit.syncedLyrics || hit.plainLyrics || hit.instrumental)) return hit;
    }
    const search = new URLSearchParams(artist ? { track_name: title, artist_name: artist } : { q: title });
    const list = await get(`${API}/search?${search}`);
    if (!Array.isArray(list) || !list.length) return null;
    // Même durée à quelques secondes près ; la version synchronisée est préférée.
    const close = list
      .filter((x) => !seconds || Math.abs((x.duration || 0) - seconds) <= 4)
      .sort((a, b) => (b.syncedLyrics ? 1 : 0) - (a.syncedLyrics ? 1 : 0) || Math.abs((a.duration || 0) - seconds) - Math.abs((b.duration || 0) - seconds));
    return close.find((x) => x.syncedLyrics || x.plainLyrics || x.instrumental) || null;
  }

  async get(id, { refresh = false } = {}) {
    const track = this.db.prepare('SELECT path, title, artist, album, duration FROM tracks WHERE id = ?').get(id);
    if (!track) throw new Error('Morceau introuvable.');

    // Un fichier .lrc posé à côté du morceau l'emporte toujours (c'est le choix explicite de l'utilisateur).
    const side = await this.#sidecar(track.path);
    if (side) {
      const row = { ...side, source: 'file', instrumental: 0 };
      this.#store(id, row);
      return this.#result(row);
    }

    const cached = this.#row(id);
    if (cached && !refresh && (cached.source !== 'none' || Date.now() - cached.fetchedAt < RETRY_NONE_AFTER)) {
      if (cached.source !== 'file') return this.#result(cached);
    }

    // Paroles intégrées au fichier (étiquette de tags).
    if (!refresh) {
      const embedded = classify(this.tags.lyricsOf(track.path));
      if (embedded.synced || embedded.plain) {
        const row = { ...embedded, source: 'tags', instrumental: 0 };
        this.#store(id, row);
        return this.#result(row);
      }
    }

    const hit = await this.#online(track);
    if (!hit) {
      const row = { source: 'none', synced: null, plain: null, instrumental: 0 };
      this.#store(id, row);
      return this.#result(row);
    }
    const row = {
      synced: hit.syncedLyrics || null,
      plain: hit.plainLyrics || null,
      source: 'lrclib',
      instrumental: hit.instrumental ? 1 : 0,
    };
    this.#store(id, row);
    return this.#result(row);
  }

  /** Paroles saisies ou collées à la main. */
  set(id, text) {
    const hit = classify(text);
    if (!hit.synced && !hit.plain) {
      this.db.prepare('DELETE FROM lyrics WHERE track_id = ?').run(id);
      return { status: 'none', source: 'none', synced: null, plain: null, instrumental: false };
    }
    const row = { ...hit, source: 'manual', instrumental: 0 };
    this.#store(id, row);
    return this.#result(row);
  }

  /** Écrit les paroles dans un fichier .lrc (ou .txt) à côté du morceau, pour que d'autres lecteurs les trouvent. */
  async saveSidecar(id) {
    const row = this.#row(id);
    const file = this.library.trackPath(id);
    if (!row || !file || !(row.synced || row.plain)) throw new Error('Aucune parole à enregistrer.');
    const base = file.slice(0, file.length - path.extname(file).length);
    const target = base + (row.synced ? '.lrc' : '.txt');
    await fsp.writeFile(target, row.synced || row.plain, 'utf8');
    // Le fichier devient la source de référence.
    this.#store(id, { synced: row.synced, plain: row.plain, source: 'file' });
    return target;
  }
}

module.exports = { Lyrics, classify, looksSynced };

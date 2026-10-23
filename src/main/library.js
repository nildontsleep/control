'use strict';

// Bibliothèque : scan incrémental des dossiers, lecture des tags, pochettes, playlists, écoutes.

const fs = require('node:fs');
const fsp = fs.promises;
const path = require('node:path');
const crypto = require('node:crypto');
const { spawn } = require('node:child_process');
const { nativeImage, shell } = require('electron');
const { transaction, TRACK_COLUMNS, plain } = require('./db');

const AUDIO_EXT = new Set(['.mp3', '.flac', '.m4a', '.m4b', '.aac', '.ogg', '.oga', '.opus', '.wav']);
const VIDEO_EXT = new Set(['.mp4', '.m4v', '.mov', '.mkv', '.webm']);
// Ces conteneurs peuvent ne contenir que du son : on regarde les pistes.
const AMBIGUOUS_EXT = new Set(['.mp4', '.m4v', '.mov', '.mkv', '.webm']);
const COVER_NAMES = ['cover', 'folder', 'front', 'album', 'albumart', 'albumartsmall'];
const COVER_EXT = ['.jpg', '.jpeg', '.png', '.webp'];
const READ_CONCURRENCY = 6;

const isMediaFile = (file) => {
  const ext = path.extname(file).toLowerCase();
  return AUDIO_EXT.has(ext) || VIDEO_EXT.has(ext);
};
const keyOf = (p) => (process.platform === 'win32' ? p.toLowerCase() : p);
const tick = () => new Promise((resolve) => setImmediate(resolve));

let mmPromise = null;
const musicMetadata = () => (mmPromise ??= import('music-metadata'));

async function* walk(dir) {
  let entries;
  try {
    entries = await fsp.readdir(dir, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    if (entry.name.startsWith('.') || entry.name.startsWith('$')) continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) yield* walk(full);
    else if (entry.isFile() && isMediaFile(entry.name)) yield full;
  }
}

class Library {
  constructor({ db, coversDir, thumbsDir, emit }) {
    this.db = db;
    this.coversDir = coversDir;
    this.thumbsDir = thumbsDir;
    this.emit = emit;
    this.scanning = null;
    this.rescan = null;
    this.folderCovers = new Map();
    this.changeTimer = null;
    fs.mkdirSync(coversDir, { recursive: true });
    fs.mkdirSync(thumbsDir, { recursive: true });
  }

  // ─── Lecture ───────────────────────────────────────────────────────────────
  tracks() {
    return plain(this.db.prepare(`SELECT ${TRACK_COLUMNS} FROM tracks`).all());
  }

  playlists() {
    const playlists = plain(this.db.prepare(
      'SELECT id, name, kind, rules, created_at AS createdAt, updated_at AS updatedAt FROM playlists ORDER BY name COLLATE NOCASE',
    ).all());
    const items = plain(this.db.prepare(
      'SELECT id, playlist_id AS playlistId, track_id AS trackId, position FROM playlist_tracks ORDER BY playlist_id, position',
    ).all());
    return { playlists, items };
  }

  trackPath(id) {
    return this.db.prepare('SELECT path FROM tracks WHERE id = ?').get(id)?.path || null;
  }

  trackInfo(id) {
    return this.db.prepare('SELECT path, kind, duration, mtime FROM tracks WHERE id = ?').get(id) || null;
  }

  /** Prévient l'interface : immédiatement, ou au plus une fois toutes les 400 ms pendant un scan. */
  changed(what = 'tracks', now = false) {
    if (now) {
      clearTimeout(this.changeTimer);
      this.changeTimer = null;
      this.emit('lib:changed', { what });
      return;
    }
    if (this.changeTimer) return;
    this.changeTimer = setTimeout(() => {
      this.changeTimer = null;
      this.emit('lib:changed', { what });
    }, 400);
  }

  // ─── Scan ──────────────────────────────────────────────────────────────────
  /** Exécute fn pendant qu'aucun scan ne tourne (renommage de fichiers, écriture de tags). */
  async exclusive(fn) {
    while (this.scanning) await this.scanning;
    this.holds = (this.holds || 0) + 1;
    try {
      return await fn();
    } finally {
      this.holds -= 1;
      if (!this.holds && this.deferred) {
        const roots = this.deferred;
        this.deferred = null;
        this.scan(roots);
      }
    }
  }

  scan(roots) {
    roots = [...new Set((roots || []).filter(Boolean))];
    if (this.holds) {
      this.deferred = [...new Set([...(this.deferred || []), ...roots])];
      return Promise.resolve({ added: 0, removed: 0, total: 0 });
    }
    if (this.scanning) {
      this.rescan = [...new Set([...(this.rescan || []), ...roots])];
      return this.scanning;
    }
    this.scanning = this.#scan(roots)
      .catch((err) => {
        console.error('Scan interrompu :', err);
        return { added: 0, removed: 0, total: 0 };
      })
      .then(async (result) => {
        this.scanning = null;
        if (this.rescan) {
          const next = this.rescan;
          this.rescan = null;
          await this.scan(next);
        }
        return result;
      });
    return this.scanning;
  }

  async #scan(roots) {
    const progress = (extra) => this.emit('lib:scan-progress', extra);
    progress({ phase: 'discover', found: 0, done: 0, total: 0 });

    const found = new Map();
    const perRoot = new Map();
    const reachable = [];
    for (const root of roots) {
      try {
        await fsp.access(root);
      } catch {
        continue; // disque débranché : on ne touche à rien
      }
      reachable.push(root);
      let n = 0;
      for await (const file of walk(root)) {
        try {
          const st = await fsp.stat(file);
          found.set(keyOf(file), { path: file, size: st.size, mtime: Math.floor(st.mtimeMs) });
          n++;
        } catch { /* fichier disparu */ }
        if (found.size % 250 === 0) {
          progress({ phase: 'discover', found: found.size, done: 0, total: 0 });
          await tick();
        }
      }
      perRoot.set(root, n);
    }

    const existing = new Map();
    for (const row of this.db.prepare('SELECT id, path, size, mtime FROM tracks').all()) existing.set(keyOf(row.path), row);

    const todo = [];
    for (const [key, file] of found) {
      const row = existing.get(key);
      if (!row || row.size !== file.size || row.mtime !== file.mtime) todo.push(file);
    }

    // Fichiers supprimés : seulement dans les dossiers lisibles et non vides.
    const removeIds = [];
    for (const root of reachable) {
      const prefix = keyOf(path.resolve(root)) + path.sep;
      const rows = [...existing.entries()].filter(([key]) => key.startsWith(prefix));
      if (!perRoot.get(root) && rows.length) continue;
      for (const [key, row] of rows) if (!found.has(key)) removeIds.push(row.id);
    }
    if (removeIds.length) this.#deleteRows(removeIds);

    let done = 0;
    let added = 0;
    let index = 0;
    let batch = [];
    const flush = () => {
      if (!batch.length) return;
      const rows = batch;
      batch = [];
      transaction(this.db, () => rows.forEach((row) => this.#upsert(row)));
      this.changed('tracks');
    };
    const worker = async () => {
      while (index < todo.length) {
        const file = todo[index++];
        const row = await this.#read(file);
        if (row) {
          batch.push(row);
          added++;
        }
        done++;
        if (batch.length >= 40) flush();
        if (done % 20 === 0) {
          progress({ phase: 'read', done, total: todo.length, found: found.size });
          await tick();
        }
      }
    };
    await Promise.all(Array.from({ length: Math.min(READ_CONCURRENCY, todo.length) }, worker));
    flush();
    this.folderCovers.clear();

    const result = { added, removed: removeIds.length, total: found.size };
    progress({ phase: 'done', ...result });
    if (added || removeIds.length) this.changed('tracks', true);
    return result;
  }

  /** Ajoute des fichiers ou dossiers précis (glisser-déposer, téléchargements). */
  async addPaths(paths, { sourceUrl = null } = {}) {
    const files = new Map();
    for (const p of paths) {
      try {
        const st = await fsp.stat(p);
        if (st.isDirectory()) {
          for await (const file of walk(p)) {
            const fst = await fsp.stat(file);
            files.set(keyOf(file), { path: file, size: fst.size, mtime: Math.floor(fst.mtimeMs) });
          }
        } else if (st.isFile() && isMediaFile(p)) {
          files.set(keyOf(p), { path: p, size: st.size, mtime: Math.floor(st.mtimeMs) });
        }
      } catch { /* chemin invalide */ }
    }
    const ids = [];
    const rows = [];
    for (const file of files.values()) {
      const row = await this.#read(file, sourceUrl);
      if (row) rows.push(row);
    }
    transaction(this.db, () => rows.forEach((row) => ids.push(this.#upsert(row))));
    this.folderCovers.clear();
    if (ids.length) this.changed('tracks', true);
    return ids;
  }

  async #read(file, sourceUrl = null) {
    const ext = path.extname(file.path).toLowerCase();
    const row = {
      path: file.path,
      kind: VIDEO_EXT.has(ext) && !AUDIO_EXT.has(ext) ? 'video' : 'audio',
      title: path.basename(file.path, ext),
      artist: '',
      album: '',
      albumArtist: '',
      genre: '',
      year: null,
      trackNo: null,
      discNo: null,
      duration: 0,
      bitrate: null,
      codec: null,
      size: file.size,
      mtime: file.mtime,
      cover: null,
      width: null,
      height: null,
      sourceUrl,
    };
    try {
      const { parseFile, selectCover } = await musicMetadata();
      // Ogg ne stocke pas sa durée : il faut lire la fin du fichier pour la connaître.
      const isOgg = ext === '.ogg' || ext === '.oga' || ext === '.opus';
      const meta = await parseFile(file.path, { duration: isOgg, skipPostHeaders: !isOgg });
      const c = meta.common;
      const f = meta.format;
      if (c.title) row.title = c.title.trim();
      row.artist = (c.artist || (c.artists || []).join(', ') || '').trim();
      row.album = (c.album || '').trim();
      row.albumArtist = (c.albumartist || '').trim();
      row.genre = (c.genre || []).join(', ');
      row.year = c.year || null;
      row.trackNo = c.track?.no || null;
      row.discNo = c.disk?.no || null;
      row.duration = f.duration || 0;
      row.bitrate = f.bitrate ? Math.round(f.bitrate / 1000) : null;
      row.codec = f.codec || f.container || null;
      const video = (f.trackInfo || []).find((t) => t.video);
      if (AMBIGUOUS_EXT.has(ext)) row.kind = video ? 'video' : 'audio';
      if (video?.video) {
        row.width = video.video.pixelWidth || null;
        row.height = video.video.pixelHeight || null;
      }
      const picture = selectCover(c.picture);
      if (picture) row.cover = this.#saveCover(picture.data);
    } catch { /* tags illisibles : on garde le nom de fichier */ }

    if (!row.cover && row.kind === 'audio') row.cover = await this.#folderCover(path.dirname(file.path));
    if (!row.sourceUrl) delete row.sourceUrl;
    return row;
  }

  /** Relit un fichier modifié (tags, nom) en gardant le même identifiant : lectures et playlists suivent. */
  async refreshTrack(id, newPath = null) {
    const current = this.trackPath(id);
    if (!current) return null;
    const file = newPath || current;
    const st = await fsp.stat(file);
    const row = await this.#read({ path: file, size: st.size, mtime: Math.floor(st.mtimeMs) });
    transaction(this.db, () => {
      if (file !== current) {
        this.db.prepare('UPDATE tracks SET path = ? WHERE id = ?').run(file, id);
      }
      this.#upsert(row);
    });
    this.folderCovers.clear();
    return this.db.prepare(`SELECT ${TRACK_COLUMNS} FROM tracks WHERE id = ?`).get(id);
  }

  #upsert(row) {
    const stmt = this.db.prepare(`
      INSERT INTO tracks (path, kind, title, artist, album, album_artist, genre, year, track_no, disc_no,
        duration, bitrate, codec, size, mtime, cover, width, height, added_at, source_url)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(path) DO UPDATE SET
        kind = excluded.kind, title = excluded.title, artist = excluded.artist, album = excluded.album,
        album_artist = excluded.album_artist, genre = excluded.genre, year = excluded.year,
        track_no = excluded.track_no, disc_no = excluded.disc_no, duration = excluded.duration,
        bitrate = excluded.bitrate, codec = excluded.codec, size = excluded.size, mtime = excluded.mtime,
        cover = excluded.cover, width = excluded.width, height = excluded.height,
        source_url = COALESCE(excluded.source_url, tracks.source_url)
      RETURNING id`);
    return stmt.get(
      row.path, row.kind, row.title, row.artist, row.album, row.albumArtist, row.genre, row.year,
      row.trackNo, row.discNo, row.duration, row.bitrate, row.codec, row.size, row.mtime, row.cover,
      row.width, row.height, Date.now(), row.sourceUrl ?? null,
    ).id;
  }

  #deleteRows(ids) {
    transaction(this.db, () => {
      const stmt = this.db.prepare('DELETE FROM tracks WHERE id = ?');
      ids.forEach((id) => stmt.run(id));
    });
  }

  async removeTracks(ids, { deleteFiles = false } = {}) {
    if (deleteFiles) {
      for (const id of ids) {
        const file = this.trackPath(id);
        if (!file) continue;
        try {
          await shell.trashItem(file);
        } catch { /* déjà supprimé ou verrouillé */ }
      }
    }
    this.#deleteRows(ids);
    this.changed('tracks', true);
    this.changed('playlists', true);
  }

  // ─── Pochettes ─────────────────────────────────────────────────────────────
  #saveCover(data) {
    const buffer = Buffer.from(data);
    const hash = crypto.createHash('sha1').update(buffer).digest('hex').slice(0, 20);
    const big = path.join(this.coversDir, `${hash}.jpg`);
    if (fs.existsSync(big)) return hash;
    const image = nativeImage.createFromBuffer(buffer);
    if (image.isEmpty()) {
      // Format que nativeImage ne décode pas (WebP…) : le navigateur saura l'afficher.
      fs.writeFileSync(big, buffer);
      fs.copyFileSync(big, path.join(this.coversDir, `${hash}-s.jpg`));
      return hash;
    }
    const { width, height } = image.getSize();
    const fit = (max) => {
      const k = Math.min(1, max / Math.max(width, height));
      return k < 1 ? image.resize({ width: Math.max(1, Math.round(width * k)), height: Math.max(1, Math.round(height * k)), quality: 'best' }) : image;
    };
    fs.writeFileSync(big, fit(640).toJPEG(88));
    fs.writeFileSync(path.join(this.coversDir, `${hash}-s.jpg`), fit(160).toJPEG(82));
    return hash;
  }

  async #folderCover(dir) {
    if (this.folderCovers.has(dir)) return this.folderCovers.get(dir);
    let hash = null;
    try {
      const names = await fsp.readdir(dir);
      const wanted = COVER_NAMES.flatMap((n) => COVER_EXT.map((e) => n + e));
      const hit = names.find((name) => wanted.includes(name.toLowerCase()));
      if (hit) hash = this.#saveCover(await fsp.readFile(path.join(dir, hit)));
    } catch { /* dossier illisible */ }
    this.folderCovers.set(dir, hash);
    return hash;
  }

  /** Vignette d’une vidéo : miniature système (Windows/macOS) sinon image extraite par ffmpeg. */
  async videoThumb(id, ffmpegPath) {
    const info = this.trackInfo(id);
    if (!info || info.kind !== 'video') return null;
    const out = path.join(this.thumbsDir, `${id}-${info.mtime}.jpg`);
    if (fs.existsSync(out)) return out;
    try {
      const image = await nativeImage.createThumbnailFromPath(info.path, { width: 480, height: 480 });
      if (!image.isEmpty()) {
        fs.writeFileSync(out, image.toJPEG(84));
        return out;
      }
    } catch { /* non supporté sur cette plateforme */ }
    if (!ffmpegPath) return null;
    const at = info.duration > 6 ? 3 : 0;
    const ok = await new Promise((resolve) => {
      const child = spawn(ffmpegPath, ['-y', '-v', 'error', '-ss', String(at), '-i', info.path, '-frames:v', '1', '-vf', 'scale=480:-2', '-q:v', '4', out], { windowsHide: true });
      child.on('error', () => resolve(false));
      child.on('close', (code) => resolve(code === 0));
    });
    return ok && fs.existsSync(out) ? out : null;
  }

  // ─── Playlists ─────────────────────────────────────────────────────────────
  createPlaylist(name, kind = 'manual', rules = null) {
    const now = Date.now();
    const id = this.db.prepare('INSERT INTO playlists (name, kind, rules, created_at, updated_at) VALUES (?, ?, ?, ?, ?) RETURNING id')
      .get(name.trim() || 'Sans titre', kind, rules ? JSON.stringify(rules) : null, now, now).id;
    this.changed('playlists', true);
    return id;
  }

  renamePlaylist(id, name) {
    this.db.prepare('UPDATE playlists SET name = ?, updated_at = ? WHERE id = ?').run(name.trim() || 'Sans titre', Date.now(), id);
    this.changed('playlists', true);
  }

  setPlaylistRules(id, rules) {
    this.db.prepare('UPDATE playlists SET rules = ?, updated_at = ? WHERE id = ?').run(JSON.stringify(rules), Date.now(), id);
    this.changed('playlists', true);
  }

  deletePlaylist(id) {
    this.db.prepare('DELETE FROM playlists WHERE id = ?').run(id);
    this.changed('playlists', true);
  }

  addToPlaylist(playlistId, trackIds) {
    const now = Date.now();
    transaction(this.db, () => {
      let { next } = this.db.prepare('SELECT COALESCE(MAX(position), -1) + 1 AS next FROM playlist_tracks WHERE playlist_id = ?').get(playlistId);
      const stmt = this.db.prepare('INSERT INTO playlist_tracks (playlist_id, track_id, position, added_at) VALUES (?, ?, ?, ?)');
      for (const trackId of trackIds) stmt.run(playlistId, trackId, next++, now);
      this.db.prepare('UPDATE playlists SET updated_at = ? WHERE id = ?').run(now, playlistId);
    });
    this.changed('playlists', true);
  }

  removeFromPlaylist(entryIds) {
    transaction(this.db, () => {
      const stmt = this.db.prepare('DELETE FROM playlist_tracks WHERE id = ?');
      entryIds.forEach((id) => stmt.run(id));
    });
    this.changed('playlists', true);
  }

  reorderPlaylist(playlistId, entryIds) {
    transaction(this.db, () => {
      const stmt = this.db.prepare('UPDATE playlist_tracks SET position = ? WHERE id = ? AND playlist_id = ?');
      entryIds.forEach((id, position) => stmt.run(position, id, playlistId));
      this.db.prepare('UPDATE playlists SET updated_at = ? WHERE id = ?').run(Date.now(), playlistId);
    });
    this.changed('playlists', true);
  }

  async importM3U(file) {
    const text = await fsp.readFile(file, 'utf8');
    const dir = path.dirname(file);
    const byPath = new Map(this.db.prepare('SELECT id, path FROM tracks').all().map((r) => [keyOf(r.path), r.id]));
    const lines = text.split(/\r?\n/).map((l) => l.trim()).filter((l) => l && !l.startsWith('#'));
    const ids = [];
    for (const line of lines) {
      const full = path.isAbsolute(line) ? line : path.resolve(dir, line);
      const id = byPath.get(keyOf(path.normalize(full)));
      if (id) ids.push(id);
    }
    const name = path.basename(file, path.extname(file));
    const playlistId = this.createPlaylist(name);
    if (ids.length) this.addToPlaylist(playlistId, ids);
    return { id: playlistId, matched: ids.length, total: lines.length };
  }

  exportM3U(file, trackIds) {
    const rows = trackIds.map((id) => this.db.prepare('SELECT path, title, artist, duration FROM tracks WHERE id = ?').get(id)).filter(Boolean);
    const lines = ['#EXTM3U'];
    for (const r of rows) {
      lines.push(`#EXTINF:${Math.round(r.duration)},${r.artist ? `${r.artist} - ` : ''}${r.title}`, r.path);
    }
    fs.writeFileSync(file, `${lines.join('\r\n')}\r\n`, 'utf8');
    return rows.length;
  }

  // ─── Écoutes ───────────────────────────────────────────────────────────────
  playStart(trackId, duration) {
    const now = Date.now();
    const id = this.db.prepare('INSERT INTO plays (track_id, played_at, duration) VALUES (?, ?, ?) RETURNING id').get(trackId, now, duration || 0).id;
    this.db.prepare('UPDATE tracks SET play_count = play_count + 1, last_played = ? WHERE id = ?').run(now, trackId);
    return id;
  }

  playEnd(playId, seconds, completed) {
    this.db.prepare('UPDATE plays SET seconds = ?, completed = ? WHERE id = ?').run(seconds, completed ? 1 : 0, playId);
  }
}

module.exports = { Library, isMediaFile, AUDIO_EXT, VIDEO_EXT };

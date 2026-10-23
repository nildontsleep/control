'use strict';

// Éditeur de tags : lecture et écriture dans les fichiers (MP3, FLAC, M4A, OGG, Opus, WAV…),
// pochettes (fichier, adresse, recherche en ligne), renommage et annulation du dernier lot.

const fs = require('node:fs');
const fsp = fs.promises;
const path = require('node:path');
const crypto = require('node:crypto');
const { nativeImage, net } = require('electron');
const tl = require('node-taglib-sharp');

// Les anciens lecteurs (Explorateur Windows, autoradios) lisent mieux l'ID3v2.3 que la 2.4.
tl.Id3v2Settings.defaultVersion = 3;
tl.Id3v2Settings.forceDefaultVersion = false;
tl.Id3v2Settings.defaultEncoding = tl.StringType.UTF16;

const ITUNES = process.env.CONTROL_ITUNES_URL || 'https://itunes.apple.com/search';
const DEEZER = process.env.CONTROL_DEEZER_URL || 'https://api.deezer.com/search/album';

const MAX_IMAGE = 25 * 1024 * 1024;
const COVER_PX = 1400;
const UNDO_BYTES = 40 * 1024 * 1024;

// Conteneurs que TagLib sait modifier (Matroska et WebM ne le sont pas).
const WRITABLE_EXT = new Set(['.mp3', '.flac', '.m4a', '.m4b', '.mp4', '.m4v', '.aac', '.ogg', '.oga', '.opus', '.wav']);
const isWritable = (file) => WRITABLE_EXT.has(path.extname(file).toLowerCase());
const UNSUPPORTED = { code: 'unsupported', message: 'Format non modifiable (MKV, WebM…).' };

const TEXT_FIELDS = ['title', 'artist', 'albumArtist', 'album', 'genre', 'composer', 'comment', 'lyrics'];
const NUMBER_FIELDS = ['year', 'trackNo', 'trackTotal', 'discNo', 'discTotal'];
const FIELDS = [...TEXT_FIELDS, ...NUMBER_FIELDS];

const join = (list) => (list || []).filter(Boolean).join('; ');

function readFields(tag) {
  return {
    title: tag.title || '',
    artist: join(tag.performers),
    albumArtist: join(tag.albumArtists),
    album: tag.album || '',
    genre: join(tag.genres),
    composer: join(tag.composers),
    comment: tag.comment || '',
    lyrics: tag.lyrics || '',
    year: tag.year || 0,
    trackNo: tag.track || 0,
    trackTotal: tag.trackCount || 0,
    discNo: tag.disc || 0,
    discTotal: tag.discCount || 0,
  };
}

function applyFields(tag, set) {
  const text = (name) => (typeof set[name] === 'string' ? set[name].trim() : null);
  const list = (name) => { const v = text(name); return v === null ? null : (v ? [v] : []); };
  const number = (name, max) => {
    if (set[name] === undefined || set[name] === null || set[name] === '') return set[name] === undefined ? null : 0;
    const n = Math.trunc(Number(set[name]));
    return Number.isFinite(n) && n >= 0 ? Math.min(n, max) : null;
  };
  let v;
  if ((v = text('title')) !== null) tag.title = v || undefined;
  if ((v = list('artist')) !== null) tag.performers = v;
  if ((v = list('albumArtist')) !== null) tag.albumArtists = v;
  if ((v = text('album')) !== null) tag.album = v || undefined;
  if ((v = list('genre')) !== null) tag.genres = v;
  if ((v = list('composer')) !== null) tag.composers = v;
  if ((v = text('comment')) !== null) tag.comment = v || undefined;
  if (typeof set.lyrics === 'string') tag.lyrics = set.lyrics.trim() || undefined;
  if ((v = number('year', 9999)) !== null) tag.year = v;
  if ((v = number('trackNo', 9999)) !== null) tag.track = v;
  if ((v = number('trackTotal', 9999)) !== null) tag.trackCount = v;
  if ((v = number('discNo', 999)) !== null) tag.disc = v;
  if ((v = number('discTotal', 999)) !== null) tag.discCount = v;
}

const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Windows verrouille un fichier en cours de lecture : on réessaie un peu avant d'abandonner. */
async function retrying(fn) {
  let last;
  for (let i = 0; i < 4; i++) {
    try {
      return await fn();
    } catch (err) {
      last = err;
      if (!['EBUSY', 'EPERM', 'EACCES'].includes(err.code)) break;
      await wait(250 * (i + 1));
    }
  }
  throw last;
}

function failure(err) {
  const code = err?.code;
  if (code === 'UNSUPPORTED') return UNSUPPORTED;
  if (code === 'ENOENT') return { code: 'missing', message: 'Fichier introuvable.' };
  if (code === 'EBUSY') return { code: 'locked', message: 'Fichier utilisé par un autre programme.' };
  if (code === 'EPERM' || code === 'EACCES' || code === 'EROFS') return { code: 'readonly', message: 'Fichier en lecture seule ou verrouillé.' };
  if (/unsupported|not supported|supported/i.test(String(err?.name) + String(err?.message))) return { code: 'unsupported', message: 'Format non pris en charge.' };
  return { code: 'other', message: String(err?.message || err) };
}

/** Nom de fichier valable sur Windows, macOS et Linux. */
function safeName(name) {
  const cleaned = String(name || '')
    .replace(/[\u0000-\u001f<>:"/\\|?*]/g, '_')
    .replace(/\s+/g, ' ')
    .replace(/^[\s.]+|[\s.]+$/g, '')
    .slice(0, 180)
    .trim();
  if (!cleaned || /^(con|prn|aux|nul|com\d|lpt\d)$/i.test(cleaned)) return '';
  return cleaned;
}

async function imageFrom(buffer) {
  if (!buffer?.length || buffer.length > MAX_IMAGE) throw new Error('Image trop volumineuse ou vide.');
  const isJpeg = buffer[0] === 0xff && buffer[1] === 0xd8;
  const image = nativeImage.createFromBuffer(buffer);
  if (image.isEmpty()) throw new Error('Format d’image non pris en charge.');
  const { width, height } = image.getSize();
  let data = buffer;
  let mime = isJpeg ? 'image/jpeg' : 'image/png';
  if (!isJpeg || Math.max(width, height) > COVER_PX || buffer.length > 3 * 1024 * 1024) {
    const k = Math.min(1, COVER_PX / Math.max(width, height));
    const fitted = k < 1 ? image.resize({ width: Math.max(1, Math.round(width * k)), height: Math.max(1, Math.round(height * k)), quality: 'best' }) : image;
    data = fitted.toJPEG(90);
    mime = 'image/jpeg';
  }
  const preview = nativeImage.createFromBuffer(data);
  const { width: pw, height: ph } = preview.getSize();
  const small = Math.max(pw, ph) > 480
    ? preview.resize({ width: Math.round(pw * 480 / Math.max(pw, ph)), height: Math.round(ph * 480 / Math.max(pw, ph)), quality: 'good' })
    : preview;
  return { data, mime, width, height, preview: `data:image/jpeg;base64,${small.toJPEG(82).toString('base64')}` };
}

class Tags {
  constructor({ library }) {
    this.library = library;
    this.staged = new Map();
    this.lastBatch = null;
  }

  // ─── Lecture ───────────────────────────────────────────────────────────────
  async read(ids) {
    const out = [];
    for (const id of ids) {
      const info = this.library.trackInfo(id);
      if (!info) {
        out.push({ id, error: { code: 'missing', message: 'Morceau introuvable.' } });
        continue;
      }
      if (!isWritable(info.path)) {
        out.push({ id, path: info.path, error: UNSUPPORTED });
        continue;
      }
      let file;
      try {
        file = tl.File.createFromPath(info.path);
        out.push({ id, path: info.path, fields: readFields(file.tag), embedded: file.tag.pictures.length });
      } catch (err) {
        out.push({ id, path: info.path, error: failure(err) });
      } finally {
        file?.dispose();
      }
    }
    return out;
  }

  /** Paroles intégrées au fichier (étiquette USLT, LYRICS…), ou chaîne vide. */
  lyricsOf(file) {
    if (!isWritable(file)) return '';
    let f;
    try {
      f = tl.File.createFromPath(file);
      return f.tag.lyrics || '';
    } catch {
      return '';
    } finally {
      f?.dispose();
    }
  }

  // ─── Pochettes ─────────────────────────────────────────────────────────────
  #keep(token, entry) {
    this.staged.set(token, { ...entry, at: Date.now() });
    if (this.staged.size > 24) this.staged.delete(this.staged.keys().next().value);
  }

  /** Prépare une pochette (fichier ou adresse) : redimensionnée, aperçu prêt à afficher. */
  async stageCover({ path: file, url }) {
    let buffer;
    if (file) {
      const stat = await fsp.stat(file);
      if (!stat.isFile() || stat.size > MAX_IMAGE) throw new Error('Image trop volumineuse.');
      buffer = await fsp.readFile(file);
    } else if (typeof url === 'string' && /^https:\/\//i.test(url)) {
      const res = await net.fetch(url, { headers: { 'user-agent': 'control (https://github.com/nildontsleep/control)' } });
      if (!res.ok) throw new Error(`Téléchargement impossible (${res.status}).`);
      buffer = Buffer.from(await res.arrayBuffer());
    } else {
      throw new Error('Source de pochette invalide.');
    }
    const image = await imageFrom(buffer);
    const token = crypto.randomBytes(8).toString('hex');
    this.#keep(token, { data: image.data, mime: image.mime });
    return { token, preview: image.preview, width: image.width, height: image.height };
  }

  async searchCovers({ artist = '', album = '' }) {
    const query = `${artist} ${album}`.replace(/\s+/g, ' ').trim();
    if (!query) return [];
    const headers = { 'user-agent': 'control (https://github.com/nildontsleep/control)', accept: 'application/json' };
    const get = async (url) => {
      const res = await net.fetch(url, { headers, signal: AbortSignal.timeout(9000) });
      if (!res.ok) throw new Error(String(res.status));
      return res.json();
    };
    const [itunes, deezer] = await Promise.allSettled([
      get(`${ITUNES}?${new URLSearchParams({ media: 'music', entity: 'album', limit: '12', term: query })}`),
      get(`${DEEZER}?${new URLSearchParams({ q: query, limit: '12' })}`),
    ]);
    const a = itunes.status === 'fulfilled' ? (itunes.value.results || []).filter((r) => r.artworkUrl100).map((r) => ({
      source: 'Apple Music',
      title: r.collectionName,
      artist: r.artistName,
      year: Number(String(r.releaseDate || '').slice(0, 4)) || null,
      genre: r.primaryGenreName || '',
      tracks: r.trackCount || null,
      preview: r.artworkUrl100.replace(/\d+x\d+bb/, '300x300bb'),
      full: r.artworkUrl100.replace(/\d+x\d+bb/, '1200x1200bb'),
    })) : [];
    const d = deezer.status === 'fulfilled' ? (deezer.value.data || []).filter((r) => r.cover_xl || r.cover_big).map((r) => ({
      source: 'Deezer',
      title: r.title,
      artist: r.artist?.name || '',
      year: null,
      genre: '',
      tracks: r.nb_tracks || null,
      preview: r.cover_medium || r.cover_big,
      full: r.cover_xl || r.cover_big,
    })) : [];
    if (itunes.status === 'rejected' && deezer.status === 'rejected') throw new Error('network');
    // Les deux services en alternance, sans doublon évident.
    const seen = new Set();
    const merged = [];
    for (let i = 0; i < Math.max(a.length, d.length); i++) {
      for (const item of [a[i], d[i]]) {
        if (!item) continue;
        const key = `${item.title}|${item.artist}`.toLowerCase();
        if (seen.has(key)) continue;
        seen.add(key);
        merged.push(item);
      }
    }
    return merged.slice(0, 16);
  }

  // ─── Écriture ──────────────────────────────────────────────────────────────
  /**
   * edits : [{ id, set: { champ: valeur }, cover: { token } | { remove: true }, rename: 'nom sans extension' }]
   * Retourne un résultat par morceau ; les erreurs n'interrompent pas le lot.
   */
  async write(edits) {
    return this.library.exclusive(async () => {
      const results = [];
      const snapshots = [];
      let bytes = 0;
      for (const edit of edits) {
        const info = this.library.trackInfo(edit.id);
        if (!info) {
          results.push({ id: edit.id, ok: false, ...failure({ code: 'ENOENT' }) });
          continue;
        }
        const snapshot = { id: edit.id, path: info.path, fields: null, pictures: null };
        try {
          if (!isWritable(info.path)) throw Object.assign(new Error('unsupported'), { code: 'UNSUPPORTED' });
          const wantsTags = edit.set && Object.keys(edit.set).some((k) => FIELDS.includes(k));
          const wantsCover = !!edit.cover;
          if (wantsTags || wantsCover) {
            await retrying(async () => {
              const file = tl.File.createFromPath(info.path);
              try {
                snapshot.fields = readFields(file.tag);
                if (wantsCover) {
                  snapshot.pictures = file.tag.pictures.map((p) => ({ data: Buffer.from(p.data.toByteArray()), mime: p.mimeType, type: p.type }));
                }
                if (wantsTags) applyFields(file.tag, edit.set);
                if (edit.cover?.remove) {
                  file.tag.pictures = [];
                } else if (edit.cover?.token) {
                  const staged = this.staged.get(edit.cover.token);
                  if (!staged) throw new Error('Pochette expirée : choisissez-la de nouveau.');
                  const pic = tl.Picture.fromData(tl.ByteVector.fromByteArray(staged.data));
                  pic.type = tl.PictureType.FrontCover;
                  pic.mimeType = staged.mime;
                  file.tag.pictures = [pic];
                }
                file.save();
              } finally {
                file.dispose();
              }
            });
          }
          let newPath = null;
          if (edit.rename) newPath = await this.#rename(info.path, edit.rename);
          const row = await this.library.refreshTrack(edit.id, newPath);
          results.push({ id: edit.id, ok: true, path: row?.path || info.path, renamed: !!newPath && newPath !== info.path });
          if (newPath && newPath !== info.path) snapshot.renamedTo = newPath;
          for (const p of snapshot.pictures || []) bytes += p.data.length;
          snapshots.push(snapshot);
        } catch (err) {
          results.push({ id: edit.id, ok: false, ...failure(err) });
        }
      }
      if (snapshots.length) {
        this.lastBatch = bytes > UNDO_BYTES ? snapshots.map((s) => ({ ...s, pictures: null })) : snapshots;
        this.library.changed('tracks', true);
      }
      return { results, undoable: snapshots.length > 0 };
    });
  }

  async #rename(file, base) {
    const dir = path.dirname(file);
    const ext = path.extname(file);
    const name = safeName(base);
    if (!name) throw new Error('Nom de fichier invalide.');
    let target = path.join(dir, name + ext);
    if (target === file) return file;
    const same = target.toLowerCase() === file.toLowerCase(); // simple changement de casse
    if (!same) {
      for (let n = 2; fs.existsSync(target); n++) target = path.join(dir, `${name} (${n})${ext}`);
    }
    await retrying(() => fsp.rename(file, target));
    return target;
  }

  /** Remet les fichiers du dernier lot dans leur état d'avant (tags, pochette, nom). */
  async undo() {
    const batch = this.lastBatch;
    if (!batch?.length) return { restored: 0, failed: 0 };
    this.lastBatch = null;
    return this.library.exclusive(async () => {
      let restored = 0;
      let failed = 0;
      for (const s of [...batch].reverse()) {
        try {
          let file = s.renamedTo || s.path;
          if (s.renamedTo) {
            await retrying(() => fsp.rename(s.renamedTo, s.path));
            file = s.path;
          }
          if (s.fields) {
            await retrying(async () => {
              const f = tl.File.createFromPath(file);
              try {
                const clear = {};
                for (const name of TEXT_FIELDS) clear[name] = s.fields[name];
                for (const name of NUMBER_FIELDS) clear[name] = s.fields[name];
                applyFields(f.tag, clear);
                if (s.pictures) {
                  f.tag.pictures = s.pictures.map((p) => {
                    const pic = tl.Picture.fromData(tl.ByteVector.fromByteArray(p.data));
                    pic.type = p.type;
                    pic.mimeType = p.mime;
                    return pic;
                  });
                }
                f.save();
              } finally {
                f.dispose();
              }
            });
          }
          await this.library.refreshTrack(s.id, file !== (this.library.trackPath(s.id)) ? file : null);
          restored++;
        } catch {
          failed++;
        }
      }
      this.library.changed('tracks', true);
      return { restored, failed };
    });
  }
}

module.exports = { Tags, FIELDS, safeName };

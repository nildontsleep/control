'use strict';

// API compatible Subsonic / OpenSubsonic (/rest/…) : les applications du commerce (Symfonium, Substreamer, DSub,
// Feishin, Sonixd…) se connectent au serveur et parcourent, écoutent et notent la bibliothèque du compte.
// Authentification propre à cette API : mot de passe d'application du compte (u + p, ou u + t + s avec t = md5(secret + s))
// ou clé d'API (apiKey), jamais le cookie de session ni le mot de passe du compte. Ni transcodage, ni podcasts, ni radios,
// ni partages : ce que le serveur ne fait pas est annoncé comme vide ou refusé avec le code d'erreur prévu.

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { readText, send, sendFile, mimeOf } = require('./http');
const { Secrets } = require('./secrets');

const API_VERSION = '1.16.1';
const ERR = { generic: 0, missing: 10, wrongCreds: 40, badAuth: 42, conflict: 43, badKey: 44, forbidden: 50, notFound: 70 };
const FAVORITES = 'Favoris'; // les titres « étoilés » sont rangés dans cette playlist (visible aussi dans control)

class SubsonicError extends Error {
  constructor(code, message) {
    super(message);
    this.code = code;
  }
}

const fold = (s) => String(s || '').normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase().replace(/\s+/g, ' ').trim();
const hash = (s) => crypto.createHash('sha1').update(String(s)).digest('base64url').slice(0, 12);
const iso = (ms) => new Date(ms || Date.now()).toISOString();
const clean = (obj) => Object.fromEntries(Object.entries(obj).filter(([, v]) => v !== undefined && v !== null && v !== ''));
const num = (v, fallback) => (v !== null && v !== undefined && v !== '' && Number.isFinite(Number(v)) ? Number(v) : fallback);
const md5 = (s) => crypto.createHash('md5').update(s, 'utf8').digest('hex');
const safeEqual = (a, b) => {
  const x = Buffer.from(String(a));
  const y = Buffer.from(String(b));
  return x.length === y.length && crypto.timingSafeEqual(x, y);
};

// ─── Sérialisation ─────────────────────────────────────────────────────────────
const XML_TEXT = new Set(['lyrics']); // éléments dont la clé `value` est le texte de l'élément
const esc = (v) => String(v).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c])).replace(/[\0-\x08\x0b\x0c\x0e-\x1f]/g, '');

/** { cle: valeur } → <nom cle="valeur">…</nom> : les valeurs simples deviennent des attributs, objets et tableaux des éléments. */
function toXml(name, obj) {
  let attrs = '';
  let children = '';
  let text = '';
  for (const [key, value] of Object.entries(obj || {})) {
    if (value === undefined || value === null) continue;
    if (Array.isArray(value)) children += value.map((item) => (item && typeof item === 'object' ? toXml(key, item) : `<${key}>${esc(item)}</${key}>`)).join('');
    else if (typeof value === 'object') children += toXml(key, value);
    else if (key === 'value' && XML_TEXT.has(name)) text = esc(value);
    else attrs += ` ${key}="${esc(value)}"`;
  }
  return children || text ? `<${name}${attrs}>${text}${children}</${name}>` : `<${name}${attrs}/>`;
}

function reply(res, params, payload, { failed = null } = {}) {
  const base = { status: failed ? 'failed' : 'ok', version: API_VERSION, type: 'control', serverVersion: require('../package.json').version, openSubsonic: true };
  const body = failed ? { ...base, error: { code: failed.code, message: failed.message } } : { ...base, ...payload };
  const format = String(params.get('f') || 'xml').toLowerCase();
  if (format === 'json' || format === 'jsonp') {
    const json = JSON.stringify({ 'subsonic-response': body });
    const callback = String(params.get('callback') || '');
    const wrapped = format === 'jsonp' && /^[\w.$]+$/.test(callback) ? `${callback}(${json});` : json;
    return send(res, 200, wrapped, { 'content-type': format === 'jsonp' ? 'text/javascript; charset=utf-8' : 'application/json; charset=utf-8', 'cache-control': 'no-store' });
  }
  const xml = `<?xml version="1.0" encoding="UTF-8"?>\n${toXml('subsonic-response', { xmlns: 'http://subsonic.org/restapi', ...body })}`;
  return send(res, 200, xml, { 'content-type': 'text/xml; charset=utf-8', 'cache-control': 'no-store' });
}

// ─── Modèle de la bibliothèque d'un compte ─────────────────────────────────────
/** Artistes, albums et genres déduits des tags des morceaux (gardés tant que les morceaux ne changent pas). */
function buildModel(library) {
  const tracks = library.tracks();
  const artists = new Map();
  const albums = new Map();
  const genres = new Map();
  const songs = new Map();
  for (const t of tracks) {
    songs.set(t.id, t);
    if (t.kind !== 'audio') continue;
    const artistName = (t.albumArtist || t.artist || '[Artiste inconnu]').trim();
    const albumName = (t.album || '[Album inconnu]').trim();
    const artistId = `ar-${hash(fold(artistName))}`;
    const albumId = `al-${hash(`${fold(artistName)}\0${fold(albumName)}`)}`;
    let artist = artists.get(artistId);
    if (!artist) artists.set(artistId, (artist = { id: artistId, name: artistName, albums: new Set(), cover: null }));
    let album = albums.get(albumId);
    if (!album) {
      albums.set(albumId, (album = { id: albumId, name: albumName, artistId, artist: artistName, year: t.year || null, genre: t.genre || '', cover: t.cover || null, tracks: [], addedAt: 0, lastPlayed: 0, playCount: 0, duration: 0 }));
      artist.albums.add(albumId);
    }
    album.tracks.push(t);
    album.addedAt = Math.max(album.addedAt, t.addedAt || 0);
    album.lastPlayed = Math.max(album.lastPlayed, t.lastPlayed || 0);
    album.playCount += t.playCount || 0;
    album.duration += t.duration || 0;
    if (!album.cover && t.cover) album.cover = t.cover;
    if (!album.year && t.year) album.year = t.year;
    if (!artist.cover && album.cover) artist.cover = album.cover;
    t._albumId = albumId;
    t._artistId = artistId;
    for (const g of String(t.genre || '').split(/[,;/]/).map((x) => x.trim()).filter(Boolean)) {
      const e = genres.get(g) || { value: g, songCount: 0, albums: new Set() };
      e.songCount++;
      e.albums.add(albumId);
      genres.set(g, e);
    }
  }
  for (const album of albums.values()) album.tracks.sort((a, b) => (a.discNo || 1) - (b.discNo || 1) || (a.trackNo || 0) - (b.trackNo || 0) || a.title.localeCompare(b.title));
  return { rev: library.rev, songs, artists, albums, genres };
}

class Subsonic {
  constructor(hub) {
    this.hub = hub;
    this.secrets = new Secrets(hub.config.dataDir);
    this.failures = hub.subsonicThrottle;
    this.small = new Map(); // pochettes réduites déjà demandées
  }

  // ─── Mot de passe d'application ────────────────────────────────────────────
  secretOf(userId) {
    const row = this.hub.db.prepare('SELECT subsonic_secret AS s FROM users WHERE id = ?').get(userId);
    return row?.s ? this.secrets.open(row.s) : null;
  }

  /** Le secret du compte ; en crée un si besoin (ou `renew`). */
  ensureSecret(userId, { renew = false } = {}) {
    let secret = renew ? null : this.secretOf(userId);
    if (!secret) {
      secret = crypto.randomBytes(18).toString('base64url');
      this.hub.db.prepare('UPDATE users SET subsonic_secret = ?, subsonic_key_hash = ? WHERE id = ?').run(this.secrets.seal(secret), crypto.createHash('sha256').update(secret).digest('hex'), userId);
    }
    return secret;
  }

  revokeSecret(userId) {
    this.hub.db.prepare('UPDATE users SET subsonic_secret = NULL, subsonic_key_hash = NULL WHERE id = ?').run(userId);
  }

  // ─── Authentification ──────────────────────────────────────────────────────
  authenticate(params) {
    if (!this.hub.settings.subsonicEnabled) throw new SubsonicError(ERR.forbidden, 'L’API Subsonic est désactivée sur ce serveur.');
    const apiKey = params.get('apiKey');
    const name = params.get('u');
    if (apiKey && name) throw new SubsonicError(ERR.conflict, 'Plusieurs méthodes d’authentification fournies.');
    let user = null;
    if (apiKey) {
      const row = this.hub.db.prepare('SELECT * FROM users WHERE subsonic_key_hash = ? AND disabled = 0').get(crypto.createHash('sha256').update(String(apiKey)).digest('hex'));
      if (!row) throw new SubsonicError(ERR.badKey, 'Clé d’API invalide.');
      user = row;
    } else {
      if (!name) throw new SubsonicError(ERR.missing, 'Paramètre manquant : u.');
      const row = this.hub.db.prepare('SELECT * FROM users WHERE username = ? COLLATE NOCASE AND disabled = 0').get(name);
      const secret = row ? this.secretOf(row.id) : null;
      let ok = false;
      if (secret) {
        if (params.get('t') && params.get('s')) ok = safeEqual(md5(secret + params.get('s')), String(params.get('t')).toLowerCase());
        else if (params.get('p') !== null) {
          const p = String(params.get('p'));
          const given = p.startsWith('enc:') ? Buffer.from(p.slice(4), 'hex').toString('utf8') : p;
          ok = safeEqual(given, secret);
        } else throw new SubsonicError(ERR.missing, 'Authentification manquante (p, ou t et s).');
      }
      if (!ok) throw new SubsonicError(ERR.wrongCreds, 'Nom d’utilisateur ou mot de passe d’application incorrect.');
      user = row;
    }
    return user;
  }

  // ─── Requête ───────────────────────────────────────────────────────────────
  async handle(req, res, url) {
    let method = decodeURIComponent(url.pathname.slice('/rest/'.length)).replace(/\.view$/i, '').toLowerCase();
    let params = url.searchParams;
    if (req.method === 'POST' && /application\/x-www-form-urlencoded/i.test(req.headers['content-type'] || '')) {
      const body = await readText(req).catch(() => '');
      params = new URLSearchParams([...url.searchParams, ...new URLSearchParams(String(body))]);
    }
    const ip = (req.socket.remoteAddress || '').replace(/^::ffff:/, '');
    try {
      if (this.failures.wait(ip)) throw new SubsonicError(ERR.wrongCreds, 'Trop d’essais : patientez quelques minutes.');
      let user;
      try {
        user = this.authenticate(params);
      } catch (err) {
        if (err.code === ERR.wrongCreds || err.code === ERR.badKey) this.failures.fail(ip);
        throw err;
      }
      if (!params.get('c') && !['ping'].includes(method)) throw new SubsonicError(ERR.missing, 'Paramètre manquant : c.');
      const handler = this.methods[method];
      if (!handler) throw new SubsonicError(ERR.generic, `Méthode inconnue : ${method}`);
      const ctx = { user, params, req, res, entry: this.hub.entry(user), url };
      const result = await handler.call(this, ctx);
      if (result === undefined) return undefined; // le gestionnaire a répondu lui-même (flux, image)
      return reply(res, params, result || {});
    } catch (err) {
      if (res.headersSent) { res.destroy(); return undefined; }
      if (err instanceof SubsonicError) return reply(res, params, null, { failed: { code: err.code, message: err.message } });
      console.error('[subsonic]', method, err);
      return reply(res, params, null, { failed: { code: ERR.generic, message: 'Erreur du serveur.' } });
    }
  }

  // ─── Outils des méthodes ───────────────────────────────────────────────────
  model(entry) {
    const library = entry.core.library;
    if (!entry.subModel || entry.subModel.rev !== library.rev) entry.subModel = buildModel(library);
    return entry.subModel;
  }

  need(params, name) {
    const v = params.get(name);
    if (v === null || v === '') throw new SubsonicError(ERR.missing, `Paramètre manquant : ${name}.`);
    return v;
  }

  coverOf(hashOrNull) {
    return hashOrNull ? `cv-${hashOrNull}` : undefined;
  }

  song(t, model) {
    const ext = path.extname(t.path).slice(1).toLowerCase();
    const album = model.albums.get(t._albumId);
    return clean({
      id: String(t.id),
      parent: t._albumId,
      isDir: false,
      title: t.title,
      album: album?.name || t.album,
      artist: t.artist || album?.artist,
      track: t.trackNo || undefined,
      discNumber: t.discNo || undefined,
      year: t.year || undefined,
      genre: t.genre || undefined,
      coverArt: this.coverOf(t.cover),
      size: t.size,
      contentType: mimeOf(t.path),
      suffix: ext,
      duration: Math.round(t.duration || 0),
      bitRate: t.bitrate || undefined,
      path: `${album?.artist || 'Inconnu'}/${album?.name || 'Inconnu'}/${path.basename(t.path)}`,
      playCount: t.playCount || undefined,
      created: iso(t.addedAt),
      albumId: t._albumId,
      artistId: t._artistId,
      type: t.kind === 'video' ? 'video' : 'music',
      isVideo: t.kind === 'video',
      mediaType: 'song',
    });
  }

  albumId3(a) {
    return clean({ id: a.id, name: a.name, artist: a.artist, artistId: a.artistId, coverArt: this.coverOf(a.cover), songCount: a.tracks.length, duration: Math.round(a.duration), playCount: a.playCount || undefined, created: iso(a.addedAt), year: a.year || undefined, genre: a.genre || undefined });
  }

  albumDir(a) {
    return clean({ id: a.id, parent: a.artistId, isDir: true, title: a.name, album: a.name, artist: a.artist, year: a.year || undefined, genre: a.genre || undefined, coverArt: this.coverOf(a.cover), playCount: a.playCount || undefined, created: iso(a.addedAt) });
  }

  artistId3(ar, model) {
    return clean({ id: ar.id, name: ar.name, albumCount: ar.albums.size, coverArt: this.coverOf(ar.cover) });
  }

  indexes(model, key = 'artistId3') {
    const letters = new Map();
    const list = [...model.artists.values()].sort((a, b) => fold(a.name).localeCompare(fold(b.name)));
    for (const ar of list) {
      const first = fold(ar.name).charAt(0).toUpperCase();
      const letter = /[A-Z]/.test(first) ? first : '#';
      if (!letters.has(letter)) letters.set(letter, []);
      letters.get(letter).push(key === 'artistId3' ? this.artistId3(ar, model) : clean({ id: ar.id, name: ar.name }));
    }
    return [...letters].map(([name, artist]) => ({ name, artist }));
  }

  page(list, params, sizeName = 'size', offsetName = 'offset', max = 500) {
    const size = Math.max(0, Math.min(max, num(params.get(sizeName), 10)));
    const offset = Math.max(0, num(params.get(offsetName), 0));
    return list.slice(offset, offset + size);
  }

  favorites(entry) {
    const data = entry.core.library.playlists();
    const pl = data.playlists.find((p) => p.name === FAVORITES && p.kind === 'manual');
    return { pl, ids: pl ? data.items.filter((i) => i.playlistId === pl.id).map((i) => i.trackId) : [] };
  }

  playlistInfo(p, user, items, model) {
    const tracks = items.map((i) => model.songs.get(i.trackId)).filter(Boolean);
    return clean({
      id: `pl-${p.id}`,
      name: p.name,
      comment: p.description || undefined,
      owner: user.username,
      public: !!p.shared,
      songCount: tracks.length,
      duration: Math.round(tracks.reduce((s, t) => s + (t.duration || 0), 0)),
      created: iso(p.createdAt),
      changed: iso(p.updatedAt),
      coverArt: this.coverOf(p.cover || tracks.find((t) => t.cover)?.cover),
    });
  }

  playlistId(params, name = 'id') {
    const m = /^pl-(\d+)$/.exec(this.need(params, name));
    if (!m) throw new SubsonicError(ERR.notFound, 'Playlist introuvable.');
    return Number(m[1]);
  }

  trackFile(ctx, id) {
    const library = ctx.entry.core.library;
    const track = Number.isInteger(Number(id)) ? library.tracks().find((t) => t.id === Number(id)) : null; // contrôle d'existence
    const file = track ? library.trackPath(track.id) : null;
    if (!file || !this.hub.isInsideRoots(ctx.user, file)) throw new SubsonicError(ERR.notFound, 'Morceau introuvable.');
    return file;
  }
}

// ─── Méthodes ──────────────────────────────────────────────────────────────────
const M = {};

M.ping = () => ({});
M.getlicense = () => ({ license: { valid: true, email: '', licenseExpires: '2099-12-31T00:00:00.000Z' } });
M.getopensubsonicextensions = () => ({ openSubsonicExtensions: [{ name: 'apiKeyAuthentication', versions: [1] }, { name: 'formPost', versions: [1] }, { name: 'songLyrics', versions: [1] }] });
M.tokeninfo = (ctx) => ({ tokenInfo: { username: ctx.user.username } });
M.getmusicfolders = () => ({ musicFolders: { musicFolder: [{ id: 1, name: 'Bibliothèque' }] } });

M.getuser = (ctx) => {
  const admin = ctx.user.role === 'admin';
  return { user: { username: ctx.user.username, email: '', scrobblingEnabled: true, adminRole: false, settingsRole: false, downloadRole: true, uploadRole: false, playlistRole: true, coverArtRole: false, commentRole: false, podcastRole: false, streamRole: true, jukeboxRole: false, shareRole: false, videoConversionRole: false, folder: [1], ...(admin ? {} : {}) } };
};

M.getindexes = function indexes(ctx) {
  const model = this.model(ctx.entry);
  return { indexes: { lastModified: Date.now(), ignoredArticles: '', index: this.indexes(model, 'artist') } };
};
M.getartists = function artists(ctx) {
  const model = this.model(ctx.entry);
  return { artists: { ignoredArticles: '', lastModified: Date.now(), index: this.indexes(model) } };
};

M.getartist = function artist(ctx) {
  const model = this.model(ctx.entry);
  const ar = model.artists.get(this.need(ctx.params, 'id'));
  if (!ar) throw new SubsonicError(ERR.notFound, 'Artiste introuvable.');
  const albums = [...ar.albums].map((id) => model.albums.get(id)).sort((a, b) => (a.year || 0) - (b.year || 0) || fold(a.name).localeCompare(fold(b.name)));
  return { artist: { ...this.artistId3(ar, model), album: albums.map((a) => this.albumId3(a)) } };
};

M.getalbum = function album(ctx) {
  const model = this.model(ctx.entry);
  const a = model.albums.get(this.need(ctx.params, 'id'));
  if (!a) throw new SubsonicError(ERR.notFound, 'Album introuvable.');
  return { album: { ...this.albumId3(a), song: a.tracks.map((t) => this.song(t, model)) } };
};

M.getsong = function song(ctx) {
  const model = this.model(ctx.entry);
  const t = model.songs.get(Number(this.need(ctx.params, 'id')));
  if (!t) throw new SubsonicError(ERR.notFound, 'Morceau introuvable.');
  return { song: this.song(t, model) };
};

M.getvideos = function videos(ctx) {
  const model = this.model(ctx.entry);
  return { videos: { video: [...model.songs.values()].filter((t) => t.kind === 'video').map((t) => this.song(t, model)) } };
};

/** Navigation par dossiers (anciens clients) : artiste → albums → morceaux. */
M.getmusicdirectory = function directory(ctx) {
  const model = this.model(ctx.entry);
  const id = this.need(ctx.params, 'id');
  const ar = model.artists.get(id);
  if (ar) return { directory: { id: ar.id, name: ar.name, child: [...ar.albums].map((a) => this.albumDir(model.albums.get(a))) } };
  const a = model.albums.get(id);
  if (a) return { directory: { id: a.id, parent: a.artistId, name: a.name, child: a.tracks.map((t) => this.song(t, model)) } };
  throw new SubsonicError(ERR.notFound, 'Dossier introuvable.');
};

M.getgenres = function genres(ctx) {
  const model = this.model(ctx.entry);
  return { genres: { genre: [...model.genres.values()].sort((a, b) => a.value.localeCompare(b.value)).map((g) => ({ value: g.value, songCount: g.songCount, albumCount: g.albums.size })) } };
};

/** Listes d'albums : random, newest, frequent, recent, starred, alphabeticalByName / ByArtist, byYear, byGenre. */
function albumList(ctx, format) {
  const model = this.model(ctx.entry);
  const type = String(ctx.params.get('type') || 'alphabeticalByName');
  let list = [...model.albums.values()];
  const byName = (a, b) => fold(a.name).localeCompare(fold(b.name));
  switch (type) {
    case 'random': list.sort(() => Math.random() - 0.5); break;
    case 'newest': list.sort((a, b) => b.addedAt - a.addedAt); break;
    case 'frequent': list = list.filter((a) => a.playCount).sort((a, b) => b.playCount - a.playCount); break;
    case 'recent': list = list.filter((a) => a.lastPlayed).sort((a, b) => b.lastPlayed - a.lastPlayed); break;
    case 'starred': list = []; break;
    case 'alphabeticalByArtist': list.sort((a, b) => fold(a.artist).localeCompare(fold(b.artist)) || byName(a, b)); break;
    case 'byYear': {
      const from = num(ctx.params.get('fromYear'), 0);
      const to = num(ctx.params.get('toYear'), 9999);
      list = list.filter((a) => a.year && a.year >= Math.min(from, to) && a.year <= Math.max(from, to));
      list.sort((a, b) => (from <= to ? a.year - b.year : b.year - a.year) || byName(a, b));
      break;
    }
    case 'byGenre': {
      const g = fold(ctx.params.get('genre'));
      list = list.filter((a) => a.tracks.some((t) => String(t.genre || '').split(/[,;/]/).some((x) => fold(x) === g)));
      list.sort(byName);
      break;
    }
    default: list.sort(byName);
  }
  const rows = this.page(list, ctx.params).map((a) => (format === 'id3' ? this.albumId3(a) : this.albumDir(a)));
  return format === 'id3' ? { albumList2: { album: rows } } : { albumList: { album: rows } };
}
M.getalbumlist = function list(ctx) { return albumList.call(this, ctx, 'dir'); };
M.getalbumlist2 = function list2(ctx) { return albumList.call(this, ctx, 'id3'); };

M.getrandomsongs = function random(ctx) {
  const model = this.model(ctx.entry);
  const g = ctx.params.get('genre') ? fold(ctx.params.get('genre')) : null;
  const from = num(ctx.params.get('fromYear'), 0);
  const to = num(ctx.params.get('toYear'), 9999);
  const pool = [...model.songs.values()].filter((t) => t.kind === 'audio' && (!g || String(t.genre || '').split(/[,;/]/).some((x) => fold(x) === g)) && (!t.year || (t.year >= from && t.year <= to)));
  const out = [];
  for (let i = 0; i < Math.min(num(ctx.params.get('size'), 10), 500) && pool.length; i++) out.push(pool.splice(Math.floor(Math.random() * pool.length), 1)[0]);
  return { randomSongs: { song: out.map((t) => this.song(t, model)) } };
};

M.getsongsbygenre = function byGenre(ctx) {
  const model = this.model(ctx.entry);
  const g = fold(this.need(ctx.params, 'genre'));
  const list = [...model.songs.values()].filter((t) => t.kind === 'audio' && String(t.genre || '').split(/[,;/]/).some((x) => fold(x) === g));
  return { songsByGenre: { song: this.page(list, ctx.params, 'count', 'offset').map((t) => this.song(t, model)) } };
};

M.gettopsongs = function top(ctx) {
  const model = this.model(ctx.entry);
  const name = fold(this.need(ctx.params, 'artist'));
  const list = [...model.songs.values()].filter((t) => fold(t.artist) === name || fold(t.albumArtist) === name).sort((a, b) => (b.playCount || 0) - (a.playCount || 0));
  return { topSongs: { song: list.slice(0, num(ctx.params.get('count'), 50)).map((t) => this.song(t, model)) } };
};

M.getsimilarsongs = () => ({ similarSongs: {} });
M.getsimilarsongs2 = () => ({ similarSongs2: {} });
M.getartistinfo = () => ({ artistInfo: {} });
M.getartistinfo2 = () => ({ artistInfo2: {} });
M.getalbuminfo = () => ({ albumInfo: {} });
M.getalbuminfo2 = () => ({ albumInfo: {} });

function search(ctx, kind) {
  const model = this.model(ctx.entry);
  const q = fold(ctx.params.get('query')).replace(/\*/g, ' ').trim();
  const tokens = q.split(' ').filter(Boolean);
  const match = (text) => tokens.every((tk) => text.includes(tk));
  const rank = (text) => (text.startsWith(q) ? 0 : 1);
  const take = (list, count, offset) => list.slice(num(ctx.params.get(offset), 0), num(ctx.params.get(offset), 0) + Math.min(500, num(ctx.params.get(count), 20)));
  const artists = [...model.artists.values()].filter((a) => !tokens.length || match(fold(a.name))).sort((a, b) => rank(fold(a.name)) - rank(fold(b.name)) || fold(a.name).localeCompare(fold(b.name)));
  const albums = [...model.albums.values()].filter((a) => !tokens.length || match(fold(`${a.name} ${a.artist}`))).sort((a, b) => rank(fold(a.name)) - rank(fold(b.name)) || fold(a.name).localeCompare(fold(b.name)));
  const songs = [...model.songs.values()].filter((t) => !tokens.length || match(fold(`${t.title} ${t.artist} ${t.album}`))).sort((a, b) => rank(fold(a.title)) - rank(fold(b.title)) || fold(a.title).localeCompare(fold(b.title)));
  const id3 = kind === 'search3';
  const result = {
    artist: take(artists, 'artistCount', 'artistOffset').map((a) => (id3 ? this.artistId3(a, model) : clean({ id: a.id, name: a.name }))),
    album: take(albums, 'albumCount', 'albumOffset').map((a) => (id3 ? this.albumId3(a) : this.albumDir(a))),
    song: take(songs, 'songCount', 'songOffset').map((t) => this.song(t, model)),
  };
  return { [kind === 'search3' ? 'searchResult3' : 'searchResult2']: result };
}
M.search2 = function s2(ctx) { return search.call(this, ctx, 'search2'); };
M.search3 = function s3(ctx) { return search.call(this, ctx, 'search3'); };

// ─── Favoris (playlist « Favoris ») ────────────────────────────────────────────
function starred(ctx) {
  const model = this.model(ctx.entry);
  const { ids } = this.favorites(ctx.entry);
  return { song: ids.map((id) => model.songs.get(id)).filter(Boolean).map((t) => this.song(t, model)), album: [], artist: [] };
}
M.getstarred = function gs(ctx) { return { starred: starred.call(this, ctx) }; };
M.getstarred2 = function gs2(ctx) { return { starred2: starred.call(this, ctx) }; };

function annotate(ctx, on) {
  const library = ctx.entry.core.library;
  const ids = ctx.params.getAll('id').map(Number).filter(Number.isInteger);
  let { pl } = this.favorites(ctx.entry);
  if (!pl && !on) return {};
  if (!pl) pl = { id: library.createPlaylist(FAVORITES, 'manual', null) };
  const data = library.playlists();
  const present = new Map(data.items.filter((i) => i.playlistId === pl.id).map((i) => [i.trackId, i.id]));
  if (on) library.addToPlaylist(pl.id, ids.filter((id) => !present.has(id)));
  else library.removeFromPlaylist(ids.map((id) => present.get(id)).filter(Boolean));
  return {};
}
M.star = function star(ctx) { return annotate.call(this, ctx, true); };
M.unstar = function unstar(ctx) { return annotate.call(this, ctx, false); };
M.setrating = () => ({});

// ─── Playlists ─────────────────────────────────────────────────────────────────
M.getplaylists = function playlists(ctx) {
  const model = this.model(ctx.entry);
  const data = ctx.entry.core.library.playlists();
  return { playlists: { playlist: data.playlists.filter((p) => p.kind === 'manual').map((p) => this.playlistInfo(p, ctx.user, data.items.filter((i) => i.playlistId === p.id), model)) } };
};

M.getplaylist = function playlist(ctx) {
  const model = this.model(ctx.entry);
  const id = this.playlistId(ctx.params);
  const data = ctx.entry.core.library.playlists();
  const p = data.playlists.find((x) => x.id === id && x.kind === 'manual');
  if (!p) throw new SubsonicError(ERR.notFound, 'Playlist introuvable.');
  const items = data.items.filter((i) => i.playlistId === id);
  return { playlist: { ...this.playlistInfo(p, ctx.user, items, model), entry: items.map((i) => model.songs.get(i.trackId)).filter(Boolean).map((t) => this.song(t, model)) } };
};

M.createplaylist = function create(ctx) {
  const library = ctx.entry.core.library;
  const songs = ctx.params.getAll('songId').map(Number).filter(Number.isInteger);
  let id;
  if (ctx.params.get('playlistId')) {
    id = this.playlistId(ctx.params, 'playlistId'); // remplace le contenu d'une playlist existante
    const data = library.playlists();
    if (!data.playlists.some((p) => p.id === id && p.kind === 'manual')) throw new SubsonicError(ERR.notFound, 'Playlist introuvable.');
    library.removeFromPlaylist(data.items.filter((i) => i.playlistId === id).map((i) => i.id));
    if (ctx.params.get('name')) library.renamePlaylist(id, ctx.params.get('name'));
  } else {
    id = library.createPlaylist(this.need(ctx.params, 'name'), 'manual', null);
  }
  if (songs.length) library.addToPlaylist(id, songs);
  return M.getplaylist.call(this, { ...ctx, params: new URLSearchParams({ id: `pl-${id}` }) });
};

M.updateplaylist = function update(ctx) {
  const library = ctx.entry.core.library;
  const id = this.playlistId(ctx.params, 'playlistId');
  const data = library.playlists();
  if (!data.playlists.some((p) => p.id === id && p.kind === 'manual')) throw new SubsonicError(ERR.notFound, 'Playlist introuvable.');
  const patch = {};
  if (ctx.params.get('name')) patch.name = ctx.params.get('name');
  if (ctx.params.get('comment') !== null) patch.description = ctx.params.get('comment');
  if (ctx.params.get('public') !== null) patch.shared = ctx.params.get('public') === 'true';
  if (Object.keys(patch).length) library.updatePlaylist(id, patch);
  const items = data.items.filter((i) => i.playlistId === id).sort((a, b) => a.position - b.position);
  const remove = ctx.params.getAll('songIndexToRemove').map(Number).filter((i) => Number.isInteger(i) && items[i]).map((i) => items[i].id);
  if (remove.length) library.removeFromPlaylist(remove);
  const add = ctx.params.getAll('songIdToAdd').map(Number).filter(Number.isInteger);
  if (add.length) library.addToPlaylist(id, add);
  return {};
};

M.deleteplaylist = function del(ctx) {
  const id = this.playlistId(ctx.params);
  const library = ctx.entry.core.library;
  if (!library.playlists().playlists.some((p) => p.id === id && p.kind === 'manual')) throw new SubsonicError(ERR.notFound, 'Playlist introuvable.');
  library.deletePlaylist(id);
  return {};
};

// ─── Médias ────────────────────────────────────────────────────────────────────
M.stream = function stream(ctx) {
  const file = this.trackFile(ctx, this.need(ctx.params, 'id'));
  return sendFile(ctx.req, ctx.res, file, { cache: 'private, no-store' }).then(() => undefined);
};

M.download = function download(ctx) {
  const file = this.trackFile(ctx, this.need(ctx.params, 'id'));
  const name = path.basename(file).replace(/[^\w. -]/g, '_');
  return sendFile(ctx.req, ctx.res, file, { cache: 'private, no-store', headers: { 'content-disposition': `attachment; filename="${name}"` } }).then(() => undefined);
};

M.getcoverart = function cover(ctx) {
  const id = String(this.need(ctx.params, 'id'));
  const model = this.model(ctx.entry);
  let hashed = null;
  if (id.startsWith('cv-')) hashed = id.slice(3);
  else if (id.startsWith('al-')) hashed = model.albums.get(id)?.cover;
  else if (id.startsWith('ar-')) hashed = model.artists.get(id)?.cover;
  else if (id.startsWith('pl-')) {
    const data = ctx.entry.core.library.playlists();
    const p = data.playlists.find((x) => x.id === Number(id.slice(3)));
    hashed = p?.cover || data.items.filter((i) => i.playlistId === p?.id).map((i) => model.songs.get(i.trackId)?.cover).find(Boolean);
  } else hashed = model.songs.get(Number(id))?.cover;
  if (!hashed || !/^[0-9a-f]{20}$/.test(hashed)) throw new SubsonicError(ERR.notFound, 'Pas de pochette.');
  const size = num(ctx.params.get('size'), 0);
  const small = path.join(this.hub.coversDir, `${hashed}-s.jpg`);
  const file = size && size <= 256 && fs.existsSync(small) ? small : path.join(this.hub.coversDir, `${hashed}.jpg`);
  if (!fs.existsSync(file)) throw new SubsonicError(ERR.notFound, 'Pochette introuvable.');
  return sendFile(ctx.req, ctx.res, file, { type: 'image/jpeg', cache: 'private, max-age=86400' }).then(() => undefined);
};

M.getavatar = function avatar(ctx) {
  const row = this.hub.db.prepare('SELECT avatar FROM users WHERE username = ? COLLATE NOCASE').get(this.need(ctx.params, 'username'));
  if (!row?.avatar || !/^[0-9a-f]{20}$/.test(row.avatar)) throw new SubsonicError(ERR.notFound, 'Pas d’avatar.');
  return sendFile(ctx.req, ctx.res, path.join(this.hub.avatarsDir, `${row.avatar}.jpg`), { type: 'image/jpeg', cache: 'private, max-age=86400' }).then(() => undefined);
};

M.getlyricsbysongid = async function lyrics(ctx) {
  const id = Number(this.need(ctx.params, 'id'));
  const model = this.model(ctx.entry);
  const t = model.songs.get(id);
  if (!t) throw new SubsonicError(ERR.notFound, 'Morceau introuvable.');
  const res = await ctx.entry.core.lyrics.get(id, {}).catch(() => null);
  const synced = res?.synced ? String(res.synced).split('\n').map((l) => /^\[(\d+):(\d+(?:\.\d+)?)\]\s*(.*)$/.exec(l)).filter(Boolean).map((m) => ({ start: Math.round((Number(m[1]) * 60 + Number(m[2])) * 1000), value: m[3] })) : [];
  const plain = res?.plain ? String(res.plain).split('\n').map((value) => ({ value })) : [];
  const lines = synced.length ? synced : plain;
  if (!lines.length) return { lyricsList: {} };
  return { lyricsList: { structuredLyrics: [{ displayArtist: t.artist, displayTitle: t.title, lang: 'xxx', synced: synced.length > 0, line: lines }] } };
};

M.getlyrics = async function legacyLyrics(ctx) {
  const model = this.model(ctx.entry);
  const artist = fold(ctx.params.get('artist'));
  const title = fold(ctx.params.get('title'));
  const t = [...model.songs.values()].find((x) => fold(x.title) === title && (!artist || fold(x.artist) === artist));
  const res = t ? await ctx.entry.core.lyrics.get(t.id, {}).catch(() => null) : null;
  const text = res?.plain || String(res?.synced || '').replace(/^\[[^\]]*\]\s*/gm, '');
  return { lyrics: clean({ artist: t?.artist, title: t?.title, value: text || undefined }) };
};

// ─── Écoutes ───────────────────────────────────────────────────────────────────
/** Un lecteur annonce qu'il joue un morceau (submission=false) ou qu'il l'a écouté (submission=true, comptée comme écoute entière). */
M.scrobble = function scrobble(ctx) {
  const ids = ctx.params.getAll('id').map(Number).filter(Number.isInteger);
  if (!ids.length) throw new SubsonicError(ERR.missing, 'Paramètre manquant : id.');
  const times = ctx.params.getAll('time').map(Number);
  const submission = ctx.params.get('submission') !== 'false';
  const core = ctx.entry.core;
  const model = this.model(ctx.entry);
  ids.forEach((id, i) => {
    const t = model.songs.get(id);
    if (!t) return;
    if (!submission) {
      core.scrobbler.nowPlaying(id);
      return;
    }
    const playId = core.library.playStart(id, t.duration);
    core.library.playEnd(playId, t.duration, true);
    core.scrobbler.submit(id, Number.isFinite(times[i]) && times[i] > 0 ? Math.floor(times[i] / 1000) : Math.floor(Date.now() / 1000));
  });
  return {};
};

M.getnowplaying = () => ({ nowPlaying: {} });

// ─── Divers : vides ou refusés ─────────────────────────────────────────────────
M.getscanstatus = function status(ctx) { return { scanStatus: { scanning: !!ctx.entry.core.library.scanning, count: this.model(ctx.entry).songs.size } }; };
M.startscan = function scan(ctx) {
  ctx.entry.core.library.scan(this.hub.rootsFor(ctx.user.id)).catch(() => {});
  return { scanStatus: { scanning: true, count: this.model(ctx.entry).songs.size } };
};
M.getbookmarks = () => ({ bookmarks: {} });
M.createbookmark = () => ({});
M.deletebookmark = () => ({});
M.getplayqueue = () => ({ playQueue: undefined });
M.saveplayqueue = () => ({});
M.getinternetradiostations = () => ({ internetRadioStations: {} });
M.getpodcasts = () => ({ podcasts: {} });
M.getnewestpodcasts = () => ({ newestPodcasts: {} });
M.getshares = () => ({ shares: {} });
M.jukeboxcontrol = () => { throw new SubsonicError(ERR.forbidden, 'Le mode jukebox n’est pas pris en charge.'); };
M.getusers = () => { throw new SubsonicError(ERR.forbidden, 'Réservé à l’administration du serveur.'); };

Subsonic.prototype.methods = M;

module.exports = { Subsonic, SubsonicError, toXml, buildModel, FAVORITES };

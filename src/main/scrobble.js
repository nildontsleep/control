'use strict';

// Scrobbling : envoie ce qu'on écoute à ListenBrainz et à Last.fm (« en écoute » au début d'un morceau, puis l'écoute
// complète quand elle compte : au moins 30 s de morceau, et la moitié écoutée ou 4 minutes). Les écoutes qui n'ont pas
// pu partir (réseau coupé…) sont gardées dans une file et renvoyées plus tard. Partagé par le bureau et le serveur web ;
// les jetons vivent dans un fichier à part (scrobble.json), jamais dans les réglages que la page reçoit.

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');

const LB_DEFAULT = 'https://api.listenbrainz.org';
const LASTFM = 'https://ws.audioscrobbler.com/2.0/';
const MAX_QUEUE = 500;
const MAX_AGE_S = 14 * 24 * 3600; // Last.fm refuse les écoutes plus vieilles
const RETRY_MS = 5 * 60 * 1000;
const AGENT = 'control (https://github.com/nildontsleep/control)';

/** Une écoute compte : morceau de 30 s au moins, écouté pendant la moitié de sa durée ou 4 minutes. */
const counts = (duration, seconds) => duration >= 30 && seconds >= Math.min(240, duration / 2);

const md5 = (s) => crypto.createHash('md5').update(s, 'utf8').digest('hex');

class Scrobbler {
  /**
   * @param {object} o
   * @param {string} o.dir  dossier du compte (scrobble.json)
   * @param {object} o.library  pour retrouver un morceau
   * @param {Function} o.emit  (channel, payload) → événement pour les pages ouvertes
   * @param {Function} [o.keys]  () → { key, secret } : l'application Last.fm (serveur : réglée par l'administrateur)
   * @param {Function} [o.guardUrl]  (url) → url : refuse les adresses internes (serveur)
   * @param {Function} [o.fetch]
   */
  constructor({ dir, library, emit, keys, guardUrl, fetch: fetchFn }) {
    this.file = path.join(dir, 'scrobble.json');
    this.library = library;
    this.emit = emit;
    this.keysFn = keys || null;
    this.guardUrl = guardUrl || (async (url) => url);
    this.fetch = fetchFn || ((...args) => globalThis.fetch(...args));
    this.data = this.#read();
    this.plays = new Map(); // playId → { track, startedAt }
    this.timer = null;
    this.flushing = null;
    this.closed = false;
  }

  /** L'application Last.fm : celle du serveur (réglée par l'administrateur), sinon celle que l'utilisateur a saisie. */
  keys() {
    const k = this.keysFn?.();
    return k?.key && k?.secret ? k : this.data.lastfmApp || null;
  }

  setLastfmKeys(key, secret) {
    const k = String(key || '').trim();
    const sec = String(secret || '').trim();
    this.data.lastfmApp = k && sec ? { key: k, secret: sec } : null;
    this.#changed();
    return this.status();
  }

  #read() {
    try {
      const data = JSON.parse(fs.readFileSync(this.file, 'utf8'));
      return { listenbrainz: data.listenbrainz || null, lastfm: data.lastfm || null, lastfmApp: data.lastfmApp || null, queue: Array.isArray(data.queue) ? data.queue : [] };
    } catch {
      return { listenbrainz: null, lastfm: null, lastfmApp: null, queue: [] };
    }
  }

  #write() {
    try {
      fs.mkdirSync(path.dirname(this.file), { recursive: true });
      const tmp = `${this.file}.${process.pid}.tmp`;
      fs.writeFileSync(tmp, JSON.stringify(this.data), { mode: 0o600 });
      fs.renameSync(tmp, this.file);
    } catch { /* disque plein ou en lecture seule : la file reste en mémoire */ }
  }

  close() {
    this.closed = true;
    clearTimeout(this.timer);
  }

  // ─── État ──────────────────────────────────────────────────────────────────
  status() {
    const { data } = this;
    const k = this.keys();
    return {
      listenbrainz: { connected: !!data.listenbrainz?.token, user: data.listenbrainz?.user || null, url: data.listenbrainz?.url || null, error: data.listenbrainz?.error || null },
      lastfm: { connected: !!data.lastfm?.session, user: data.lastfm?.user || null, error: data.lastfm?.error || null, configured: !!(k?.key && k?.secret), keysFromServer: !!(this.keysFn?.() || {}).key },
      pending: data.queue.length,
    };
  }

  #changed() {
    this.#write();
    this.emit('scrobble:changed', this.status());
  }

  // ─── ListenBrainz ──────────────────────────────────────────────────────────
  async #lbUrl(path_) {
    const base = this.data.listenbrainz?.url || LB_DEFAULT;
    return `${(await this.guardUrl(base)).replace(/\/$/, '')}${path_}`;
  }

  /** Vérifie un jeton ListenBrainz et l'enregistre. `url` : serveur compatible (facultatif). */
  async connectListenBrainz(token, url) {
    const clean = String(token || '').trim();
    if (!/^[\w-]{8,100}$/.test(clean)) throw new Error('Jeton invalide.');
    const base = url && String(url).trim() ? String(url).trim() : null;
    if (base) await this.guardUrl(base);
    const res = await this.fetch(`${(base || LB_DEFAULT).replace(/\/$/, '')}/1/validate-token`, { headers: { authorization: `Token ${clean}`, 'user-agent': AGENT }, signal: AbortSignal.timeout(15000) });
    const body = await res.json().catch(() => ({}));
    if (!res.ok || !body.valid) throw new Error('Jeton refusé par ListenBrainz.');
    this.data.listenbrainz = { token: clean, user: body.user_name || null, url: base };
    this.#changed();
    this.#schedule(1000);
    return this.status();
  }

  #lbMeta(play) {
    return {
      artist_name: play.artist,
      track_name: play.title,
      ...(play.album ? { release_name: play.album } : {}),
      additional_info: { media_player: 'control', submission_client: 'control', ...(play.duration ? { duration_ms: Math.round(play.duration * 1000) } : {}), ...(play.trackNo ? { tracknumber: play.trackNo } : {}) },
    };
  }

  async #lbSubmit(type, payload) {
    const res = await this.fetch(await this.#lbUrl('/1/submit-listens'), {
      method: 'POST',
      headers: { authorization: `Token ${this.data.listenbrainz.token}`, 'content-type': 'application/json', 'user-agent': AGENT },
      body: JSON.stringify({ listen_type: type, payload }),
      signal: AbortSignal.timeout(20000),
    });
    if (res.status === 401) throw Object.assign(new Error('Jeton ListenBrainz refusé'), { permanent: true });
    if (res.status === 429) throw Object.assign(new Error('ListenBrainz limite les envois'), { retryAfter: Number(res.headers.get('x-ratelimit-reset-in')) || 30 });
    if (res.status >= 400 && res.status < 500) throw Object.assign(new Error(`ListenBrainz : ${res.status}`), { permanent: true });
    if (!res.ok) throw new Error(`ListenBrainz : ${res.status}`);
  }

  // ─── Last.fm ───────────────────────────────────────────────────────────────
  async #lastfm(params, { sign = true } = {}) {
    const k = this.keys();
    if (!k?.key || !k?.secret) throw Object.assign(new Error('Clés Last.fm absentes.'), { permanent: true });
    const all = { ...params, api_key: k.key };
    if (sign) {
      const base = Object.keys(all).sort().map((name) => `${name}${all[name]}`).join('');
      all.api_sig = md5(base + k.secret);
    }
    all.format = 'json';
    const res = await this.fetch(LASTFM, { method: 'POST', headers: { 'content-type': 'application/x-www-form-urlencoded', 'user-agent': AGENT }, body: new URLSearchParams(all), signal: AbortSignal.timeout(20000) });
    const body = await res.json().catch(() => ({}));
    if (body.error) {
      // 9 : session invalide ; 4, 10, 26 : clés ou autorisation refusées ; 11, 16 : service indisponible, à réessayer.
      throw Object.assign(new Error(body.message || `Last.fm : erreur ${body.error}`), { permanent: [4, 9, 10, 26].includes(body.error), code: body.error });
    }
    if (!res.ok) throw new Error(`Last.fm : ${res.status}`);
    return body;
  }

  /** Première étape : un jeton et l'adresse où l'utilisateur autorise control. */
  async lastfmStart() {
    const { token } = await this.#lastfm({ method: 'auth.getToken' });
    return { token, url: `https://www.last.fm/api/auth/?api_key=${encodeURIComponent(this.keys().key)}&token=${encodeURIComponent(token)}` };
  }

  /** Seconde étape, une fois l'autorisation donnée sur le site de Last.fm. */
  async lastfmFinish(token) {
    const { session } = await this.#lastfm({ method: 'auth.getSession', token: String(token || '') });
    this.data.lastfm = { session: session.key, user: session.name };
    this.#changed();
    this.#schedule(1000);
    return this.status();
  }

  disconnect(service) {
    if (service === 'listenbrainz') this.data.listenbrainz = null;
    else if (service === 'lastfm') this.data.lastfm = null;
    this.data.queue = this.data.queue.map((q) => ({ ...q, todo: q.todo.filter((s) => s !== service) })).filter((q) => q.todo.length);
    this.#changed();
    return this.status();
  }

  // ─── Écoutes ───────────────────────────────────────────────────────────────
  #services() {
    return [this.data.listenbrainz?.token && 'listenbrainz', this.data.lastfm?.session && 'lastfm'].filter(Boolean);
  }

  #describe(trackId) {
    const t = this.library.db.prepare('SELECT title, artist, album, album_artist AS albumArtist, track_no AS trackNo, duration, kind FROM tracks WHERE id = ?').get(trackId);
    if (!t || t.kind !== 'audio' || !t.title || !t.artist) return null; // pas de vidéo, pas de morceau sans artiste
    return { ...t };
  }

  /** Un morceau commence : « en écoute » partout (sans attendre ni bloquer la lecture). */
  started(playId, trackId) {
    if (!this.#services().length) return;
    const play = this.#describe(trackId);
    if (!play) return;
    this.plays.set(playId, { play, startedAt: Math.floor(Date.now() / 1000) });
    if (this.plays.size > 50) this.plays.delete(this.plays.keys().next().value);
    this.#nowPlaying(play).catch(() => {});
  }

  async #nowPlaying(play) {
    const jobs = [];
    if (this.data.listenbrainz?.token) jobs.push(this.#lbSubmit('playing_now', [{ track_metadata: this.#lbMeta(play) }]));
    if (this.data.lastfm?.session) {
      jobs.push(this.#lastfm({ method: 'track.updateNowPlaying', artist: play.artist, track: play.title, ...(play.album ? { album: play.album } : {}), ...(play.duration ? { duration: Math.round(play.duration) } : {}), sk: this.data.lastfm.session }));
    }
    await Promise.allSettled(jobs);
  }

  /** Pour l'API Subsonic : un lecteur externe annonce un morceau en cours, ou une écoute terminée à telle date. */
  nowPlaying(trackId) {
    const play = this.#describe(trackId);
    if (play && this.#services().length) this.#nowPlaying(play).catch(() => {});
  }

  submit(trackId, listenedAt) {
    const play = this.#describe(trackId);
    if (play && counts(play.duration, play.duration)) this.enqueue(play, listenedAt);
  }

  /** Un morceau s'arrête : si l'écoute compte, elle est mise en file puis envoyée. */
  ended(playId, seconds) {
    const rec = this.plays.get(playId);
    this.plays.delete(playId);
    if (!rec) return;
    if (!counts(rec.play.duration, Number(seconds) || 0)) return;
    this.enqueue(rec.play, rec.startedAt);
  }

  /** Une écoute à envoyer (aussi utilisée par l'API Subsonic : `scrobble`). */
  enqueue(play, listenedAt) {
    const todo = this.#services();
    if (!todo.length) return;
    this.data.queue.push({ at: listenedAt, play: { title: play.title, artist: play.artist, album: play.album || '', duration: Math.round(play.duration || 0), trackNo: play.trackNo || null }, todo });
    if (this.data.queue.length > MAX_QUEUE) this.data.queue.splice(0, this.data.queue.length - MAX_QUEUE);
    this.#changed();
    this.flush().catch(() => {});
  }

  // ─── Envoi de la file ──────────────────────────────────────────────────────
  #schedule(ms = RETRY_MS) {
    if (this.closed) return;
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.flush().catch(() => {}), ms);
    this.timer.unref?.();
  }

  /** Envoie ce qui attend, service par service ; ce qui échoue sans remède est abandonné, le reste est retenté plus tard. */
  flush() {
    if (this.flushing) return this.flushing;
    this.flushing = (async () => {
      const now = Math.floor(Date.now() / 1000);
      this.data.queue = this.data.queue.filter((q) => now - q.at < MAX_AGE_S);
      let retry = false;
      for (const service of ['listenbrainz', 'lastfm']) {
        const items = this.data.queue.filter((q) => q.todo.includes(service));
        if (!items.length || !this.#services().includes(service)) continue;
        for (let i = 0; i < items.length; i += 50) {
          const batch = items.slice(i, i + 50);
          try {
            // eslint-disable-next-line no-await-in-loop
            await this.#send(service, batch);
            for (const q of batch) q.todo = q.todo.filter((s) => s !== service);
            if (this.data[service]) delete this.data[service].error;
          } catch (err) {
            if (err.permanent) {
              // Jeton ou session refusés : inutile d'insister, l'utilisateur doit se reconnecter.
              if (this.data[service]) this.data[service].error = err.message;
              for (const q of batch) q.todo = q.todo.filter((s) => s !== service);
            } else retry = true;
            break;
          }
        }
      }
      this.data.queue = this.data.queue.filter((q) => q.todo.length);
      this.#changed();
      if (retry) this.#schedule();
    })().finally(() => { this.flushing = null; });
    return this.flushing;
  }

  async #send(service, items) {
    if (service === 'listenbrainz') {
      await this.#lbSubmit(items.length === 1 ? 'single' : 'import', items.map((q) => ({ listened_at: q.at, track_metadata: this.#lbMeta(q.play) })));
      return;
    }
    const params = { method: 'track.scrobble', sk: this.data.lastfm.session };
    items.forEach((q, i) => {
      params[`artist[${i}]`] = q.play.artist;
      params[`track[${i}]`] = q.play.title;
      params[`timestamp[${i}]`] = String(q.at);
      if (q.play.album) params[`album[${i}]`] = q.play.album;
      if (q.play.duration) params[`duration[${i}]`] = String(q.play.duration);
    });
    await this.#lastfm(params);
  }

  /** Au démarrage : reprend l'envoi de ce qui attendait. */
  resume() {
    if (this.data.queue.length && this.#services().length) this.#schedule(5000);
  }
}

module.exports = { Scrobbler, counts };

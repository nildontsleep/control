'use strict';

// Discord Rich Presence : dialogue avec le client Discord installé sur l'ordinateur par son canal local
// (tube nommé sous Windows, socket Unix ailleurs). Aucun compte, aucun jeton : seul l'identifiant public
// de l'application Discord de control est nécessaire.

const net = require('node:net');
const fs = require('node:fs');
const path = require('node:path');
const { EventEmitter } = require('node:events');

// Identifiant de l'application créée sur https://discord.com/developers/applications (public, ce n'est pas un secret).
const APP_ID = process.env.CONTROL_DISCORD_ID || '1563182510202949733';

const ITUNES = process.env.CONTROL_ITUNES_URL || 'https://itunes.apple.com/search';
const OP = { HANDSHAKE: 0, FRAME: 1, CLOSE: 2, PING: 3, PONG: 4 };
const TYPE = { listening: 2, watching: 3 };
const ICONS = new Set(['play', 'pause', 'watching', 'video']);
const MIN_GAP = 3500; // Discord limite à 5 mises à jour par 20 s
const RETRY = 15000;
const HANDSHAKE_TIMEOUT = 6000;
const DEBOUNCE = 120;

const clip = (value, max = 128) => {
  let s = String(value ?? '').replace(/\s+/g, ' ').trim();
  if (!s) return undefined;
  if (s.length > max) s = `${s.slice(0, max - 1)}…`;
  return s.length < 2 ? s.padEnd(2, ' ') : s; // Discord exige au moins 2 caractères
};

const plain = (value, max) => String(value ?? '').replace(/\s+/g, ' ').trim().slice(0, max);

const httpsUrl = (v) => (typeof v === 'string' && /^https:\/\//i.test(v) && v.length <= 256 ? v : null);

/** Normalise ce que la fenêtre envoie : rien de ce qui arrive du rendu n'est transmis tel quel. */
function sanitize(raw) {
  if (!raw || typeof raw !== 'object') return null;
  const kind = raw.kind === 'watching' ? 'watching' : raw.kind === 'listening' ? 'listening' : null;
  const details = clip(raw.details);
  if (!kind || !details) return null;
  const num = (v) => (Number.isFinite(Number(v)) && Number(v) >= 0 ? Number(v) : 0);
  const lookup = raw.lookup && typeof raw.lookup === 'object'
    ? { artist: plain(raw.lookup.artist, 80), album: plain(raw.lookup.album, 80), title: plain(raw.lookup.title, 80) }
    : null;
  return {
    kind,
    details,
    state: clip(raw.state),
    image: httpsUrl(raw.image),
    imageText: clip(raw.imageText),
    icon: ICONS.has(raw.icon) ? raw.icon : null,
    iconText: clip(raw.iconText),
    playing: !!raw.playing,
    position: num(raw.position),
    duration: num(raw.duration),
    lookup: lookup && (lookup.artist || lookup.album || lookup.title) ? lookup : null,
  };
}

/** Les emplacements possibles du canal de Discord (Discord, Canary, PTB, Snap, Flatpak, Vesktop…). */
function pipePaths() {
  const out = [];
  if (process.platform === 'win32') {
    for (let i = 0; i < 10; i++) out.push(`\\\\?\\pipe\\discord-ipc-${i}`);
    return out;
  }
  const base = process.env.CONTROL_DISCORD_IPC_DIR || process.env.XDG_RUNTIME_DIR || process.env.TMPDIR || process.env.TMP || process.env.TEMP || '/tmp';
  const dirs = [base];
  if (!process.env.CONTROL_DISCORD_IPC_DIR) {
    dirs.push(path.join(base, 'snap.discord'));
    for (const app of ['com.discordapp.Discord', 'com.discordapp.DiscordCanary', 'dev.vencord.Vesktop']) dirs.push(path.join(base, 'app', app));
  }
  for (const dir of dirs) for (let i = 0; i < 10; i++) out.push(path.join(dir, `discord-ipc-${i}`));
  return out;
}

function encode(op, data) {
  const body = Buffer.from(JSON.stringify(data));
  const head = Buffer.alloc(8);
  head.writeInt32LE(op, 0);
  head.writeInt32LE(body.length, 4);
  return Buffer.concat([head, body]);
}

/** Découpe le flux en trames (un message peut arriver en plusieurs morceaux, ou plusieurs à la fois). */
function decoder(onFrame) {
  let buf = Buffer.alloc(0);
  return (chunk) => {
    buf = Buffer.concat([buf, chunk]);
    while (buf.length >= 8) {
      const len = buf.readInt32LE(4);
      if (len < 0 || len > 1 << 20) { buf = Buffer.alloc(0); return; }
      if (buf.length < 8 + len) return;
      const op = buf.readInt32LE(0);
      const raw = buf.subarray(8, 8 + len).toString('utf8');
      buf = buf.subarray(8 + len);
      let data = null;
      try { data = JSON.parse(raw); } catch { /* trame illisible */ }
      onFrame(op, data);
    }
  };
}

function tryConnect(file) {
  return new Promise((resolve) => {
    if (process.platform !== 'win32' && !fs.existsSync(file)) { resolve(null); return; }
    const sock = net.createConnection(file);
    const timer = setTimeout(() => { sock.destroy(); resolve(null); }, 1500);
    sock.once('connect', () => { clearTimeout(timer); resolve(sock); });
    sock.once('error', () => { clearTimeout(timer); resolve(null); });
  });
}

class DiscordPresence extends EventEmitter {
  #enabled = false;
  #clientId = APP_ID;
  #override = '';
  #sock = null;
  #ready = false;
  #connecting = false;
  #status = 'off';
  #detail = '';
  #activity = null; // ce que la fenêtre veut afficher (normalisé)
  #cover = null; // pochette trouvée pour l'activité en cours
  #coverKey = '';
  #lastBase = '';
  #lastStart = null;
  #lastSendAt = 0;
  #flushTimer = null;
  #retryTimer = null;
  #nonce = 0;
  #covers = new Map();
  #coverToken = 0;

  get status() {
    return { state: this.#status, detail: this.#detail, clientId: this.#id() };
  }

  #id() {
    return this.#override || this.#clientId;
  }

  #set(state, detail = '') {
    if (state === this.#status && detail === this.#detail) return;
    this.#status = state;
    this.#detail = detail;
    this.emit('status', this.status);
  }

  /** Réglages : activation et identifiant d'application (celui des réglages prime sur celui du programme). */
  configure({ enabled, clientId } = {}) {
    const id = /^\d{15,25}$/.test(String(clientId || '').trim()) ? String(clientId).trim() : '';
    const changed = id !== this.#override || !!enabled !== this.#enabled;
    this.#override = id;
    this.#enabled = !!enabled;
    if (!changed) return;
    this.#drop();
    clearTimeout(this.#retryTimer);
    if (!this.#enabled) { this.#set('off'); return; }
    if (!this.#id()) { this.#set('missing-id'); return; }
    this.#connect();
  }

  /** `null` efface la présence. */
  setActivity(raw) {
    this.#activity = sanitize(raw);
    const lookup = this.#activity?.lookup;
    const key = lookup ? `${lookup.artist}|${lookup.album}|${lookup.title}`.toLowerCase() : '';
    if (key !== this.#coverKey) {
      this.#coverKey = key;
      this.#cover = null;
      this.#coverToken++;
      if (lookup) this.#findCover(lookup, key, this.#coverToken);
    }
    this.#schedule();
  }

  async #findCover(lookup, key, token) {
    if (!this.#covers.has(key)) {
      this.#covers.set(key, null);
      try {
        const term = [lookup.artist, lookup.album || lookup.title].filter(Boolean).join(' ');
        const params = new URLSearchParams({ media: 'music', entity: lookup.album ? 'album' : 'song', limit: '1', term });
        // `net` n'existe qu'une fois l'application prête : on l'importe à l'usage.
        const { net: electronNet } = require('electron');
        const res = await electronNet.fetch(`${ITUNES}?${params}`, { headers: { 'user-agent': 'control (https://github.com/nildontsleep/control)' }, signal: AbortSignal.timeout(6000) });
        if (res.ok) {
          const art = (await res.json()).results?.[0]?.artworkUrl100;
          if (art) this.#covers.set(key, art.replace(/\d+x\d+bb/, '512x512bb'));
        }
      } catch { /* hors ligne : on garde le logo */ }
      if (this.#covers.size > 200) this.#covers.delete(this.#covers.keys().next().value);
    }
    const url = this.#covers.get(key);
    if (url && token === this.#coverToken) {
      this.#cover = url;
      this.#schedule();
    }
  }

  // ─── Connexion ─────────────────────────────────────────────────────────────
  async #connect() {
    if (this.#connecting || this.#sock || !this.#enabled || !this.#id()) return;
    this.#connecting = true;
    this.#set('connecting');
    let sock = null;
    for (const file of pipePaths()) {
      // eslint-disable-next-line no-await-in-loop
      sock = await tryConnect(file);
      if (sock) break;
    }
    this.#connecting = false;
    if (!this.#enabled) { sock?.destroy(); return; }
    if (!sock) {
      this.#set('no-discord');
      this.#retryLater();
      return;
    }
    this.#sock = sock;
    this.#ready = false;
    const timer = setTimeout(() => { if (!this.#ready) { this.#set('error', 'timeout'); this.#drop(); this.#retryLater(); } }, HANDSHAKE_TIMEOUT);
    sock.on('data', decoder((op, data) => this.#frame(sock, op, data, timer)));
    sock.on('close', () => {
      clearTimeout(timer);
      if (this.#sock !== sock) return;
      this.#sock = null;
      this.#ready = false;
      if (this.#enabled && this.#status !== 'bad-id') { this.#set('no-discord'); this.#retryLater(); }
    });
    sock.on('error', () => { /* géré par « close » */ });
    sock.write(encode(OP.HANDSHAKE, { v: 1, client_id: this.#id() }));
  }

  #frame(sock, op, data, timer) {
    if (sock !== this.#sock) return;
    if (op === OP.PING) { sock.write(encode(OP.PONG, data)); return; }
    if (op === OP.CLOSE) {
      clearTimeout(timer);
      const bad = data?.code === 4000 || /client.?id/i.test(String(data?.message || ''));
      this.#set(bad ? 'bad-id' : 'error', String(data?.message || ''));
      this.#drop();
      if (!bad) this.#retryLater();
      return;
    }
    if (!data) return;
    if (data.cmd === 'DISPATCH' && data.evt === 'READY') {
      clearTimeout(timer);
      this.#ready = true;
      this.#lastBase = '';
      this.#set('connected', data.data?.user?.username || '');
      this.#flush();
      return;
    }
    if (data.evt === 'ERROR') this.#set('error', String(data.data?.message || data.data?.code || ''));
  }

  #retryLater() {
    clearTimeout(this.#retryTimer);
    if (!this.#enabled) return;
    this.#retryTimer = setTimeout(() => this.#connect(), RETRY);
    this.#retryTimer.unref?.();
  }

  #drop() {
    clearTimeout(this.#flushTimer);
    const sock = this.#sock;
    this.#sock = null;
    this.#ready = false;
    if (sock) {
      try { sock.write(encode(OP.CLOSE, {})); } catch { /* déjà fermé */ }
      sock.destroy();
    }
  }

  // ─── Envoi ─────────────────────────────────────────────────────────────────
  #build() {
    const a = this.#activity;
    if (!a) return null;
    const now = Date.now();
    const activity = {
      type: TYPE[a.kind],
      details: a.details,
      assets: { large_image: this.#cover || a.image || 'control' },
      instance: false,
      // « Écoute <artiste> » / « Regarde <titre> » dans la liste des membres.
      status_display_type: a.kind === 'listening' ? 1 : 2,
    };
    if (a.state) activity.state = a.state;
    if (a.imageText) activity.assets.large_text = a.imageText;
    if (a.icon) {
      activity.assets.small_image = a.icon;
      if (a.iconText) activity.assets.small_text = a.iconText;
    }
    if (a.playing) {
      const start = Math.round(now - a.position * 1000);
      activity.timestamps = a.duration > a.position ? { start, end: Math.round(start + a.duration * 1000) } : { start };
    }
    return activity;
  }

  #schedule() {
    clearTimeout(this.#flushTimer);
    const wait = Math.max(DEBOUNCE, this.#lastSendAt + MIN_GAP - Date.now());
    this.#flushTimer = setTimeout(() => this.#flush(), wait);
    this.#flushTimer.unref?.();
  }

  #flush() {
    if (!this.#ready || !this.#sock) return;
    const activity = this.#build();
    // Ne rien renvoyer si rien n'a changé : la fenêtre rappelle souvent la même position.
    const { timestamps, ...rest } = activity || {};
    const base = JSON.stringify(activity ? rest : null);
    const start = timestamps?.start ?? null;
    const same = base === this.#lastBase && (start === null) === (this.#lastStart === null)
      && (start === null || Math.abs(start - this.#lastStart) < 3000);
    if (same) return;
    this.#lastBase = base;
    this.#lastStart = start;
    this.#lastSendAt = Date.now();
    this.#sock.write(encode(OP.FRAME, {
      cmd: 'SET_ACTIVITY',
      args: { pid: process.pid, activity: activity || undefined },
      nonce: String(++this.#nonce),
    }));
  }

  /** Essaie de se connecter maintenant et dit ce qui s'est passé (bouton « Tester » des réglages). */
  async test() {
    if (!this.#enabled) return { state: 'off', detail: '', clientId: this.#id() };
    if (!this.#id()) return this.status;
    if (this.#status !== 'connected') {
      clearTimeout(this.#retryTimer);
      await this.#connect();
      await new Promise((r) => { const t0 = Date.now(); const poll = () => (this.#status !== 'connecting' || Date.now() - t0 > HANDSHAKE_TIMEOUT + 500 ? r() : setTimeout(poll, 100)); poll(); });
    }
    return this.status;
  }

  shutdown() {
    this.#enabled = false;
    clearTimeout(this.#retryTimer);
    if (this.#ready && this.#sock) {
      try { this.#sock.write(encode(OP.FRAME, { cmd: 'SET_ACTIVITY', args: { pid: process.pid }, nonce: 'bye' })); } catch { /* fermé */ }
    }
    this.#drop();
  }
}

module.exports = { DiscordPresence, sanitize, encode, decoder, pipePaths, APP_ID };

#!/usr/bin/env node
'use strict';

// Sauvegardes du serveur : la base des comptes (server.db) et, pour chaque compte, sa base (écoutes, playlists, tags,
// progression…) et ses réglages, rangées dans une archive .tgz (chiffrée avec une phrase secrète si l'administrateur
// en a choisi une), découpée en parties assez petites pour un webhook Discord, et envoyée à ce webhook.
// Ne sont jamais copiés : les sessions ouvertes, les jetons et clés (GitHub, TMDB, Last.fm, webhook), les cookies YouTube.
//
// Restauration : voir « Sauvegardes » dans le README. Déchiffrement : `CONTROL_BACKUP_PASSPHRASE=… node server/backup.js decrypt <fichier>`.

const crypto = require('node:crypto');
const fs = require('node:fs');
const fsp = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const zlib = require('node:zlib');
const { promisify } = require('node:util');
const { DatabaseSync } = require('node:sqlite');
const { packRaw } = require('./tar');
const { httpError } = require('./auth');

const gzip = promisify(zlib.gzip);
const WEBHOOK = /^https:\/\/(?:(?:ptb|canary)\.)?discord(?:app)?\.com\/api\/webhooks\/\d{15,25}\/[\w-]{20,120}$/;
const MAGIC = Buffer.from('CTRLBK1\n');
const SECRET_KEYS = ['updateToken', 'tmdbKey', 'backupWebhook', 'backupPassphrase', 'lastfmKey', 'lastfmSecret'];
const KEEP_LOCAL = 2;
const MAX_DB_BYTES = 200 * 1024 * 1024;
const EVERY = { daily: 24 * 3600 * 1000, weekly: 7 * 24 * 3600 * 1000 };

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Chiffre une archive avec une phrase secrète (scrypt + AES-256-GCM) : MAGIC | sel | iv | étiquette | données. */
function encrypt(data, passphrase) {
  const salt = crypto.randomBytes(16);
  const iv = crypto.randomBytes(12);
  const key = crypto.scryptSync(passphrase, salt, 32);
  const cipher = crypto.createCipheriv('aes-256-gcm', key, iv);
  const body = Buffer.concat([cipher.update(data), cipher.final()]);
  return Buffer.concat([MAGIC, salt, iv, cipher.getAuthTag(), body]);
}

function decrypt(blob, passphrase) {
  if (!blob.subarray(0, MAGIC.length).equals(MAGIC)) throw new Error('Ce fichier n’est pas une sauvegarde chiffrée de control.');
  const salt = blob.subarray(MAGIC.length, MAGIC.length + 16);
  const iv = blob.subarray(MAGIC.length + 16, MAGIC.length + 28);
  const tag = blob.subarray(MAGIC.length + 28, MAGIC.length + 44);
  const decipher = crypto.createDecipheriv('aes-256-gcm', crypto.scryptSync(passphrase, salt, 32), iv);
  decipher.setAuthTag(tag);
  try {
    return Buffer.concat([decipher.update(blob.subarray(MAGIC.length + 44)), decipher.final()]);
  } catch {
    throw new Error('Phrase secrète incorrecte, ou fichier abîmé.');
  }
}

/** Les réglages d'un compte sans ce qui est secret (clés d'API, jetons de scrobbling…). */
function scrubSettings(json) {
  let data;
  try {
    data = JSON.parse(json);
  } catch {
    return json;
  }
  const root = data.settings || data;
  if (root.media) delete root.media.tmdbKey;
  if (root.update) delete root.update.token;
  delete root.scrobble;
  return JSON.stringify(data);
}

/** Copie cohérente d'une base SQLite (VACUUM INTO, utilisable pendant que le serveur écrit). */
function snapshot(source, file) {
  fs.rmSync(file, { force: true });
  if (typeof source === 'string') {
    const db = new DatabaseSync(source, { readOnly: true });
    try { db.exec(`VACUUM INTO '${file.replace(/'/g, "''")}'`); } finally { db.close(); }
  } else {
    source.exec(`VACUUM INTO '${file.replace(/'/g, "''")}'`);
  }
}

class Backups {
  /**
   * @param {object} o
   * @param {object} o.hub  le serveur (base, comptes, réglages, comptes chargés)
   * @param {object} [o.options]  { fetch, allowAnyUrl } (essais)
   */
  constructor({ hub, options = {} }) {
    this.hub = hub;
    this.fetch = options.fetch || ((...args) => globalThis.fetch(...args));
    this.allowAnyUrl = !!options.allowAnyUrl;
    this.running = null;
    this.timer = null;
  }

  // ─── Réglages et état ──────────────────────────────────────────────────────
  get settings() {
    const s = this.hub.settings;
    return { webhook: s.backupWebhook || '', every: s.backupEvery || 'off', hour: Number.isInteger(s.backupHour) ? s.backupHour : 3, passphrase: s.backupPassphrase || '' };
  }

  #state() {
    try {
      const row = this.hub.db.prepare("SELECT value FROM kv WHERE key = 'backup_state'").get();
      return row ? JSON.parse(row.value) : {};
    } catch {
      return {};
    }
  }

  #save(state) {
    this.hub.db.prepare("INSERT INTO kv (key, value) VALUES ('backup_state', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value").run(JSON.stringify(state));
  }

  status() {
    const s = this.settings;
    return { configured: !!s.webhook, every: s.every, hour: s.hour, encrypted: !!s.passphrase, running: !!this.running, last: this.#state() };
  }

  validUrl(url) {
    return this.allowAnyUrl ? /^https?:\/\//.test(url) : WEBHOOK.test(url);
  }

  // ─── Contenu ───────────────────────────────────────────────────────────────
  /** L'archive (tar non compressé) : server.db, bibliothèques et réglages des comptes, manifeste. */
  async collect() {
    const { hub } = this;
    const tmp = await fsp.mkdtemp(path.join(os.tmpdir(), 'control-backup-'));
    const files = [];
    const skipped = [];
    try {
      // Base des comptes, sans les sessions ni les secrets.
      const serverCopy = path.join(tmp, 'server.db');
      snapshot(hub.db, serverCopy);
      const copy = new DatabaseSync(serverCopy);
      try {
        copy.exec('DELETE FROM sessions');
        const row = copy.prepare("SELECT value FROM kv WHERE key = 'settings'").get();
        if (row) {
          const settings = JSON.parse(row.value);
          for (const key of SECRET_KEYS) delete settings[key];
          copy.prepare("UPDATE kv SET value = ? WHERE key = 'settings'").run(JSON.stringify(settings));
        }
        copy.exec("DELETE FROM kv WHERE key = 'backup_state'");
        copy.exec('VACUUM');
      } finally {
        copy.close();
      }
      files.push({ name: 'server.db', data: await fsp.readFile(serverCopy) });

      const accounts = hub.accounts.list();
      for (const user of accounts) {
        const dir = path.join(hub.usersDir, user.id);
        const dbFile = path.join(dir, 'library.db');
        if (fs.existsSync(dbFile)) {
          const loaded = hub.entries.get(user.id)?.core?.db;
          const out = path.join(tmp, `${user.id}.db`);
          try {
            snapshot(loaded || dbFile, out);
            const size = (await fsp.stat(out)).size;
            if (size > MAX_DB_BYTES) skipped.push({ id: user.id, username: user.username, reason: `base trop volumineuse (${Math.round(size / 1048576)} Mo)` });
            else files.push({ name: `users/${user.id}/library.db`, data: await fsp.readFile(out) });
          } catch (err) {
            skipped.push({ id: user.id, username: user.username, reason: String(err.message || err) });
          } finally {
            await fsp.rm(out, { force: true });
          }
        }
        const settingsFile = path.join(dir, 'settings.json');
        if (fs.existsSync(settingsFile)) files.push({ name: `users/${user.id}/settings.json`, data: Buffer.from(scrubSettings(await fsp.readFile(settingsFile, 'utf8'))) });
        await sleep(0); // laisse respirer le serveur entre deux comptes
      }
      const manifest = {
        app: 'control',
        version: require('../package.json').version,
        createdAt: new Date().toISOString(),
        accounts: accounts.map((u) => ({ id: u.id, username: u.username })),
        skipped,
        note: 'Sans sessions, jetons, clés d’API ni cookies. Restauration : voir le README (section Sauvegardes).',
      };
      files.unshift({ name: 'MANIFEST.json', data: Buffer.from(JSON.stringify(manifest, null, 2)) });
      return { tar: packRaw(files), manifest };
    } finally {
      await fsp.rm(tmp, { recursive: true, force: true });
    }
  }

  /** Les parties à envoyer : { name, data }[] (une seule si l'archive tient dans une partie). */
  async build({ partBytes = 9 * 1024 * 1024 } = {}) {
    const { tar, manifest } = await this.collect();
    let blob = await gzip(tar, { level: 9 });
    const pass = this.settings.passphrase;
    const stamp = manifest.createdAt.slice(0, 16).replace(/[-:T]/g, '');
    let name = `control-backup-${stamp}.tgz`;
    if (pass) {
      blob = encrypt(blob, pass);
      name += '.enc';
    }
    const total = Math.max(1, Math.ceil(blob.length / partBytes));
    const parts = [];
    for (let i = 0; i < total; i++) {
      parts.push({ name: total === 1 ? name : `${name}.part${String(i + 1).padStart(2, '0')}`, data: blob.subarray(i * partBytes, (i + 1) * partBytes) });
    }
    return { name, parts, size: blob.length, manifest, encrypted: !!pass };
  }

  // ─── Envoi ─────────────────────────────────────────────────────────────────
  async #post(url, content, file) {
    for (let attempt = 0; attempt < 3; attempt++) {
      const form = new FormData();
      form.append('payload_json', JSON.stringify({ content, username: this.hub.settings.serverName || 'control', allowed_mentions: { parse: [] } }));
      if (file) form.append('files[0]', new Blob([file.data]), file.name);
      // eslint-disable-next-line no-await-in-loop
      const res = await this.fetch(`${url}?wait=true`, { method: 'POST', body: form, signal: AbortSignal.timeout(120000) });
      if (res.status === 429) {
        // eslint-disable-next-line no-await-in-loop
        const wait = Number((await res.json().catch(() => ({}))).retry_after) || 2;
        // eslint-disable-next-line no-await-in-loop
        await sleep(Math.min(30, wait) * 1000 + 200);
        continue;
      }
      if (res.status === 404 || res.status === 401) throw httpError(400, 'bad_webhook', 'Webhook refusé ou supprimé (Discord répond « introuvable »).');
      if (!res.ok) throw httpError(502, 'webhook_failed', `Discord a répondu ${res.status}.`);
      return;
    }
    throw httpError(502, 'webhook_failed', 'Discord limite les envois : réessayez plus tard.');
  }

  /** Un message court pour vérifier que le webhook marche. */
  async test() {
    const { webhook } = this.settings;
    if (!webhook) throw httpError(400, 'no_webhook', 'Aucun webhook configuré.');
    await this.#post(webhook, '✅ control : le webhook de sauvegarde fonctionne.');
    return true;
  }

  /** Fait une sauvegarde maintenant et l'envoie. Une seule à la fois. */
  run({ reason = 'manuelle' } = {}) {
    if (this.running) return this.running;
    const { webhook } = this.settings;
    if (!webhook) return Promise.reject(httpError(400, 'no_webhook', 'Aucun webhook configuré.'));
    this.running = (async () => {
      const startedAt = Date.now();
      try {
        const built = await this.build();
        const accounts = built.manifest.accounts.length;
        const head = `💾 **Sauvegarde de ${this.hub.settings.serverName || 'control'}** (${reason}) · ${accounts} compte(s) · ${(built.size / 1048576).toFixed(1)} Mo${built.encrypted ? ' · chiffrée' : ' · **non chiffrée** (elle contient les empreintes des mots de passe)'}${built.parts.length > 1 ? ` · ${built.parts.length} parties` : ''}`;
        for (let i = 0; i < built.parts.length; i++) {
          // eslint-disable-next-line no-await-in-loop
          await this.#post(webhook, i === 0 ? head : `Partie ${i + 1}/${built.parts.length}`, built.parts[i]);
        }
        await this.#keepLocal(built);
        const last = { at: Date.now(), ok: true, size: built.size, parts: built.parts.length, accounts, encrypted: built.encrypted, tookMs: Date.now() - startedAt, skipped: built.manifest.skipped.length };
        this.#save(last);
        return last;
      } catch (err) {
        const last = { at: Date.now(), ok: false, error: String(err.message || err) };
        this.#save(last);
        throw err;
      } finally {
        this.running = null;
      }
    })();
    return this.running;
  }

  /** Garde les deux dernières sauvegardes sur le disque du serveur (en plus de Discord). */
  async #keepLocal(built) {
    const dir = path.join(this.hub.config.dataDir, 'backups');
    await fsp.mkdir(dir, { recursive: true });
    await fsp.writeFile(path.join(dir, built.name), Buffer.concat(built.parts.map((p) => p.data)), { mode: 0o600 });
    const all = (await fsp.readdir(dir)).filter((n) => /^control-backup-.*\.tgz(\.enc)?$/.test(n)).sort();
    for (const old of all.slice(0, Math.max(0, all.length - KEEP_LOCAL))) await fsp.rm(path.join(dir, old), { force: true });
  }

  // ─── Planification ─────────────────────────────────────────────────────────
  /** Une sauvegarde est-elle due ? (à l'heure choisie, une fois par jour ou par semaine ; nouvel essai une heure après un échec) */
  due(now = new Date()) {
    const { webhook, every, hour } = this.settings;
    const period = EVERY[every];
    if (!webhook || !period) return false;
    const last = this.#state();
    if (last.at && last.ok === false && now.getTime() - last.at < 3600 * 1000) return false;
    const slot = new Date(now);
    slot.setHours(hour, 0, 0, 0); // l'heure du jour
    if (now < slot) return false;
    return !last.ok || now.getTime() - last.at >= period - 3600 * 1000;
  }

  start() {
    this.timer = setInterval(() => {
      if (!this.running && this.due()) this.run({ reason: 'planifiée' }).catch((err) => console.error('Sauvegarde :', err.message));
    }, 5 * 60 * 1000);
    this.timer.unref();
  }

  stop() {
    clearInterval(this.timer);
  }
}

if (require.main === module) {
  const [command, file, out] = process.argv.slice(2);
  try {
    if (command === 'decrypt' && file) {
      const pass = process.env.CONTROL_BACKUP_PASSPHRASE;
      if (!pass) throw new Error('Variable CONTROL_BACKUP_PASSPHRASE absente.');
      const target = out || file.replace(/\.enc$/, '');
      fs.writeFileSync(target, decrypt(fs.readFileSync(file), pass));
      console.log(`${target} écrit (archive .tgz).`);
    } else {
      console.log('Utilisation : CONTROL_BACKUP_PASSPHRASE=… node server/backup.js decrypt <sauvegarde.tgz.enc> [sortie.tgz]\nParties découpées : cat control-backup-*.part* > control-backup.tgz.enc');
      process.exit(command ? 1 : 0);
    }
  } catch (err) {
    console.error(err.message);
    process.exit(1);
  }
}

module.exports = { Backups, encrypt, decrypt, scrubSettings, WEBHOOK };

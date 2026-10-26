'use strict';

// Le cœur du serveur : comptes, réglages généraux, et un noyau (réglages, base, stats, playlists…) par compte,
// chargé à la demande et déchargé quand il n'est plus utilisé. Les dossiers de musique, eux, sont communs.

const fs = require('node:fs');
const path = require('node:path');
const { DatabaseSync } = require('node:sqlite');
const platform = require('../src/main/platform');
const { Core } = require('../src/main/core');
const { Tools } = require('../src/main/tools');
const { openServerDb } = require('./serverdb');
const { Accounts, Throttle, httpError } = require('./auth');
const { createRegistry } = require('./rpc');

const IDLE_MS = 30 * 60 * 1000;
const HEARTBEAT_MS = 20 * 1000;

// Les données propres à un compte : on les vide quand on crée sa base à partir de celle d'un autre.
const USER_TABLES = ['plays', 'playlist_tracks', 'playlists', 'watch_progress', 'watchlist', 'watch_time', 'media_cache'];

class Hub {
  constructor(config) {
    this.config = config;
    fs.mkdirSync(config.dataDir, { recursive: true });
    platform.configure({ trashDir: path.join(config.dataDir, 'trash') });
    this.db = openServerDb(path.join(config.dataDir, 'server.db'));
    this.accounts = new Accounts(this.db, { sessionDays: config.sessionDays });
    this.loginThrottle = new Throttle({ max: 8, windowMs: 10 * 60 * 1000 });
    this.ipThrottle = new Throttle({ max: 40, windowMs: 10 * 60 * 1000 });
    this.coversDir = path.join(config.dataDir, 'covers');
    this.usersDir = path.join(config.dataDir, 'users');
    this.entries = new Map();
    this.watchers = [];
    this.timers = [];
    this.tools = new Tools({ binDir: path.join(config.dataDir, 'bin'), tmpDir: path.join(config.dataDir, 'tmp'), emit: (ch, payload) => this.broadcast(ch, payload) });
    this.settings = this.#loadSettings();
  }

  // ─── Réglages du serveur ───────────────────────────────────────────────────
  #loadSettings() {
    const defaults = {
      serverName: 'control',
      registrationEnabled: false,
      allowUserDownloads: false,
      libraryFolders: this.config.libraryFolders,
      downloadDir: path.join(this.config.dataDir, 'downloads'),
      tmdbKey: this.config.tmdbKey,
    };
    let saved = {};
    try {
      const row = this.db.prepare("SELECT value FROM kv WHERE key = 'settings'").get();
      if (row) saved = JSON.parse(row.value);
    } catch { /* réglages illisibles : valeurs par défaut */ }
    const merged = { ...defaults, ...saved };
    if (!Array.isArray(merged.libraryFolders) || (!merged.libraryFolders.length && this.config.libraryFolders.length)) merged.libraryFolders = defaults.libraryFolders;
    return merged;
  }

  patchSettings(patch) {
    const allowed = ['serverName', 'registrationEnabled', 'allowUserDownloads', 'libraryFolders', 'downloadDir', 'tmdbKey'];
    for (const key of allowed) {
      if (!(key in patch)) continue;
      const v = patch[key];
      if (key === 'libraryFolders') this.settings.libraryFolders = (Array.isArray(v) ? v : []).filter((x) => typeof x === 'string' && x);
      else if (key === 'registrationEnabled' || key === 'allowUserDownloads') this.settings[key] = !!v;
      else if (key === 'serverName') this.settings.serverName = String(v || 'control').trim().slice(0, 60) || 'control';
      else this.settings[key] = String(v || '').trim();
    }
    this.db.prepare("INSERT INTO kv (key, value) VALUES ('settings', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value").run(JSON.stringify(this.settings));
    if ('libraryFolders' in patch || 'downloadDir' in patch) this.#syncFolders();
  }

  /** Dossiers analysés : les dossiers de musique, plus celui des téléchargements s'il existe. */
  roots() {
    const roots = [...this.settings.libraryFolders];
    const dl = this.settings.downloadDir;
    if (dl && fs.existsSync(dl) && !roots.some((r) => path.resolve(dl).startsWith(path.resolve(r)))) roots.push(dl);
    return roots;
  }

  /** Le chemin est-il dans un des dossiers de la bibliothèque ? (Garde-fou avant de servir un fichier.) */
  isInsideRoots(file) {
    const resolved = path.resolve(file);
    return this.roots().some((r) => {
      const root = path.resolve(r);
      return resolved === root || resolved.startsWith(root + path.sep);
    });
  }

  #syncFolders() {
    const patch = { libraryFolders: this.settings.libraryFolders, downloadDir: this.settings.downloadDir };
    for (const entry of this.entries.values()) {
      entry.core.store.patch(patch);
      entry.emit('settings:changed', patch);
    }
    this.#watch();
    this.rescanAll();
  }

  async addFolder(dir) {
    const resolved = path.resolve(String(dir || ''));
    let stat;
    try {
      stat = await fs.promises.stat(resolved);
    } catch {
      throw httpError(400, 'not_found', 'Ce dossier n’existe pas sur le serveur.');
    }
    if (!stat.isDirectory()) throw httpError(400, 'not_a_folder', 'Ce n’est pas un dossier.');
    if (path.parse(resolved).root === resolved) throw httpError(400, 'too_broad', 'Choisissez un dossier plutôt que la racine du disque.');
    if (this.settings.libraryFolders.includes(resolved)) return this.settings.libraryFolders;
    this.patchSettings({ libraryFolders: [...this.settings.libraryFolders, resolved] });
    return this.settings.libraryFolders;
  }

  removeFolder(dir) {
    this.patchSettings({ libraryFolders: this.settings.libraryFolders.filter((f) => f !== dir) });
    return this.settings.libraryFolders;
  }

  /** Sous-dossiers d'un dossier du serveur, pour le sélecteur de l'administrateur. */
  async browse(dir) {
    const target = path.resolve(String(dir || path.parse(process.cwd()).root));
    let entries;
    try {
      entries = await fs.promises.readdir(target, { withFileTypes: true });
    } catch {
      throw httpError(400, 'unreadable', 'Dossier illisible.');
    }
    const dirs = entries.filter((e) => e.isDirectory() && !e.name.startsWith('.')).map((e) => e.name).sort((a, b) => a.localeCompare(b));
    const parent = path.dirname(target);
    return { path: target, parent: parent === target ? null : parent, dirs };
  }

  // ─── Noyaux par compte ─────────────────────────────────────────────────────
  /** Crée la base d'un nouveau compte à partir de celle d'un autre (la musique est déjà analysée), sans ses écoutes. */
  #provision(dir) {
    const dest = path.join(dir, 'library.db');
    if (fs.existsSync(dest)) return;
    fs.mkdirSync(dir, { recursive: true });
    const loaded = [...this.entries.values()][0]?.core.db;
    let source = loaded;
    let opened = null;
    if (!source) {
      const files = fs.existsSync(this.usersDir)
        ? fs.readdirSync(this.usersDir).map((u) => path.join(this.usersDir, u, 'library.db')).filter((f) => fs.existsSync(f))
          .sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs)
        : [];
      if (!files.length) return;
      opened = new DatabaseSync(files[0]);
      source = opened;
    }
    try {
      source.prepare('VACUUM INTO ?').run(dest);
      const copy = new DatabaseSync(dest);
      try {
        copy.exec(USER_TABLES.map((t) => `DELETE FROM ${t};`).join('\n'));
        copy.exec('UPDATE tracks SET play_count = 0, last_played = NULL;');
      } finally {
        copy.close();
      }
    } catch (err) {
      console.error('Copie de la bibliothèque impossible, nouvelle analyse :', err.message);
      try { fs.rmSync(dest, { force: true }); } catch { /* rien */ }
    } finally {
      opened?.close();
    }
  }

  /** Le noyau d'un compte (créé au besoin). */
  entry(user) {
    let entry = this.entries.get(user.id);
    if (entry) {
      entry.lastUsed = Date.now();
      return entry;
    }
    const dir = path.join(this.usersDir, user.id);
    this.#provision(dir);
    entry = { id: user.id, clients: new Map(), lastUsed: Date.now(), registry: null, core: null };
    entry.emit = (channel, payload, exceptClient) => {
      const frame = `event: ${channel}\ndata: ${JSON.stringify(payload ?? null)}\n\n`;
      for (const [clientId, res] of entry.clients) {
        if (clientId !== exceptClient) res.write(frame);
      }
    };
    entry.core = new Core({
      dir,
      coversDir: this.coversDir,
      tools: this.tools,
      emit: entry.emit,
      defaultDownloadDir: this.settings.downloadDir,
      // Le service films/séries reçoit la clé TMDB du serveur sans que le compte la voie dans ses réglages.
      mediaStore: {
        get: (key, fallback) => {
          const value = entry.core.store.get(key, fallback);
          if (key === 'media' && this.settings.tmdbKey && !String(value?.tmdbKey || '').trim()) return { ...(value || {}), tmdbKey: this.settings.tmdbKey };
          return value;
        },
        patch: (p) => entry.core.store.patch(p),
      },
    });
    entry.registry = createRegistry(this, entry);
    this.entries.set(user.id, entry);
    entry.core.store.patch({ libraryFolders: this.settings.libraryFolders, downloadDir: this.settings.downloadDir });
    this.#refresh(entry, this.roots());
    return entry;
  }

  /** Envoie un événement à tous les comptes connectés (outils installés, par exemple). */
  broadcast(channel, payload) {
    for (const entry of this.entries.values()) entry.emit(channel, payload);
  }

  addClient(entry, clientId, res) {
    entry.clients.set(clientId, res);
    entry.lastUsed = Date.now();
  }

  removeClient(entry, clientId, res) {
    if (entry.clients.get(clientId) === res) entry.clients.delete(clientId);
    entry.lastUsed = Date.now();
  }

  /** Coupe ce qui est ouvert pour un compte (désactivé, supprimé) : connexions en direct et noyau. */
  dropUser(id) {
    const entry = this.entries.get(id);
    if (!entry) return;
    for (const res of entry.clients.values()) res.end();
    entry.clients.clear();
    this.#unload(entry);
  }

  #unload(entry) {
    this.entries.delete(entry.id);
    try { entry.core.close(); } catch { /* déjà fermé */ }
  }

  #reap() {
    const now = Date.now();
    for (const entry of [...this.entries.values()]) {
      if (entry.clients.size || now - entry.lastUsed < IDLE_MS) continue;
      const busy = entry.core.downloads.list().some((j) => ['queued', 'downloading', 'processing'].includes(j.status));
      if (!busy) this.#unload(entry);
    }
  }

  /** Efface le dossier d'un compte supprimé. */
  forgetUser(id) {
    this.dropUser(id);
    fs.rmSync(path.join(this.usersDir, id), { recursive: true, force: true });
  }

  // ─── Analyse des dossiers ──────────────────────────────────────────────────
  /** Retire de la bibliothèque d'un compte les morceaux qui ne sont plus dans un dossier du serveur (les fichiers restent). */
  async #prune(entry, roots) {
    const key = (p) => (process.platform === 'win32' ? p.toLowerCase() : p);
    const prefixes = roots.map((r) => key(path.resolve(r)) + path.sep);
    const library = entry.core.library;
    const ids = library.tracks().filter((t) => !prefixes.some((p) => key(path.resolve(t.path)).startsWith(p))).map((t) => t.id);
    if (ids.length) await library.removeTracks(ids);
  }

  async #refresh(entry, roots) {
    try {
      await this.#prune(entry, roots);
      await entry.core.library.scan(roots);
    } catch (err) {
      console.error('Analyse de la bibliothèque :', err.message);
    }
  }

  rescanAll() {
    const roots = this.roots();
    for (const entry of this.entries.values()) this.#refresh(entry, roots);
  }

  #watch() {
    for (const w of this.watchers) w.close();
    this.watchers = [];
    const schedule = () => {
      clearTimeout(this.watchTimer);
      this.watchTimer = setTimeout(() => this.rescanAll(), 5000);
    };
    for (const root of this.roots()) {
      try {
        const watcher = fs.watch(root, { recursive: true }, (event, name) => {
          if (name && /\.(part|ytdl|tmp|temp)$/i.test(String(name))) return;
          schedule();
        });
        watcher.on('error', () => {});
        this.watchers.push(watcher);
      } catch { /* dossier absent ou surveillance impossible : l'analyse périodique prendra le relais */ }
    }
  }

  start() {
    this.#watch();
    this.timers.push(setInterval(() => this.#reap(), 60 * 1000));
    this.timers.push(setInterval(() => this.rescanAll(), this.config.rescanMinutes * 60 * 1000));
    this.timers.push(setInterval(() => this.accounts.purge(), 6 * 3600 * 1000));
    this.timers.push(setInterval(() => {
      for (const entry of this.entries.values()) for (const res of entry.clients.values()) res.write(': ping\n\n');
    }, HEARTBEAT_MS));
    this.tools.refreshVersions().then(() => this.broadcast('tools:status', this.tools.status())).catch(() => {});
    this.accounts.purge();
  }

  close() {
    for (const t of this.timers) clearInterval(t);
    clearTimeout(this.watchTimer);
    for (const w of this.watchers) w.close();
    for (const entry of [...this.entries.values()]) {
      for (const res of entry.clients.values()) res.end();
      this.#unload(entry);
    }
    this.db.close();
  }
}

module.exports = { Hub };

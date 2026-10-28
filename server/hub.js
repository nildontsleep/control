'use strict';

// Le cœur du serveur : comptes, réglages généraux, et un noyau (réglages, base, stats, playlists…) par compte,
// chargé à la demande et déchargé quand il n'est plus utilisé. Chaque compte a sa propre bibliothèque : ses dossiers
// de musique (attribués par l'administrateur), plus les éventuels dossiers communs à tous.

const fs = require('node:fs');
const path = require('node:path');
const platform = require('../src/main/platform');
const { Core } = require('../src/main/core');
const { Tools } = require('../src/main/tools');
const { openServerDb } = require('./serverdb');
const { Accounts, Throttle, httpError } = require('./auth');
const { Social } = require('./social');
const { createRegistry } = require('./rpc');
const { WebUpdater } = require('./updater');
const pkg = require('../package.json');

const IDLE_MS = 30 * 60 * 1000;
const HEARTBEAT_MS = 20 * 1000;

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
    this.avatarsDir = path.join(config.dataDir, 'avatars');
    this.usersDir = path.join(config.dataDir, 'users');
    this.social = new Social(this.db, this.accounts, { avatarsDir: this.avatarsDir });
    this.entries = new Map();
    this.watchers = [];
    this.timers = [];
    this.tools = new Tools({ binDir: path.join(config.dataDir, 'bin'), tmpDir: path.join(config.dataDir, 'tmp'), emit: (ch, payload) => this.broadcast(ch, payload) });
    this.settings = this.#loadSettings();
    // Mise à jour automatique : ne peut redémarrer le serveur que s'il a été lancé par server/launcher.js.
    this.updater = new WebUpdater({
      version: pkg.version,
      dataDir: config.dataDir,
      settings: () => this.updateConfig(),
      supervised: process.env.CONTROL_SUPERVISED === '1' && typeof process.send === 'function',
      restart: () => process.send({ type: 'restart' }),
      backup: () => WebUpdater.backupDb(this.db, config.dataDir, pkg.version),
      idle: () => this.connectedClients() === 0,
      ...(config.updateOptions || {}),
    });
    this.updater.on('state', (state) => this.toAdmins('update:state', state));
  }

  // ─── Réglages du serveur ───────────────────────────────────────────────────
  #loadSettings() {
    const defaults = {
      serverName: 'control',
      registrationEnabled: false,
      allowUserDownloads: false,
      libraryFolders: [], // dossiers communs : tous les comptes les voient (les dossiers propres à un compte sont dans la table users)
      usersRoot: '', // si défini, chaque nouveau compte reçoit un dossier de musique à son nom dedans
      downloadDir: path.join(this.config.dataDir, 'downloads'), // un sous-dossier par compte

      tmdbKey: this.config.tmdbKey,
      updateMode: 'notify', // mise à jour du serveur : 'off', 'notify' (on prévient) ou 'auto'
      updateHour: 4, // heure (0-23) à laquelle une mise à jour automatique s'installe même si des comptes sont connectés
      updateToken: '', // jeton GitHub en lecture (dépôt privé)
    };
    let saved = {};
    try {
      const row = this.db.prepare("SELECT value FROM kv WHERE key = 'settings'").get();
      if (row) saved = JSON.parse(row.value);
    } catch { /* réglages illisibles : valeurs par défaut */ }
    const merged = { ...defaults, ...saved };
    if (!Array.isArray(merged.libraryFolders)) merged.libraryFolders = [];
    return merged;
  }

  patchSettings(patch) {
    const allowed = ['serverName', 'registrationEnabled', 'allowUserDownloads', 'libraryFolders', 'downloadDir', 'tmdbKey', 'usersRoot', 'updateMode', 'updateHour', 'updateToken'];
    for (const key of allowed) {
      if (!(key in patch)) continue;
      const v = patch[key];
      if (key === 'libraryFolders') this.settings.libraryFolders = (Array.isArray(v) ? v : []).filter((x) => typeof x === 'string' && x);
      else if (key === 'registrationEnabled' || key === 'allowUserDownloads') this.settings[key] = !!v;
      else if (key === 'serverName') this.settings.serverName = String(v || 'control').trim().slice(0, 60) || 'control';
      else if (key === 'updateMode') this.settings.updateMode = ['off', 'notify', 'auto'].includes(v) ? v : 'notify';
      else if (key === 'updateHour') this.settings.updateHour = Math.min(23, Math.max(0, Math.floor(Number(v)) || 0));
      else this.settings[key] = String(v || '').trim();
    }
    this.db.prepare("INSERT INTO kv (key, value) VALUES ('settings', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value").run(JSON.stringify(this.settings));
    if ('libraryFolders' in patch || 'downloadDir' in patch) this.#syncFolders();
    if (['updateMode', 'updateHour', 'updateToken'].some((k) => k in patch)) this.updater.refresh();
  }

  /** Les réglages tels que la page d'administration les voit (le jeton GitHub ne quitte jamais le serveur). */
  publicSettings() {
    const { updateToken, ...rest } = this.settings;
    return { ...rest, hasUpdateToken: !!(updateToken || this.config.updateToken), updateMode: this.config.update || rest.updateMode, updateLocked: !!this.config.update };
  }

  updateConfig() {
    return { mode: this.config.update || this.settings.updateMode, hour: this.settings.updateHour, token: this.config.updateToken || this.settings.updateToken, locked: !!this.config.update };
  }

  // ─── Dossiers de musique ───────────────────────────────────────────────────
  /** Dossiers communs : tous les comptes les voient dans leur bibliothèque. */
  commonFolders() {
    return [...this.settings.libraryFolders];
  }

  /** Dossiers propres à un compte (attribués par l'administrateur). */
  ownFolders(userId) {
    try {
      const list = JSON.parse(this.accounts.byId(userId)?.folders || '[]');
      return Array.isArray(list) ? list.filter((f) => typeof f === 'string' && f) : [];
    } catch {
      return [];
    }
  }

  /** Dossier des téléchargements d'un compte : un sous-dossier à son nom. */
  downloadDirFor(user) {
    const row = typeof user === 'string' ? this.accounts.byId(user) : user;
    return path.join(this.settings.downloadDir, row?.username || String(user));
  }

  /** Dossiers analysés pour la bibliothèque d'un compte : les siens, ses téléchargements, et les dossiers communs. */
  rootsFor(user) {
    const row = typeof user === 'string' ? this.accounts.byId(user) : user;
    if (!row) return [];
    const roots = [...this.commonFolders(), ...this.ownFolders(row.id)];
    const dl = this.downloadDirFor(row);
    if (fs.existsSync(dl) && !roots.some((r) => path.resolve(dl).startsWith(path.resolve(r)))) roots.push(dl);
    return roots;
  }

  /** Dossiers qui n'appartiennent qu'à ce compte : ce que les autres voient de sa bibliothèque (sans les dossiers communs). */
  ownerRoots(user) {
    const row = typeof user === 'string' ? this.accounts.byId(user) : user;
    if (!row) return [];
    const roots = [...this.ownFolders(row.id)];
    const dl = this.downloadDirFor(row);
    if (fs.existsSync(dl)) roots.push(dl);
    return roots;
  }

  /** Le fichier est-il dans un des dossiers de cette liste ? (Garde-fou avant de servir un fichier.) */
  static inside(roots, file) {
    const key = (p) => (process.platform === 'win32' ? p.toLowerCase() : p);
    const resolved = key(path.resolve(file));
    return roots.some((r) => {
      const root = key(path.resolve(r));
      return resolved === root || resolved.startsWith(root + path.sep);
    });
  }

  isInsideRoots(user, file) {
    return Hub.inside(this.rootsFor(user), file);
  }

  /** Chemin relatif au dossier qui le contient (précédé du nom de ce dossier) : ce que voient les autres comptes. */
  static relative(roots, file) {
    const resolved = path.resolve(file);
    for (const r of roots) {
      const root = path.resolve(r);
      if (resolved === root || resolved.startsWith(root + path.sep)) return path.join(path.basename(root), path.relative(root, resolved));
    }
    return path.basename(file);
  }

  /** Reporte les dossiers dans les réglages de chaque compte chargé (pour l'affichage) ; la source reste le serveur. */
  mirror(entry) {
    const patch = { libraryFolders: this.ownFolders(entry.id), commonFolders: this.commonFolders(), downloadDir: this.downloadDirFor(entry.id) };
    entry.core.store.patch(patch);
    entry.emit('settings:changed', patch);
  }

  #syncFolders() {
    for (const entry of this.entries.values()) {
      entry.core.downloads.defaultDir = this.downloadDirFor(entry.id);
      this.mirror(entry);
    }
    this.#watch();
    this.rescanAll();
  }

  static async checkFolder(dir) {
    const resolved = path.resolve(String(dir || ''));
    let stat;
    try {
      stat = await fs.promises.stat(resolved);
    } catch {
      throw httpError(400, 'not_found', 'Ce dossier n’existe pas sur le serveur.');
    }
    if (!stat.isDirectory()) throw httpError(400, 'not_a_folder', 'Ce n’est pas un dossier.');
    if (path.parse(resolved).root === resolved) throw httpError(400, 'too_broad', 'Choisissez un dossier plutôt que la racine du disque.');
    return resolved;
  }

  async addFolder(dir) {
    const resolved = await Hub.checkFolder(dir);
    if (this.settings.libraryFolders.includes(resolved)) return this.settings.libraryFolders;
    this.patchSettings({ libraryFolders: [...this.settings.libraryFolders, resolved] });
    return this.settings.libraryFolders;
  }

  removeFolder(dir) {
    this.patchSettings({ libraryFolders: this.settings.libraryFolders.filter((f) => f !== dir) });
    return this.settings.libraryFolders;
  }

  #saveOwnFolders(userId, list) {
    this.db.prepare('UPDATE users SET folders = ? WHERE id = ?').run(JSON.stringify(list), userId);
    const entry = this.entries.get(userId);
    if (entry) {
      this.mirror(entry);
      this.#refresh(entry, this.rootsFor(userId));
    }
    this.#watch();
    return list;
  }

  async addOwnFolder(userId, dir) {
    if (!this.accounts.byId(userId)) throw httpError(404, 'not_found', 'Compte introuvable.');
    const resolved = await Hub.checkFolder(dir);
    const list = this.ownFolders(userId);
    return list.includes(resolved) ? list : this.#saveOwnFolders(userId, [...list, resolved]);
  }

  removeOwnFolder(userId, dir) {
    return this.#saveOwnFolders(userId, this.ownFolders(userId).filter((f) => f !== dir));
  }

  /** Crée le dossier de musique d'un compte dans le dossier des comptes (si l'administrateur en a défini un). */
  async ensurePersonalFolder(user) {
    const root = this.settings.usersRoot;
    if (!root) throw httpError(400, 'no_users_root', 'Aucun dossier des comptes n’est défini dans les réglages du serveur.');
    const dir = path.join(path.resolve(root), user.username);
    try {
      await fs.promises.mkdir(dir, { recursive: true });
    } catch (err) {
      throw httpError(400, 'mkdir_failed', `Impossible de créer ${dir} : ${err.code || err.message}`);
    }
    return this.addOwnFolder(user.id, dir);
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
  /** Le noyau d'un compte (créé au besoin). */
  entry(user) {
    let entry = this.entries.get(user.id);
    if (entry) {
      entry.lastUsed = Date.now();
      return entry;
    }
    const dir = path.join(this.usersDir, user.id);
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
      defaultDownloadDir: this.downloadDirFor(user),
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
    this.mirror(entry);
    this.#refresh(entry, this.rootsFor(user));
    return entry;
  }

  /** Envoie un événement à tous les comptes connectés (outils installés, par exemple). */
  broadcast(channel, payload) {
    for (const entry of this.entries.values()) entry.emit(channel, payload);
  }

  /** … ou aux seuls administrateurs (mises à jour du serveur). */
  toAdmins(channel, payload) {
    for (const entry of this.entries.values()) if (this.accounts.byId(entry.id)?.role === 'admin') entry.emit(channel, payload);
  }

  /** Nombre de pages ouvertes (flux d'événements) : zéro, c'est le bon moment pour redémarrer. */
  connectedClients() {
    let n = 0;
    for (const entry of this.entries.values()) n += entry.clients.size;
    return n;
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
    this.social.forget(id);
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
    for (const entry of this.entries.values()) this.#refresh(entry, this.rootsFor(entry.id));
  }

  /** Tous les dossiers surveillés : communs, propres à un compte, téléchargements. */
  #allRoots() {
    const roots = new Set(this.commonFolders());
    for (const row of this.db.prepare("SELECT id, username, folders FROM users WHERE disabled = 0").all()) {
      try {
        for (const f of JSON.parse(row.folders || '[]')) roots.add(f);
      } catch { /* liste illisible */ }
      const dl = this.downloadDirFor(row);
      if (fs.existsSync(dl)) roots.add(dl);
    }
    return [...roots];
  }

  #watch() {
    for (const w of this.watchers) w.close();
    this.watchers = [];
    const schedule = () => {
      clearTimeout(this.watchTimer);
      this.watchTimer = setTimeout(() => this.rescanAll(), 5000);
    };
    for (const root of this.#allRoots()) {
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
    this.updater.start();
  }

  close() {
    this.updater.stop();
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

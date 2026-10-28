'use strict';

// Mise à jour automatique du serveur web. Elle cherche la dernière release GitHub, télécharge l'archive du serveur
// (`control-web-<version>.tgz`, jointe à chaque release), vérifie son empreinte SHA-256, l'installe à côté de la
// version en cours (`<données>/app/versions/<version>`), puis demande au lanceur (server/launcher.js) de redémarrer
// le serveur sur cette version. Le lanceur revient à la précédente si la nouvelle ne démarre pas.
//
// Trois modes (réglage de l'administrateur) : « off » (rien), « notify » (on prévient, l'administrateur installe),
// « auto » (téléchargement automatique, installation dès que plus personne n'est connecté, ou à l'heure choisie).

const fs = require('node:fs');
const fsp = require('node:fs/promises');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawn } = require('node:child_process');
const { EventEmitter } = require('node:events');
const { unpack } = require('./tar');
const appstate = require('./appstate');

const REPO = 'nildontsleep/control';
const API = () => process.env.CONTROL_UPDATE_URL || `https://api.github.com/repos/${REPO}/releases/latest`;
const TRUSTED = [
  new RegExp(`^https://github\\.com/${REPO}/releases/download/`),
  new RegExp(`^https://api\\.github\\.com/repos/${REPO}/releases/assets/\\d+$`),
];
const AGENT = 'control-web-updater (https://github.com/nildontsleep/control)';
const ASSET = /^control-web-(\d+\.\d+\.\d+)\.tgz$/;
const MAX_ARCHIVE = 100 * 1024 * 1024;
const CHECK_EVERY_MS = 6 * 60 * 60 * 1000;
const KEEP_BACKUPS = 3;

const sha256 = (buf) => crypto.createHash('sha256').update(buf).digest('hex');

/** « >=22.13 » (la seule forme employée) satisfaite par la version de Node en cours ? */
function nodeSatisfies(range, version = process.versions.node) {
  const m = /^>=\s*(\d+)(?:\.(\d+))?/.exec(String(range || ''));
  if (!m) return true;
  const [maj, min] = version.split('.').map(Number);
  return maj > Number(m[1]) || (maj === Number(m[1]) && min >= Number(m[2] || 0));
}

class WebUpdater extends EventEmitter {
  /**
   * @param {object} o
   * @param {string} o.version  version en cours
   * @param {string} o.dataDir  dossier de données du serveur
   * @param {Function} o.settings  () → { mode: 'off'|'notify'|'auto', hour: 0-23, token: string, locked: boolean }
   * @param {boolean} o.supervised  lancé par server/launcher.js (sinon on ne peut que prévenir)
   * @param {Function} o.restart  demande au lanceur de redémarrer le serveur
   * @param {Function} [o.backup]  () → copie de la base du serveur avant de changer de version
   * @param {Function} [o.idle]  () → plus personne n'est connecté ?
   * @param {Function} [o.fetch]  équivalent de fetch
   */
  constructor({ version, dataDir, settings, supervised, restart, backup, idle, fetch: fetchFn }) {
    super();
    this.version = version;
    this.dataDir = dataDir;
    this.settings = settings;
    this.supervised = !!supervised;
    this.restartServer = restart;
    this.backup = backup || (() => null);
    this.idle = idle || (() => false);
    this.fetch = fetchFn || ((...args) => globalThis.fetch(...args));
    this.paths = appstate.paths(dataDir);
    this.release = null;
    this.state = { phase: 'idle' };
    this.busy = false;
    this.timers = [];
    this.baseRoot = process.env.CONTROL_BASE_ROOT || path.resolve(__dirname, '..');
    this.#set({ phase: 'idle' });
  }

  #set(patch) {
    const { mode, hour, locked } = this.#config();
    this.state = {
      current: this.version,
      mode,
      hour,
      locked,
      supervised: this.supervised,
      failure: appstate.readState(this.dataDir).failure || null,
      ...patch,
    };
    this.emit('state', this.state);
  }

  get status() {
    return this.state;
  }

  /** Réglages en cours (ils peuvent changer à tout moment : on les relit). */
  #config() {
    const c = { mode: 'notify', hour: 4, token: '', locked: false, ...this.settings() };
    return { ...c, hour: Math.min(23, Math.max(0, Math.floor(Number(c.hour)) || 0)) };
  }

  /** Les réglages ont changé : l'état montré aux clients suit. */
  refresh() {
    const { current, mode, hour, locked, supervised, failure, ...rest } = this.state; // eslint-disable-line no-unused-vars
    this.#set(rest);
  }

  #auth() {
    const token = String(this.#config().token || '').trim();
    return token ? { authorization: `Bearer ${token}` } : {};
  }

  async #api() {
    const res = await this.fetch(API(), { headers: { 'user-agent': AGENT, accept: 'application/vnd.github+json', ...this.#auth() }, signal: AbortSignal.timeout(12000) });
    if (!res.ok) {
      const authed = !!this.#auth().authorization;
      // Sans jeton, GitHub répond 404 pour un dépôt privé ; avec un jeton, 401/404 veulent dire qu'il est refusé.
      const code = res.status === 404 ? (authed ? 'token' : 'private')
        : res.status === 401 ? 'token'
          : res.status === 403 || res.status === 429 ? (authed ? 'token' : 'busy') : 'http';
      throw Object.assign(new Error(`HTTP ${res.status}`), { code });
    }
    return res.json();
  }

  /** Lit la redirection d'une adresse sans la suivre (le jeton ne doit jamais partir vers l'adresse de destination). */
  async #redirect(url, headers) {
    const res = await this.fetch(url, { headers, redirect: 'manual', signal: AbortSignal.timeout(12000) });
    return res.status >= 300 && res.status < 400 ? res.headers.get('location') : null;
  }

  /**
   * Cherche une nouvelle version. `manual` : demandé par l'administrateur (l'échec est signalé, et une version déjà
   * abandonnée reste proposée) ; sinon c'est la vérification périodique, silencieuse.
   */
  async check({ manual = false } = {}) {
    if (this.busy || ['downloading', 'installing'].includes(this.state.phase)) return this.state;
    if (!manual && (this.#config().mode === 'off' || this.state.phase === 'ready')) return this.state; // une version prête n'est pas écartée par une vérification périodique
    this.busy = true;
    if (manual) this.#set({ phase: 'checking' });
    try {
      const rel = await this.#api();
      const latest = String(rel.tag_name || '').replace(/^v/i, '');
      if (!appstate.VERSION.test(latest) || rel.draft || rel.prerelease || appstate.compareVersions(latest, this.version) <= 0) {
        this.release = null;
        this.#set({ phase: 'none', checkedAt: Date.now() });
        return this.state;
      }
      if (!manual && appstate.readState(this.dataDir).bad.includes(latest)) {
        // Cette version a déjà échoué ici : on attend la suivante, ou que l'administrateur la redemande.
        this.release = null;
        this.#set({ phase: 'none', checkedAt: Date.now(), skipped: latest });
        return this.state;
      }
      const info = { version: latest, name: rel.name || `v${latest}`, notes: String(rel.body || '').slice(0, 4000), page: rel.html_url, checkedAt: Date.now() };
      const asset = (rel.assets || []).find((a) => ASSET.exec(a.name)?.[1] === latest && a.browser_download_url);
      const digest = /^sha256:([0-9a-f]{64})$/i.exec(asset?.digest || '')?.[1]?.toLowerCase() || null;
      // Rien à installer tout seul : on signale la version et on renvoie vers sa page.
      const reason = !this.supervised ? 'unsupervised' : !asset ? 'asset' : !digest ? 'nodigest' : null;
      if (reason) {
        this.release = null;
        this.#set({ phase: 'available', manual: true, reason, ...info });
        return this.state;
      }
      // Dépôt privé : le téléchargement passe par l'API (avec le jeton) ; sinon par l'adresse publique.
      const authed = !!this.#auth().authorization;
      const url = authed && asset.url ? asset.url : asset.browser_download_url;
      if (!TRUSTED.some((re) => re.test(url)) && !process.env.CONTROL_UPDATE_URL) throw Object.assign(new Error('untrusted'), { code: 'untrusted' });
      this.release = { ...info, url, size: asset.size || 0, digest, api: url === asset.url };
      this.#set({ phase: 'available', size: this.release.size, ...info });
    } catch (err) {
      if (manual) this.#set({ phase: 'error', code: err.code || 'network', checkedAt: Date.now() });
      else if (this.state.phase === 'checking') this.#set({ phase: 'idle' });
    } finally {
      this.busy = false;
    }
    return this.state;
  }

  /** Télécharge l'archive, vérifie son empreinte, puis l'installe à côté de la version en cours (sans la lancer). */
  async download() {
    const rel = this.release;
    if (!rel || this.busy || ['downloading', 'installing', 'ready'].includes(this.state.phase)) return this.state;
    this.busy = true;
    const info = { version: rel.version, name: rel.name, notes: rel.notes, page: rel.page };
    this.#set({ phase: 'downloading', size: rel.size, received: 0, ...info });
    try {
      let url = rel.url;
      let headers = { 'user-agent': AGENT };
      if (rel.api) {
        const target = await this.#redirect(rel.url, { 'user-agent': AGENT, accept: 'application/octet-stream', ...this.#auth() });
        if (target) {
          const host = (() => { try { const u = new URL(target); return u.protocol === 'https:' ? u.hostname : ''; } catch { return ''; } })();
          if (!/(^|\.)(github|githubusercontent)\.com$/i.test(host) && !process.env.CONTROL_UPDATE_URL) throw Object.assign(new Error('untrusted'), { code: 'untrusted' });
          url = target;
        } else {
          headers = { 'user-agent': AGENT, accept: 'application/octet-stream', ...this.#auth() };
        }
      }
      const res = await this.fetch(url, { headers, redirect: 'follow' });
      if (!res.ok || !res.body) throw Object.assign(new Error(`HTTP ${res.status}`), { code: 'http' });
      const total = Number(res.headers.get('content-length')) || rel.size;
      const chunks = [];
      let received = 0;
      let last = 0;
      const reader = res.body.getReader();
      for (;;) {
        // eslint-disable-next-line no-await-in-loop
        const { done, value } = await reader.read();
        if (done) break;
        chunks.push(value);
        received += value.length;
        if (received > MAX_ARCHIVE) throw Object.assign(new Error('taille'), { code: 'corrupt' });
        if (Date.now() - last > 200) {
          last = Date.now();
          this.#set({ phase: 'downloading', size: total, received, ...info });
        }
      }
      const archive = Buffer.concat(chunks);
      if (rel.size && archive.length !== rel.size) throw Object.assign(new Error('taille'), { code: 'corrupt' });
      if (sha256(archive) !== rel.digest) throw Object.assign(new Error('empreinte'), { code: 'corrupt' });
      await this.#stage(archive, rel.version);
      this.#set({ phase: 'ready', size: total, verified: true, ...info });
    } catch (err) {
      this.#set({ phase: 'error', code: err.code || 'network', detail: err.detail || undefined, ...info });
    } finally {
      this.busy = false;
    }
    return this.state;
  }

  /** Extrait l'archive vérifiée dans `versions/<version>` : complète ou absente, jamais à moitié. */
  async #stage(archive, version) {
    let files;
    try {
      files = unpack(archive);
    } catch (err) {
      throw Object.assign(new Error(err.message), { code: 'stage', detail: err.message });
    }
    const manifest = files.find((f) => f.name === 'package.json');
    let pkg;
    try {
      pkg = JSON.parse(manifest.data.toString('utf8'));
    } catch {
      throw Object.assign(new Error('package.json'), { code: 'stage', detail: 'package.json absent ou illisible' });
    }
    if (pkg.version !== version || !files.some((f) => f.name === 'server/index.js')) throw Object.assign(new Error('contenu'), { code: 'stage', detail: 'archive inattendue' });
    if (!nodeSatisfies(pkg.engines?.node)) throw Object.assign(new Error('node'), { code: 'engine', detail: `Node.js ${pkg.engines.node} requis` });

    const final = path.join(this.paths.versions, version);
    const work = `${final}.tmp-${process.pid}`;
    await fsp.rm(work, { recursive: true, force: true });
    try {
      for (const file of files) {
        const dest = path.join(work, file.name);
        // eslint-disable-next-line no-await-in-loop
        await fsp.mkdir(path.dirname(dest), { recursive: true });
        // eslint-disable-next-line no-await-in-loop
        await fsp.writeFile(dest, file.data, { mode: file.mode || 0o644 });
      }
      // Les modules de l'installation d'origine servent tant que les dépendances ne changent pas.
      const base = JSON.parse(await fsp.readFile(path.join(this.baseRoot, 'package.json'), 'utf8'));
      if (JSON.stringify(Object.entries(base.dependencies || {}).sort()) !== JSON.stringify(Object.entries(pkg.dependencies || {}).sort())) await this.#npmInstall(work);
      await fsp.writeFile(path.join(work, '.ok'), `${new Date().toISOString()}\n`);
      await fsp.rm(final, { recursive: true, force: true });
      await fsp.rename(work, final);
    } catch (err) {
      await fsp.rm(work, { recursive: true, force: true }).catch(() => {});
      throw err;
    }
  }

  /** Installe les dépendances d'une version qui en a changé (réseau et npm requis). */
  #npmInstall(cwd) {
    return new Promise((resolve, reject) => {
      const child = spawn(process.platform === 'win32' ? 'npm.cmd' : 'npm', ['ci', '--omit=dev', '--ignore-scripts', '--no-audit', '--no-fund'], { cwd, shell: process.platform === 'win32', stdio: ['ignore', 'ignore', 'pipe'] });
      let tail = '';
      child.stderr.on('data', (d) => { tail = (tail + d).slice(-600); });
      const timer = setTimeout(() => child.kill('SIGKILL'), 5 * 60 * 1000);
      child.on('error', (err) => { clearTimeout(timer); reject(Object.assign(new Error('npm'), { code: 'deps', detail: err.message })); });
      child.on('exit', (code) => {
        clearTimeout(timer);
        if (code === 0) resolve();
        else reject(Object.assign(new Error('npm'), { code: 'deps', detail: tail.trim() || `npm ci : code ${code}` }));
      });
    });
  }

  /** Choisit la nouvelle version pour le prochain démarrage, puis demande au lanceur de redémarrer le serveur. */
  async install() {
    if (this.state.phase !== 'ready' || !this.release) return this.state;
    const version = this.release.version;
    if (!this.supervised || !appstate.versionDir(this.dataDir, version)) {
      this.#set({ phase: 'error', code: this.supervised ? 'stage' : 'unsupervised', version });
      return this.state;
    }
    try {
      await this.backup();
      const st = appstate.readState(this.dataDir);
      appstate.writeState(this.dataDir, { target: version, previous: process.env.CONTROL_STAGED || null, bad: st.bad.filter((v) => v !== version), failure: null });
    } catch (err) {
      this.#set({ phase: 'error', code: 'stage', detail: err.message, version });
      return this.state;
    }
    this.#set({ phase: 'installing', version, name: this.release.name });
    setTimeout(() => this.restartServer(), 400);
    // Le lanceur ne redémarre pas : on le dit plutôt que d'afficher « Installation… » indéfiniment (la version reste choisie pour le prochain démarrage).
    setTimeout(() => { if (this.state.phase === 'installing') this.#set({ phase: 'error', code: 'launch', version }); }, 90 * 1000).unref();
    return this.state;
  }

  // ─── Planification ─────────────────────────────────────────────────────────
  /** Au lancement (une minute après), puis toutes les 6 heures ; en mode automatique, télécharge et installe. */
  start() {
    this.#cleanup().catch(() => {});
    const run = () => this.run().catch(() => {});
    this.timers.push(setTimeout(run, 60 * 1000), setInterval(run, CHECK_EVERY_MS), setInterval(() => this.auto().catch(() => {}), 60 * 1000));
    for (const t of this.timers) t.unref?.();
  }

  stop() {
    for (const t of this.timers) { clearTimeout(t); clearInterval(t); }
    this.timers = [];
  }

  /** Une vérification périodique, suivie de ce que le mode automatique en fait. */
  async run() {
    await this.check();
    await this.auto();
  }

  /** Mode automatique : télécharge, puis installe quand plus personne n'est connecté (ou à l'heure choisie). */
  async auto() {
    const { mode, hour } = this.#config();
    if (mode !== 'auto' || !this.supervised) return;
    if (this.state.phase === 'available' && !this.state.manual) await this.download();
    if (this.state.phase === 'ready' && (this.idle() || new Date().getHours() === hour)) await this.install();
  }

  /** Ménage au démarrage : téléchargements interrompus, versions dont on n'a plus besoin. */
  async #cleanup() {
    const st = appstate.readState(this.dataDir);
    // Une version installée dans l'image Docker (ou par l'administrateur) plus récente que celle téléchargée : on l'oublie.
    if (st.target && appstate.compareVersions(st.target, this.version) <= 0 && !process.env.CONTROL_STAGED) appstate.writeState(this.dataDir, { target: null, previous: null });
    const keep = new Set([st.target, st.previous, process.env.CONTROL_STAGED].filter(Boolean));
    try {
      for (const name of await fsp.readdir(this.paths.versions)) {
        if (/\.tmp-\d+$/.test(name) || (appstate.VERSION.test(name) && !keep.has(name))) await fsp.rm(path.join(this.paths.versions, name), { recursive: true, force: true });
      }
    } catch { /* aucun dossier de versions */ }
    await fsp.rm(this.paths.tmp, { recursive: true, force: true }).catch(() => {});
  }

  /** Copie de la base du serveur avant de changer de version (les 3 dernières sont gardées). */
  static async backupDb(db, dataDir, version) {
    const dir = path.join(dataDir, 'backups');
    await fsp.mkdir(dir, { recursive: true });
    const file = path.join(dir, `server-${version}-${Date.now()}.db`);
    db.exec(`VACUUM INTO '${file.replace(/'/g, "''")}'`);
    const all = (await fsp.readdir(dir)).filter((n) => /^server-.*\.db$/.test(n)).sort();
    for (const old of all.slice(0, Math.max(0, all.length - KEEP_BACKUPS))) await fsp.rm(path.join(dir, old), { force: true });
    return file;
  }
}

module.exports = { WebUpdater, nodeSatisfies };

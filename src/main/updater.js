'use strict';

// Mise à jour de l'application : cherche la dernière release GitHub, télécharge l'installeur (ou l'exécutable
// portable) en vérifiant son empreinte SHA-256, puis l'installe et relance control.

const fs = require('node:fs');
const fsp = require('node:fs/promises');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawn } = require('node:child_process');
const { EventEmitter } = require('node:events');

const REPO = 'nildontsleep/control';
const API = process.env.CONTROL_UPDATE_URL || `https://api.github.com/repos/${REPO}/releases/latest`;
const TRUSTED = [
  new RegExp(`^https://github\\.com/${REPO}/releases/download/`),
  new RegExp(`^https://api\\.github\\.com/repos/${REPO}/releases/assets/\\d+$`),
];
const AGENT = 'control-updater (https://github.com/nildontsleep/control)';

/** Compare deux versions « 1.2.3 » (le suffixe de préversion est ignoré). Positif si a > b. */
function compare(a, b) {
  const parts = (v) => String(v).replace(/^v/i, '').split('-')[0].split('.').map((n) => Number(n) || 0);
  const [x, y] = [parts(a), parts(b)];
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const d = (x[i] || 0) - (y[i] || 0);
    if (d) return d;
  }
  return 0;
}

/** Choisit le bon fichier de la release : installeur ou portable, selon l'édition en cours. */
function pickAsset(assets, portable) {
  const want = portable ? /-portable\.exe$/i : /-setup\.exe$/i;
  return (assets || []).find((a) => want.test(a.name) && a.browser_download_url) || null;
}

const sha256File = (file) => new Promise((resolve, reject) => {
  const hash = crypto.createHash('sha256');
  fs.createReadStream(file).on('data', (c) => hash.update(c)).on('end', () => resolve(hash.digest('hex'))).on('error', reject);
});

class Updater extends EventEmitter {
  /**
   * @param {object} o
   * @param {string} o.version  version en cours
   * @param {boolean} o.portable  édition portable
   * @param {string} o.dir  dossier de travail pour les téléchargements
   * @param {Function} o.fetch  équivalent de fetch (net.fetch d'Electron)
   * @param {Function} o.quit  ferme l'application
   * @param {string} [o.portableFile]  chemin de l'exécutable portable en cours
   * @param {Function} [o.token]  jeton GitHub en lecture (dépôt privé), lu à chaque requête
   * @param {Function} [o.redirect]  (url, headers) → adresse de redirection (sans la suivre), ou null
   */
  constructor({ version, portable, dir, fetch: fetchFn, quit, portableFile, token, redirect }) {
    super();
    this.token = token || (() => '');
    this.redirect = redirect || (async () => null);
    this.version = version;
    this.portable = !!portable;
    this.dir = dir;
    this.fetch = fetchFn;
    this.quit = quit;
    this.portableFile = portableFile || '';
    this.release = null;
    this.file = null;
    this.state = { phase: 'idle', current: version };
    this.busy = false;
  }

  #set(patch) {
    this.state = { current: this.version, ...patch };
    this.emit('state', this.state);
  }

  get status() {
    return this.state;
  }


  #auth() {
    const token = String(this.token() || '').trim();
    return token ? { authorization: `Bearer ${token}` } : {};
  }

  async #api() {
    const res = await this.fetch(API, { headers: { 'user-agent': AGENT, accept: 'application/vnd.github+json', ...this.#auth() }, signal: AbortSignal.timeout(12000) });
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

  /** Sans l'API : « releases/latest » redirige vers la page du dernier tag, et les fichiers portent un nom prévisible. */
  async #web() {
    const signal = AbortSignal.timeout(12000);
    const page = await this.fetch(`https://github.com/${REPO}/releases/latest`, { method: 'HEAD', headers: { 'user-agent': AGENT }, signal });
    const tag = /\/releases\/tag\/([^/?#]+)/.exec(page.url || '')?.[1];
    if (!page.ok || !tag) throw Object.assign(new Error('web'), { code: 'http' });
    const version = decodeURIComponent(tag).replace(/^v/i, '');
    const assets = [];
    for (const kind of ['setup', 'portable']) {
      const name = `control-${version}-${kind}.exe`;
      const url = `https://github.com/${REPO}/releases/download/${tag}/${name}`;
      // eslint-disable-next-line no-await-in-loop
      const head = await this.fetch(url, { method: 'HEAD', headers: { 'user-agent': AGENT }, signal }).catch(() => null);
      if (head?.ok) assets.push({ name, size: Number(head.headers.get('content-length')) || 0, browser_download_url: url, digest: null });
    }
    return { tag_name: tag, name: `v${version}`, body: '', html_url: `https://github.com/${REPO}/releases/tag/${tag}`, assets };
  }

  /** Cherche une nouvelle version. `manual` : un échec réseau est signalé au lieu d'être passé sous silence. */
  async check({ manual = false } = {}) {
    if (this.busy || ['downloading', 'installing'].includes(this.state.phase)) return this.state;
    this.busy = true;
    if (manual) this.#set({ phase: 'checking' });
    try {
      let rel;
      try {
        rel = await this.#api();
      } catch (err) {
        // L'API GitHub limite les requêtes anonymes (60 par heure et par adresse IP) : on passe alors par les pages publiques.
        if (err.code !== 'busy' || process.env.CONTROL_UPDATE_URL) throw err;
        rel = await this.#web();
      }
      const latest = String(rel.tag_name || '').replace(/^v/i, '');
      if (!latest || rel.draft || rel.prerelease || compare(latest, this.version) <= 0) {
        this.release = null;
        this.#set({ phase: 'none', checkedAt: Date.now() });
        return this.state;
      }
      const asset = pickAsset(rel.assets, this.portable);
      if (!asset) throw Object.assign(new Error('asset'), { code: 'asset' });
      // Dépôt privé : le téléchargement passe par l'API (avec le jeton) ; sinon par l'adresse publique.
      const authed = !!this.#auth().authorization;
      const url = authed && asset.url ? asset.url : asset.browser_download_url;
      if (!TRUSTED.some((re) => re.test(url)) && !process.env.CONTROL_UPDATE_URL) throw Object.assign(new Error('untrusted'), { code: 'untrusted' });
      const digest = /^sha256:([0-9a-f]{64})$/i.exec(asset.digest || '')?.[1]?.toLowerCase() || null;
      this.release = { version: latest, name: rel.name || `v${latest}`, notes: String(rel.body || '').slice(0, 4000), page: rel.html_url, url, size: asset.size || 0, digest, fileName: asset.name, api: url === asset.url };
      this.#set({ phase: 'available', version: latest, name: this.release.name, notes: this.release.notes, page: this.release.page, size: this.release.size, checkedAt: Date.now() });
    } catch (err) {
      if (manual) this.#set({ phase: 'error', code: err.code || 'network', checkedAt: Date.now() });
      else if (this.state.phase === 'checking') this.#set({ phase: 'idle' });
    } finally {
      this.busy = false;
    }
    return this.state;
  }

  /** Télécharge le fichier de la mise à jour et vérifie son empreinte. */
  async download() {
    const rel = this.release;
    if (!rel || this.state.phase === 'downloading') return this.state;
    await fsp.mkdir(this.dir, { recursive: true });
    const file = path.join(this.dir, rel.fileName);
    const part = `${file}.part`;
    this.#set({ phase: 'downloading', version: rel.version, size: rel.size, received: 0 });
    try {
      let url = rel.url;
      let headers = { 'user-agent': AGENT };
      if (rel.api) {
        // Dépôt privé : l'API répond par une redirection vers une adresse signée. Le jeton ne doit jamais la suivre
        // (fetch le transmettrait au serveur de destination) : on lit la redirection, puis on télécharge sans jeton.
        const target = await this.redirect(rel.url, { 'user-agent': AGENT, accept: 'application/octet-stream', ...this.#auth() });
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
      const out = fs.createWriteStream(part);
      const hash = crypto.createHash('sha256');
      let received = 0;
      let last = 0;
      const reader = res.body.getReader();
      for (;;) {
        // eslint-disable-next-line no-await-in-loop
        const { done, value } = await reader.read();
        if (done) break;
        hash.update(value);
        received += value.length;
        if (!out.write(value)) await new Promise((r) => out.once('drain', r)); // eslint-disable-line no-await-in-loop
        if (Date.now() - last > 200) {
          last = Date.now();
          this.#set({ phase: 'downloading', version: rel.version, size: total, received });
        }
      }
      await new Promise((resolve, reject) => { out.end((err) => (err ? reject(err) : resolve())); });
      if (rel.size && received !== rel.size) throw Object.assign(new Error('size'), { code: 'corrupt' });
      if (rel.digest && hash.digest('hex') !== rel.digest) throw Object.assign(new Error('digest'), { code: 'corrupt' });
      await fsp.rename(part, file);
      this.file = file;
      this.#set({ phase: 'ready', version: rel.version, size: total, verified: !!rel.digest });
    } catch (err) {
      await fsp.rm(part, { force: true }).catch(() => {});
      this.#set({ phase: 'error', code: err.code || 'network', version: rel.version });
    }
    return this.state;
  }

  /** Lance l'installation puis ferme l'application ; le nouvel exécutable la relance. */
  async install() {
    if (this.state.phase !== 'ready' || !this.file) return this.state;
    // Le fichier a pu être modifié depuis la vérification : on revérifie avant de l'exécuter.
    if (this.release?.digest && (await sha256File(this.file)) !== this.release.digest) {
      await fsp.rm(this.file, { force: true }).catch(() => {});
      this.#set({ phase: 'error', code: 'corrupt', version: this.release.version });
      return this.state;
    }
    const plan = this.portable ? this.#portablePlan() : { command: this.file, args: ['/S', '--updated', '--force-run'], options: {} };
    if (!plan) {
      this.#set({ phase: 'error', code: 'unsupported', version: this.release?.version });
      return this.state;
    }
    this.#set({ phase: 'installing', version: this.release?.version });
    if (process.env.CONTROL_UPDATE_DRYRUN) {
      // Essais : on note la commande au lieu de l'exécuter.
      await fsp.writeFile(process.env.CONTROL_UPDATE_DRYRUN, JSON.stringify({ command: plan.command, args: plan.args, script: plan.script || null }, null, 2));
      return this.state;
    }
    try {
      if (plan.script) await fsp.writeFile(plan.scriptPath, plan.script, 'utf8');
      const child = spawn(plan.command, plan.args, { detached: true, stdio: 'ignore', windowsHide: true, ...plan.options });
      child.unref();
      child.on('error', () => this.#set({ phase: 'error', code: 'launch', version: this.release?.version }));
      setTimeout(() => this.quit(), 400);
    } catch {
      this.#set({ phase: 'error', code: 'launch', version: this.release?.version });
    }
    return this.state;
  }

  /**
   * Édition portable : un petit script attend que control se ferme, remplace l'ancien exécutable par le nouveau
   * (rangé à côté, au nom de la nouvelle version), puis le lance.
   */
  #portablePlan() {
    if (process.platform !== 'win32' && !process.env.CONTROL_UPDATE_DRYRUN) return null;
    const old = this.portableFile;
    if (!old) return null;
    try {
      fs.accessSync(path.dirname(old), fs.constants.W_OK); // dossier protégé : mieux vaut le dire que fermer l'application pour rien
    } catch {
      return null;
    }
    const dest = path.join(path.dirname(old), this.release.fileName);
    const scriptPath = path.join(this.dir, 'apply-update.cmd');
    const q = (p) => `"${String(p).replace(/"/g, '')}"`;
    const script = [
      '@echo off',
      'chcp 65001 >nul',
      'setlocal',
      'set /a n=0',
      ':wait',
      'set /a n+=1',
      `if %n% gtr 90 ( if exist ${q(old)} start "" ${q(old)} & exit /b 1 )`,
      'ping 127.0.0.1 -n 2 >nul',
      // Tant que control tourne, l'ancien exécutable est verrouillé et ne peut pas être supprimé.
      `if exist ${q(old)} del /F /Q ${q(old)} >nul 2>&1`,
      `if exist ${q(old)} goto wait`,
      `move /Y ${q(this.file)} ${q(dest)} >nul 2>&1`,
      `if not exist ${q(dest)} exit /b 1`,
      `start "" ${q(dest)}`,
      '(goto) 2>nul & del "%~f0"',
      '',
    ].join('\r\n');
    return { command: process.env.ComSpec || 'cmd.exe', args: ['/d', '/s', '/c', `"${q(scriptPath)}"`], options: { windowsVerbatimArguments: true }, script, scriptPath };
  }

  /** Nettoie les anciens téléchargements (appelé au démarrage, une fois la nouvelle version lancée). */
  async cleanup() {
    try {
      for (const name of await fsp.readdir(this.dir)) {
        const m = /-(\d+\.\d+\.\d+)-(setup|portable)\.exe(\.part)?$/.exec(name);
        if (m && (compare(m[1], this.version) <= 0 || m[3])) await fsp.rm(path.join(this.dir, name), { force: true });
      }
      await fsp.rm(path.join(this.dir, 'apply-update.cmd'), { force: true });
    } catch { /* dossier absent */ }
  }
}

module.exports = { Updater, compare, pickAsset };

'use strict';

// Sources : « cet ordinateur » et des serveurs control (la version web, hébergée ailleurs).
//
// Un serveur s'affiche dans la fenêtre, sous la barre que l'interface locale dessine : c'est une vue web à part, avec son
// propre stockage (la connexion est conservée), sans pont vers le système. L'interface locale reste en dessous, intacte,
// et continue de jouer ; basculer d'une source à l'autre ne coupe rien. Une seule source joue à la fois : quand l'une
// démarre, l'autre est mise en pause.

const crypto = require('node:crypto');
const { WebContentsView, session, net } = require('electron');
const { candidates } = require('./source-url');

/** Hauteur de la barre de fenêtre dessinée par l'interface locale, au-dessus du serveur affiché. */
const BAR = 40;
const PERMISSIONS = new Set(['fullscreen', 'clipboard-sanitized-write']);

async function probe(origin) {
  const res = await net.fetch(`${origin}/api/health`, { signal: AbortSignal.timeout(6000) });
  const body = res.ok ? await res.json().catch(() => null) : null;
  if (!body?.ok || body.name !== 'control') throw new Error('Ce n’est pas un serveur control.');
  return body;
}

/** Message lisible pour une erreur réseau de Chromium (« net::ERR_… »). */
function readable(err) {
  const text = String(err?.message || err);
  if (/CERT|SSL/i.test(text)) return 'Le certificat du serveur n’est pas accepté.';
  if (/UNSAFE_PORT/i.test(text)) return 'Ce port est refusé par Chromium (port réservé) : choisissez-en un autre.';
  if (/REFUSED|RESET|UNREACHABLE|NAME_NOT_RESOLVED|TIMED_OUT|ABORT|TimeoutError|fetch failed/i.test(text)) return 'Impossible de joindre le serveur.';
  return text.replace(/^Error:\s*/, '');
}

class Sources {
  /**
   * @param store      réglages (servers, source)
   * @param send       (canal, charge) vers l'interface locale
   * @param guard      (webContents, origine) : fenêtres et liens externes d'une vue distante
   */
  constructor({ store, send, guard }) {
    this.store = store;
    this.send = send;
    this.guard = guard;
    this.win = null;
    this.views = new Map(); // id → { view, state: 'loading' | 'ready' | 'error', error }
    this.htmlFullscreen = false;
    this.peeking = false;
  }

  get servers() {
    return this.store.data.servers;
  }

  get active() {
    return this.store.data.source;
  }

  state() {
    const entry = this.views.get(this.active);
    return {
      servers: this.servers.map(({ id, name, url }) => ({ id, name, url })),
      active: this.servers.some((s) => s.id === this.active) ? this.active : 'local',
      status: entry ? { state: entry.state, error: entry.error || '' } : { state: 'idle' },
    };
  }

  push() {
    this.send('sources:changed', this.state());
  }

  attach(win) {
    this.win = win;
    this.views.clear();
    for (const ev of ['resize', 'maximize', 'unmaximize', 'restore', 'enter-full-screen', 'leave-full-screen']) win.on(ev, () => this.layout());
    win.on('closed', () => {
      this.views.clear();
      this.win = null;
    });
    // Le lecteur local démarre : les serveurs se taisent.
    win.webContents.on('media-started-playing', () => this.pauseRemotes());
    // Reprise là où on s'était arrêté (le serveur choisi la dernière fois).
    win.once('show', () => {
      if (this.active !== 'local') this.switch(this.active).catch(() => {});
    });
  }

  layout() {
    if (!this.win || this.win.isDestroyed()) return;
    const [width, height] = this.win.getContentSize();
    const bounds = this.htmlFullscreen ? { x: 0, y: 0, width, height } : { x: 0, y: BAR, width, height: Math.max(0, height - BAR) };
    for (const { view } of this.views.values()) view.setBounds(bounds);
  }

  #setState(entry, state, error = '') {
    entry.state = state;
    entry.error = error;
    if (this.views.get(this.active) === entry) {
      this.#show(entry);
      this.push();
    }
  }

  /** Montre la vue du serveur actif quand elle est prête (et qu'aucun menu de l'interface locale n'est ouvert). */
  #show(entry) {
    entry.view.setVisible(entry.state === 'ready' && !this.peeking);
    if (entry.state === 'ready' && !this.peeking && this.views.get(this.active) === entry) entry.view.webContents.focus();
  }

  #viewFor(server) {
    let entry = this.views.get(server.id);
    if (entry) return entry;
    const ses = session.fromPartition(`persist:control-src-${server.id}`);
    ses.setPermissionRequestHandler((_wc, permission, done) => done(PERMISSIONS.has(permission)));
    ses.setPermissionCheckHandler((_wc, permission) => PERMISSIONS.has(permission));
    // Le service worker du serveur sert à l'installer sur un téléphone ; ici il bloquerait le chargement au lancement suivant.
    ses.webRequest.onBeforeRequest({ urls: ['*://*/sw.js'] }, (_details, done) => done({ cancel: true }));
    const view = new WebContentsView({
      webPreferences: { session: ses, contextIsolation: true, nodeIntegration: false, sandbox: true, spellcheck: false, backgroundThrottling: false },
    });
    view.setBackgroundColor('#0c0c0f');
    view.setVisible(false);
    entry = { view, state: 'loading', error: '' };
    this.views.set(server.id, entry);
    const wc = view.webContents;
    this.guard(wc, server.url);
    // Chromium charge sa page d'erreur après un échec : elle ne compte pas comme une page prête.
    wc.on('did-start-navigation', (details) => {
      if (details.isMainFrame && !details.isSameDocument && !details.url.startsWith('chrome-error:')) entry.failed = false;
    });
    wc.on('did-finish-load', () => { if (!entry.failed) this.#setState(entry, 'ready'); });
    wc.on('did-fail-load', (_e, code, description, _url, isMainFrame) => {
      if (!isMainFrame || code === -3) return;
      entry.failed = true;
      this.#setState(entry, 'error', readable(description));
    });
    // Un lecteur plein écran (vidéo, mode immersif du serveur) prend toute la fenêtre, barre comprise.
    wc.on('enter-html-full-screen', () => { this.htmlFullscreen = true; this.layout(); });
    wc.on('leave-html-full-screen', () => { this.htmlFullscreen = false; this.layout(); });
    wc.on('media-started-playing', () => {
      this.send('sources:pause');
      this.pauseRemotes(server.id);
    });
    this.win.contentView.addChildView(view);
    this.layout();
    ses.clearStorageData({ storages: ['serviceworkers'] }).catch(() => {}).then(() => wc.loadURL(`${server.url}/`)).catch(() => {});
    return entry;
  }

  async switch(id) {
    if (!this.win || this.win.isDestroyed()) return this.state();
    if (id !== 'local' && !this.servers.some((s) => s.id === id)) id = 'local';
    this.store.data.source = id;
    this.store.save();
    for (const entry of this.views.values()) entry.view.setVisible(false);
    if (id === 'local') {
      this.htmlFullscreen = false;
      this.win.webContents.focus();
    } else {
      const entry = this.#viewFor(this.servers.find((s) => s.id === id));
      this.win.contentView.addChildView(entry.view); // au-dessus des autres
      this.layout();
      this.#show(entry);
    }
    this.push();
    return this.state();
  }

  /**
   * L'interface locale doit dessiner un menu par-dessus le serveur, mais une vue web native passe toujours au-dessus de la
   * page : on en prend une capture, on cache la vue, et la page affiche la capture à sa place le temps du menu.
   */
  async peek() {
    const entry = this.views.get(this.active);
    if (!entry || entry.state !== 'ready') return null;
    let image = null;
    try {
      image = `data:image/jpeg;base64,${(await entry.view.webContents.capturePage()).toJPEG(80).toString('base64')}`;
    } catch { /* pas de capture : le menu s'ouvrira sous la vue, sans plus */ }
    this.peeking = true;
    entry.view.setVisible(false);
    return image;
  }

  unpeek() {
    this.peeking = false;
    const entry = this.views.get(this.active);
    if (entry) this.#show(entry);
  }

  async reload() {
    const server = this.servers.find((s) => s.id === this.active);
    if (!server) return this.state();
    const entry = this.#viewFor(server);
    this.#setState(entry, 'loading');
    entry.view.webContents.loadURL(`${server.url}/`).catch(() => {});
    return this.state();
  }

  async add({ name, url } = {}) {
    let origins;
    try {
      origins = candidates(url);
    } catch (err) {
      return { ok: false, error: err.message };
    }
    let found = null;
    let lastError = null;
    for (const origin of origins) {
      try {
        await probe(origin);
        found = origin;
        break;
      } catch (err) {
        lastError = err;
      }
    }
    if (!found) return { ok: false, error: readable(lastError) };
    if (this.servers.some((s) => s.url === found)) return { ok: false, error: 'Ce serveur est déjà dans la liste.' };
    const server = { id: `s${crypto.randomBytes(5).toString('hex')}`, name: String(name || '').trim().slice(0, 60) || new URL(found).host, url: found };
    this.servers.push(server);
    this.store.save();
    this.push();
    return { ok: true, server };
  }

  rename(id, name) {
    const server = this.servers.find((s) => s.id === id);
    if (!server) return { ok: false, error: 'Serveur inconnu.' };
    server.name = String(name || '').trim().slice(0, 60) || server.name;
    this.store.save();
    this.push();
    return { ok: true };
  }

  async remove(id) {
    const index = this.servers.findIndex((s) => s.id === id);
    if (index < 0) return { ok: false, error: 'Serveur inconnu.' };
    if (this.active === id) await this.switch('local');
    const entry = this.views.get(id);
    if (entry) {
      this.win?.contentView.removeChildView(entry.view);
      entry.view.webContents.close();
      this.views.delete(id);
    }
    // La connexion à ce serveur est oubliée avec lui.
    const ses = session.fromPartition(`persist:control-src-${id}`);
    await ses.clearStorageData().catch(() => {});
    await ses.clearCache().catch(() => {});
    this.servers.splice(index, 1);
    this.store.save();
    this.push();
    return { ok: true };
  }

  /** Met en pause les médias des vues distantes (sauf celle dont l'identifiant est donné). */
  pauseRemotes(except) {
    for (const [id, { view }] of this.views) {
      if (id === except || view.webContents.isDestroyed()) continue;
      view.webContents.executeJavaScript("for (const m of document.querySelectorAll('audio, video')) m.pause()").catch(() => {});
    }
  }
}

module.exports = { Sources, BAR };

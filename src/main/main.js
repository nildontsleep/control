'use strict';

const {
  app, BrowserWindow, ipcMain, protocol, net, session, shell, dialog, screen, Menu,
} = require('electron');
const path = require('node:path');
const fs = require('node:fs');
const { Readable } = require('node:stream');
const { pathToFileURL } = require('node:url');
const { Core } = require('./core');
const { DiscordPresence } = require('./discord');
const { Updater } = require('./updater');
const { frameOrigins, imageOrigins } = require('./media');

// ─── Mode portable : les données restent à côté de l'exécutable ──────────────
const PORTABLE_DIR = process.env.PORTABLE_EXECUTABLE_DIR;
if (PORTABLE_DIR) app.setPath('userData', path.join(PORTABLE_DIR, 'control data'));

app.setAppUserModelId('com.nildontsleep.control');
app.commandLine.appendSwitch('autoplay-policy', 'no-user-gesture-required');

// ─── Linux ───────────────────────────────────────────────────────────────────
if (process.platform === 'linux') {
  // Touches multimédia et panneau « en cours de lecture » du bureau (MPRIS).
  app.commandLine.appendSwitch('enable-features', 'HardwareMediaKeyHandling,MediaSessionService');
  // Le bac à sable de Chromium demande soit l'espace utilisateur isolé (interdit par défaut sur Ubuntu 24.04 et
  // certains Debian), soit un `chrome-sandbox` appartenant à root avec le bit setuid. Le paquet .deb règle cela à
  // l'installation (profil AppArmor, bit setuid) ; une AppImage ne peut rien installer. Si aucun des deux n'est
  // disponible, Chromium refuse de démarrer : pour l'AppImage seulement, on désactive alors le bac à sable.
  const readFlag = (file) => { try { return fs.readFileSync(file, 'utf8').trim(); } catch { return null; } };
  const userNamespaces = readFlag('/proc/sys/kernel/unprivileged_userns_clone') !== '0'
    && readFlag('/proc/sys/kernel/apparmor_restrict_unprivileged_userns') !== '1';
  const suidSandbox = (() => {
    try {
      const st = fs.statSync(path.join(path.dirname(process.execPath), 'chrome-sandbox'));
      return st.uid === 0 && (st.mode & 0o4000) !== 0;
    } catch { return false; }
  })();
  if (process.env.APPIMAGE && !userNamespaces && !suidSandbox) app.commandLine.appendSwitch('no-sandbox');
}

const RENDERER_DIR = path.join(__dirname, '..', 'renderer');
const PRELOAD = path.join(__dirname, 'preload.js');
const APP_ICON = path.join(RENDERER_DIR, 'assets', 'icon.png');

protocol.registerSchemesAsPrivileged([{
  scheme: 'app',
  privileges: { standard: true, secure: true, supportFetchAPI: true, stream: true, codeCache: true },
}]);

const gotLock = app.requestSingleInstanceLock();
if (!gotLock) app.quit();

let core;
let store;
let library;
let tools;
let downloads;
let media;
let tags;
let stats;
let lyrics;
let discord;
let updater;
let mainWin = null;
let quitting = false;
let boundsTimer = null;
let watchers = [];
let watchTimer = null;

const broadcast = (channel, payload) => {
  for (const win of BrowserWindow.getAllWindows()) {
    if (!win.isDestroyed()) win.webContents.send(channel, payload);
  }
};

// ─── Dossiers ────────────────────────────────────────────────────────────────
const defaultDownloadDir = () => path.join(app.getPath('music'), 'control');

function scanRoots() {
  const roots = [...(store.get('libraryFolders', []) || [])];
  const dl = store.get('downloadDir', '') || defaultDownloadDir();
  if (fs.existsSync(dl) && !roots.some((r) => dl.toLowerCase().startsWith(r.toLowerCase()))) roots.push(dl);
  return roots;
}

function setupWatchers() {
  watchers.forEach((w) => w.close());
  watchers = [];
  const schedule = () => {
    clearTimeout(watchTimer);
    watchTimer = setTimeout(() => library.scan(scanRoots()), 5000);
  };
  for (const root of scanRoots()) {
    try {
      const watcher = fs.watch(root, { recursive: true }, (event, name) => {
        if (name && /\.(part|ytdl|tmp|temp)$/i.test(String(name))) return;
        schedule();
      });
      watcher.on('error', () => {});
      watchers.push(watcher);
    } catch { /* dossier inaccessible */ }
  }
}

// ─── Fenêtre ─────────────────────────────────────────────────────────────────
const boundsAreVisible = (b) => b && typeof b.x === 'number' && screen.getAllDisplays().some(({ workArea: w }) =>
  b.x + 120 < w.x + w.width && b.x + b.width - 120 > w.x && b.y + 40 < w.y + w.height && b.y + 20 > w.y);

function windowState(win) {
  return {
    maximized: win.isMaximized(),
    fullscreen: win.isFullScreen(),
    focused: win.isFocused(),
  };
}

function createWindow() {
  const saved = store.data.window.main || {};
  const opts = {
    width: 1280,
    height: 820,
    minWidth: 920,
    minHeight: 600,
    frame: false,
    show: false,
    title: 'control',
    icon: APP_ICON,
    backgroundColor: '#0c0c0f',
    webPreferences: {
      preload: PRELOAD,
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      spellcheck: false,
      backgroundThrottling: false,
    },
  };
  if (saved.bounds && boundsAreVisible(saved.bounds)) Object.assign(opts, saved.bounds);
  else opts.center = true;

  mainWin = new BrowserWindow(opts);
  // Au premier clic, le lecteur vidrift vérifie qu'il peut ouvrir une fenêtre (c'est ainsi qu'il détecte un iframe
  // « sandboxé ») et affiche « Remove sandboxing » si window.open() échoue. On accepte donc cette fenêtre blanche, mais
  // cachée, sans réseau et refermée aussitôt : toute vraie page (publicité…) reste refusée.
  mainWin.webContents.setWindowOpenHandler(({ url }) => {
    if (url !== 'about:blank') return { action: 'deny' };
    return {
      action: 'allow',
      overrideBrowserWindowOptions: {
        show: false,
        width: 1,
        height: 1,
        skipTaskbar: true,
        focusable: false,
        frame: false,
        webPreferences: { sandbox: true, contextIsolation: true, nodeIntegration: false },
      },
    };
  });
  mainWin.webContents.on('did-create-window', (probe) => {
    probe.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
    for (const ev of ['will-navigate', 'will-redirect']) probe.webContents.on(ev, (e) => e.preventDefault());
    // Le lecteur la ferme lui-même ; filet de sécurité s'il ne le faisait pas.
    const timer = setTimeout(() => { if (!probe.isDestroyed()) probe.destroy(); }, 4000);
    probe.once('closed', () => clearTimeout(timer));
  });
  mainWin.webContents.on('will-navigate', (e, url) => {
    if (!url.startsWith('app://nilcam/')) e.preventDefault();
  });
  if (saved.maximized) mainWin.maximize();
  mainWin.loadURL('app://nilcam/index.html');
  mainWin.once('ready-to-show', () => mainWin.show());

  const push = () => mainWin && !mainWin.isDestroyed() && mainWin.webContents.send('window:state', windowState(mainWin));
  for (const ev of ['maximize', 'unmaximize', 'enter-full-screen', 'leave-full-screen', 'focus', 'blur']) mainWin.on(ev, push);

  const remember = () => {
    clearTimeout(boundsTimer);
    boundsTimer = setTimeout(() => {
      if (!mainWin || mainWin.isDestroyed()) return;
      store.data.window.main = {
        bounds: mainWin.isMaximized() || mainWin.isFullScreen() ? (store.data.window.main?.bounds || null) : mainWin.getBounds(),
        maximized: mainWin.isMaximized(),
      };
      store.save();
    }, 400);
  };
  mainWin.on('resize', remember);
  mainWin.on('move', remember);
  mainWin.on('closed', () => { mainWin = null; });
}

function showMain() {
  if (!mainWin) createWindow();
  if (mainWin.isMinimized()) mainWin.restore();
  mainWin.show();
  mainWin.focus();
}

// ─── Protocole app:// ────────────────────────────────────────────────────────
const MIME = {
  '.mp3': 'audio/mpeg', '.flac': 'audio/flac', '.m4a': 'audio/mp4', '.m4b': 'audio/mp4', '.aac': 'audio/aac',
  '.ogg': 'audio/ogg', '.oga': 'audio/ogg', '.opus': 'audio/ogg', '.wav': 'audio/wav',
  '.mp4': 'video/mp4', '.m4v': 'video/mp4', '.mov': 'video/mp4', '.webm': 'video/webm', '.mkv': 'video/webm',
};

const notFound = () => new Response('Not found', { status: 404, headers: { 'content-type': 'text/plain' } });

/** Sert un fichier avec prise en charge des requêtes Range (indispensable pour la lecture et le seek). */
async function serveFile(request, file, type) {
  let stat;
  try {
    stat = await fs.promises.stat(file);
  } catch {
    return notFound();
  }
  const headers = { 'content-type': type, 'accept-ranges': 'bytes' };
  const range = /^bytes=(\d*)-(\d*)$/.exec(request.headers.get('range') || '');
  if (range && stat.size > 0) {
    let start = range[1] ? parseInt(range[1], 10) : 0;
    let end = range[2] ? parseInt(range[2], 10) : stat.size - 1;
    if (!range[1] && range[2]) {
      start = Math.max(0, stat.size - parseInt(range[2], 10));
      end = stat.size - 1;
    }
    end = Math.min(end, stat.size - 1);
    if (start > end) return new Response(null, { status: 416, headers: { 'content-range': `bytes */${stat.size}` } });
    return new Response(Readable.toWeb(fs.createReadStream(file, { start, end })), {
      status: 206,
      headers: { ...headers, 'content-range': `bytes ${start}-${end}/${stat.size}`, 'content-length': String(end - start + 1) },
    });
  }
  return new Response(Readable.toWeb(fs.createReadStream(file)), { status: 200, headers: { ...headers, 'content-length': String(stat.size) } });
}

/** En-tête CSP de la page : les iframes ne peuvent venir que du lecteur vidrift. */
function contentSecurityPolicy() {
  const images = ['\'self\'', 'data:', 'blob:', 'app:', 'https:', ...imageOrigins()].join(' ');
  return [
    "default-src 'self'",
    "script-src 'self'",
    "style-src 'self' 'unsafe-inline'",
    `img-src ${images}`,
    "media-src 'self' blob: app:",
    "font-src 'self'",
    "connect-src 'self' app: blob:",
    `frame-src ${frameOrigins().join(' ')}`,
    "object-src 'none'",
    "base-uri 'none'",
  ].join('; ');
}

function registerProtocol() {
  protocol.handle('app', async (request) => {
    const url = new URL(request.url);
    if (url.host !== 'nilcam') return notFound();
    const pathname = decodeURIComponent(url.pathname);

    if (pathname.startsWith('/media/')) {
      const id = Number(pathname.slice(7));
      const file = Number.isInteger(id) ? library.trackPath(id) : null;
      if (!file) return notFound();
      return serveFile(request, file, MIME[path.extname(file).toLowerCase()] || 'application/octet-stream');
    }
    if (pathname.startsWith('/cover/')) {
      const name = pathname.slice(7);
      if (!/^[0-9a-f]{20}(-s)?\.jpg$/.test(name)) return notFound();
      const res = await serveFile(request, path.join(library.coversDir, name), 'image/jpeg');
      res.headers.set('cache-control', 'public, max-age=31536000, immutable');
      return res;
    }
    if (pathname.startsWith('/vthumb/')) {
      const id = Number(pathname.slice(8));
      const file = Number.isInteger(id) ? await library.videoThumb(id, tools.path('ffmpeg')) : null;
      if (!file) return notFound();
      return serveFile(request, file, 'image/jpeg');
    }

    const file = path.normalize(path.join(RENDERER_DIR, pathname));
    if (!file.startsWith(RENDERER_DIR + path.sep)) return notFound();
    if (pathname === '/index.html') {
      return new Response(await fs.promises.readFile(file), {
        headers: { 'content-type': 'text/html; charset=utf-8', 'content-security-policy': contentSecurityPolicy() },
      });
    }
    return net.fetch(pathToFileURL(file).toString());
  });
}

function setupPermissions() {
  const allowed = new Set(['media', 'fullscreen', 'clipboard-sanitized-write']);
  const players = frameOrigins();
  const fromApp = (u) => typeof u === 'string' && (u.startsWith('app://nilcam') || players.some((o) => u.startsWith(o)));
  session.defaultSession.setPermissionRequestHandler((wc, permission, callback, details) => {
    callback(allowed.has(permission) && fromApp(details.requestingUrl || wc.getURL()));
  });
  session.defaultSession.setPermissionCheckHandler((wc, permission, origin) => allowed.has(permission) && fromApp(origin || wc?.getURL()));
}

// ─── IPC ─────────────────────────────────────────────────────────────────────
function configureDiscord() {
  const d = store.get('discord', {}) || {};
  discord.configure({ enabled: d.enabled !== false, clientId: d.clientId });
}

/** Adresse vers laquelle `url` redirige, sans la suivre (null si la réponse est directe). */
function redirectTarget(url, headers) {
  return new Promise((resolve, reject) => {
    const req = net.request({ url, headers, redirect: 'manual' });
    const timer = setTimeout(() => { req.abort(); reject(new Error('timeout')); }, 15000);
    req.on('redirect', (_status, _method, location) => { clearTimeout(timer); req.abort(); resolve(location); });
    req.on('response', (res) => { clearTimeout(timer); res.on('data', () => {}); req.abort(); resolve(null); });
    req.on('error', (err) => { clearTimeout(timer); reject(err); });
    req.end();
  });
}

/** Cherche une mise à jour peu après le lancement, puis toutes les 6 heures (si l'option est active). */
function scheduleUpdateChecks() {
  const run = () => {
    if ((store.get('update', {}) || {}).auto !== false) updater.check();
  };
  setTimeout(run, 12000).unref?.();
  setInterval(run, 6 * 3600 * 1000).unref?.();
}

function applySettingEffects(patch, { scan = true } = {}) {
  if ('libraryFolders' in patch || 'downloadDir' in patch) {
    setupWatchers();
    if (scan) library.scan(scanRoots());
  }
  if ('discord' in patch) configureDiscord();
}

function setupIpc() {
  const handle = (channel, fn) => ipcMain.handle(channel, (event, ...args) => fn(event, ...args));
  const winOf = (e) => BrowserWindow.fromWebContents(e.sender);

  // Ce que le noyau partage avec le serveur web : réglages, bibliothèque, paroles, stats, tags, playlists, téléchargements.
  core.register(ipcMain, {
    scanRoots,
    applySettingEffects,
    emitOthers: (e, channel, payload) => {
      for (const win of BrowserWindow.getAllWindows()) if (win.webContents !== e.sender) win.webContents.send(channel, payload);
    },
    onReset: () => {
      setupWatchers();
      configureDiscord();
    },
  });

  ipcMain.on('settings:get-all', (e) => { e.returnValue = store.settings; });

  handle('update:state', () => updater.status);
  handle('update:check', () => updater.check({ manual: true }));
  handle('update:download', () => updater.download());
  handle('update:install', () => updater.install());

  ipcMain.on('discord:activity', (_e, activity) => discord.setActivity(activity));
  handle('discord:status', () => discord.status);
  handle('discord:test', () => discord.test());

  ipcMain.on('win:close', (e) => winOf(e)?.close());
  ipcMain.on('win:minimize', (e) => winOf(e)?.minimize());
  ipcMain.on('win:toggle-maximize', (e) => {
    const w = winOf(e);
    if (!w) return;
    if (w.isFullScreen()) w.setFullScreen(false);
    else if (w.isMaximized()) w.unmaximize();
    else w.maximize();
  });
  ipcMain.on('win:set-fullscreen', (e, on) => winOf(e)?.setFullScreen(!!on));
  handle('win:state', (e) => (winOf(e) ? windowState(winOf(e)) : {}));

  // Dossiers de la bibliothèque : choisis par une boîte de dialogue du système.
  handle('lib:add-folder', async (e) => {
    const res = await dialog.showOpenDialog(winOf(e), {
      title: 'Ajouter un dossier à la bibliothèque',
      defaultPath: app.getPath('music'),
      properties: ['openDirectory', 'multiSelections'],
    });
    if (res.canceled || !res.filePaths.length) return [];
    const folders = [...new Set([...(store.get('libraryFolders', []) || []), ...res.filePaths])];
    store.patch({ libraryFolders: folders });
    broadcast('settings:changed', { libraryFolders: folders });
    applySettingEffects({ libraryFolders: folders });
    return folders;
  });
  handle('lib:add-music-folder', () => {
    const folder = app.getPath('music');
    const folders = [...new Set([...(store.get('libraryFolders', []) || []), folder])];
    store.patch({ libraryFolders: folders });
    broadcast('settings:changed', { libraryFolders: folders });
    applySettingEffects({ libraryFolders: folders });
    return folders;
  });
  ipcMain.on('lib:reveal', (e, id) => {
    const file = library.trackPath(id);
    if (file) shell.showItemInFolder(file);
  });

  handle('stats:save-image', async (e, dataUrl, name) => {
    const match = /^data:image\/png;base64,([A-Za-z0-9+/=]+)$/.exec(String(dataUrl || ''));
    if (!match || match[1].length > 30 * 1024 * 1024) return false;
    const res = await dialog.showSaveDialog(winOf(e), {
      title: 'Enregistrer l’image',
      defaultPath: path.join(app.getPath('pictures'), `${String(name || 'control').replace(/[\\/:*?"<>|]/g, '_')}.png`),
      filters: [{ name: 'Image PNG', extensions: ['png'] }],
    });
    if (res.canceled || !res.filePath) return false;
    await fs.promises.writeFile(res.filePath, Buffer.from(match[1], 'base64'));
    return res.filePath;
  });

  handle('tags:pick-cover', async (e) => {
    try {
      const res = await dialog.showOpenDialog(winOf(e), {
        title: 'Choisir une pochette',
        defaultPath: app.getPath('pictures'),
        filters: [{ name: 'Images', extensions: ['jpg', 'jpeg', 'png', 'webp', 'bmp', 'gif'] }],
        properties: ['openFile'],
      });
      if (res.canceled || !res.filePaths[0]) return { ok: true, data: null };
      return { ok: true, data: await tags.stageCover({ path: res.filePaths[0] }) };
    } catch (err) {
      return { ok: false, message: String(err?.message || err) };
    }
  });

  handle('pl:import', async (e) => {
    const res = await dialog.showOpenDialog(winOf(e), {
      title: 'Importer une playlist',
      filters: [{ name: 'Playlist M3U', extensions: ['m3u', 'm3u8'] }],
      properties: ['openFile'],
    });
    if (res.canceled || !res.filePaths[0]) return null;
    return library.importM3U(res.filePaths[0]);
  });
  handle('pl:export', async (e, name, trackIds) => {
    const res = await dialog.showSaveDialog(winOf(e), {
      title: 'Exporter la playlist',
      defaultPath: path.join(app.getPath('music'), `${String(name).replace(/[\\/:*?"<>|]/g, '_')}.m3u8`),
      filters: [{ name: 'Playlist M3U', extensions: ['m3u8', 'm3u'] }],
    });
    if (res.canceled || !res.filePath) return null;
    return { file: res.filePath, count: library.exportM3U(res.filePath, trackIds) };
  });

  handle('dl:choose-cookies', async (e) => {
    const res = await dialog.showOpenDialog(winOf(e), {
      title: 'Fichier cookies.txt',
      properties: ['openFile'],
      filters: [{ name: 'cookies.txt', extensions: ['txt'] }, { name: 'Tous les fichiers', extensions: ['*'] }],
    });
    if (res.canceled || !res.filePaths[0]) return null;
    const dl = { ...(store.get('dl', {}) || {}), cookiesFile: res.filePaths[0] };
    store.patch({ dl });
    broadcast('settings:changed', { dl: { cookiesFile: res.filePaths[0] } });
    return res.filePaths[0];
  });
  handle('dl:choose-dir', async (e) => {
    const res = await dialog.showOpenDialog(winOf(e), {
      title: 'Dossier des téléchargements',
      defaultPath: store.get('downloadDir', '') || defaultDownloadDir(),
      properties: ['openDirectory', 'createDirectory'],
    });
    if (res.canceled || !res.filePaths[0]) return null;
    store.patch({ downloadDir: res.filePaths[0] });
    broadcast('settings:changed', { downloadDir: res.filePaths[0] });
    applySettingEffects({ downloadDir: res.filePaths[0] });
    return res.filePaths[0];
  });
  ipcMain.on('dl:open-dir', () => {
    const dir = store.get('downloadDir', '') || defaultDownloadDir();
    fs.mkdirSync(dir, { recursive: true });
    shell.openPath(dir);
  });

  handle('app:info', () => ({
    version: app.getVersion(),
    electron: process.versions.electron,
    chrome: process.versions.chrome,
    platform: process.platform,
    portable: !!PORTABLE_DIR,
    appimage: !!process.env.APPIMAGE,
    dataDir: app.getPath('userData'),
    musicDir: app.getPath('music'),
    downloadDir: store.get('downloadDir', '') || defaultDownloadDir(),
  }));
  ipcMain.on('app:open-data-dir', () => shell.openPath(app.getPath('userData')));
  ipcMain.on('app:open-external', (e, url) => {
    if (typeof url === 'string' && /^https:\/\//.test(url)) shell.openExternal(url);
  });
  ipcMain.on('app:quit', () => { quitting = true; app.quit(); });
}

// ─── Cycle de vie ────────────────────────────────────────────────────────────
app.on('second-instance', showMain);

app.whenReady().then(() => {
  if (!gotLock) return;
  const userData = app.getPath('userData');
  core = new Core({ dir: userData, coversDir: path.join(userData, 'covers'), emit: broadcast, defaultDownloadDir: defaultDownloadDir() });
  ({ store, library, tools, downloads, media, tags, stats, lyrics } = core);
  discord = new DiscordPresence({
    cacheFile: path.join(userData, 'discord-covers.json'),
    // Pochette intégrée au fichier (ou du dossier), ou image extraite d'une vidéo : déjà rangées par la bibliothèque.
    coverFile: async ({ kind, id }) => {
      if (kind === 'cover') {
        const file = path.join(library.coversDir, `${id}.jpg`);
        return fs.existsSync(file) ? file : null;
      }
      return library.videoThumb(id, tools.path('ffmpeg'));
    },
  });
  discord.on('status', (status) => broadcast('discord:status', status));
  updater = new Updater({
    version: app.getVersion(),
    edition: PORTABLE_DIR ? 'portable'
      : process.platform === 'win32' ? 'setup'
        : process.platform === 'linux' && process.env.APPIMAGE ? 'appimage' : 'manual',
    selfFile: process.env.PORTABLE_EXECUTABLE_FILE || process.env.APPIMAGE,
    dir: path.join(userData, 'updates'),
    fetch: (...args) => net.fetch(...args),
    token: () => (store.get('update', {}) || {}).token || '',
    redirect: redirectTarget,
    quit: () => { quitting = true; app.quit(); },
  });
  updater.on('state', (state) => broadcast('update:state', state));
  updater.cleanup();

  Menu.setApplicationMenu(null);
  registerProtocol();
  setupPermissions();
  setupIpc();
  createWindow();
  configureDiscord();
  scheduleUpdateChecks();

  library.scan(scanRoots());
  setupWatchers();
  tools.refreshVersions().then(() => {
    broadcast('tools:status', tools.status());
    const settings = store.get('dl', {}) || {};
    const last = store.get('lastToolUpdate', 0);
    if (settings.autoUpdate !== false && tools.path('ytdlp') && Date.now() - last > 24 * 3600 * 1000) {
      store.patch({ lastToolUpdate: Date.now() });
      tools.update().then((r) => broadcast('tools:updated', r));
    }
  });
});

app.on('before-quit', () => {
  quitting = true;
  downloads?.killAll();
  discord?.shutdown();
  store?.flush();
});

app.on('window-all-closed', () => app.quit());

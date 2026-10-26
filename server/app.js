'use strict';

// Les routes : pages, authentification, RPC, événements en direct et médias.

const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const crypto = require('node:crypto');
const pkg = require('../package.json');
const { frameOrigins, imageOrigins } = require('../src/main/media');
const { invoke, send: sendCommand } = require('./rpc');
const { httpError } = require('./auth');
const {
  parseCookies, cookieHeader, readJson, send, sendJson, sendBuffer, sendFile, safeJoin, MIME,
} = require('./http');

const RENDERER_DIR = path.resolve(__dirname, '../src/renderer');
const WEB_DIR = path.resolve(__dirname, '../src/web');
const COOKIE = 'control_session';
const MEDIA_CACHE = 'private, no-cache';

function createApp(hub, config) {
  const clientIp = (req) => {
    if (config.trustProxy) {
      const forwarded = String(req.headers['x-forwarded-for'] || '').split(',')[0].trim();
      if (forwarded) return forwarded;
    }
    return req.socket.remoteAddress || '';
  };
  const isHttps = (req) => !!req.socket.encrypted || (config.trustProxy && String(req.headers['x-forwarded-proto'] || '').split(',')[0].trim() === 'https');
  const secureCookie = (req) => (config.cookieSecure === 'auto' ? isHttps(req) : /^(1|true|yes|on)$/.test(config.cookieSecure));

  const sessionCookie = (req, token, maxAge) => cookieHeader(COOKIE, token, { maxAge, secure: secureCookie(req) });
  const userOf = (req) => hub.accounts.lookup(parseCookies(req.headers.cookie)[COOKIE]);

  // ─── Pages ─────────────────────────────────────────────────────────────────
  const pageCsp = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; font-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

  function appCsp() {
    const images = ['\'self\'', 'data:', 'blob:', 'https:', ...imageOrigins()].join(' ');
    return [
      "default-src 'self'",
      "script-src 'self'",
      "style-src 'self' 'unsafe-inline'",
      `img-src ${images}`,
      "media-src 'self' blob:",
      "font-src 'self'",
      "connect-src 'self'",
      "worker-src 'self'",
      "manifest-src 'self'",
      `frame-src ${frameOrigins().join(' ')}`,
      "object-src 'none'",
      "base-uri 'none'",
      "frame-ancestors 'none'",
    ].join('; ');
  }

  let indexTemplate = null;
  let indexMtime = 0;
  async function template() {
    const file = path.join(RENDERER_DIR, 'index.html');
    const { mtimeMs } = await fs.promises.stat(file);
    if (!indexTemplate || mtimeMs !== indexMtime) {
      indexTemplate = await fs.promises.readFile(file, 'utf8');
      indexMtime = mtimeMs;
    }
    return indexTemplate;
  }

  const forJson = (obj) => JSON.stringify(obj).replace(/</g, '\\u003c').replace(/\u2028/g, '\\u2028').replace(/\u2029/g, '\\u2029');

  /** La page de l'application, avec l'état de démarrage de ce compte (ce que le bureau obtient par IPC synchrone). */
  async function indexPage(user, entry) {
    const caps = {
      editFiles: user.role === 'admin',
      download: user.role === 'admin' || hub.settings.allowUserDownloads,
      manageServer: user.role === 'admin',
      installTools: user.role === 'admin',
    };
    const boot = {
      settings: entry.core.store.settings,
      env: { streamOverride: process.env.CONTROL_STREAM_BASE ? process.env.CONTROL_STREAM_BASE.replace(/\/$/, '') : null },
      user: { id: user.id, username: user.username, role: user.role },
      caps,
      server: { name: hub.settings.serverName },
      info: { version: pkg.version, platform: 'web', electron: null, chrome: null, portable: false, appimage: false, dataDir: '', musicDir: hub.settings.libraryFolders[0] || '', downloadDir: hub.settings.downloadDir },
      clientId: crypto.randomBytes(8).toString('hex'),
    };
    const head = [
      '<meta name="theme-color" content="#0c0c0f">',
      '<meta name="apple-mobile-web-app-capable" content="yes">',
      '<meta name="mobile-web-app-capable" content="yes">',
      '<meta name="apple-mobile-web-app-status-bar-style" content="black-translucent">',
      `<meta name="apple-mobile-web-app-title" content="${escapeAttr(hub.settings.serverName)}">`,
      '<link rel="manifest" href="/manifest.webmanifest">',
      '<link rel="apple-touch-icon" href="/web/icon-180.png">',
      `<script id="boot" type="application/json">${forJson(boot)}</script>`,
      '<script src="/web/shim.js"></script>',
    ].join('\n  ');
    return (await template())
      .replace('<html lang="fr"', `<html lang="${escapeAttr(entry.core.store.get('language', 'fr') === 'en' ? 'en' : 'fr')}"`)
      .replace('width=device-width, initial-scale=1', 'width=device-width, initial-scale=1, viewport-fit=cover')
      .replace('<title>control</title>', `<title>${escapeHtml(hub.settings.serverName)}</title>\n  ${head}`)
      .replace('</head>', '  <link rel="stylesheet" href="/web/app-web.css">\n</head>')
      .replace('</body>', '<script src="/web/app-web.js" type="module"></script>\n</body>');
  }

  const escapeHtml = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  const escapeAttr = escapeHtml;

  async function staticPage(req, res, name) {
    const html = await fs.promises.readFile(path.join(WEB_DIR, name), 'utf8');
    const page = html.replaceAll('{{SERVER_NAME}}', escapeHtml(hub.settings.serverName));
    return sendBuffer(req, res, 200, Buffer.from(page), 'text/html; charset=utf-8', { 'content-security-policy': pageCsp, 'cache-control': 'no-store' });
  }

  const redirect = (res, location) => send(res, 302, null, { location, 'cache-control': 'no-store' });
  const json = (req, res, status, data, headers) => sendJson(req, res, status, data, headers);

  function requireCsrf(req) {
    if (req.headers['x-control'] !== '1') throw httpError(403, 'csrf', 'Requête refusée.');
  }

  // ─── Authentification ──────────────────────────────────────────────────────
  async function login(req, res) {
    requireCsrf(req);
    const body = await readJson(req, 64 * 1024);
    const ip = clientIp(req);
    const name = String(body.username || '').slice(0, 64);
    const keys = [`ip:${ip}`, `user:${ip}:${name.toLowerCase()}`];
    const wait = Math.max(...keys.map((k) => (k.startsWith('ip:') ? hub.ipThrottle : hub.loginThrottle).wait(k)));
    if (wait > 0) return json(req, res, 429, { ok: false, error: { code: 'too_many', message: `Trop d’essais. Réessayez dans ${Math.ceil(wait / 60)} min.` } }, { 'retry-after': String(wait) });
    const user = await hub.accounts.check(name, String(body.password || ''));
    if (!user) {
      hub.loginThrottle.fail(keys[1]);
      hub.ipThrottle.fail(keys[0]);
      return json(req, res, 401, { ok: false, error: { code: 'invalid_credentials', message: 'Nom ou mot de passe incorrect.' } });
    }
    hub.loginThrottle.clear(keys[1]);
    const session = hub.accounts.open(user.id, { userAgent: req.headers['user-agent'], ip });
    return json(req, res, 200, { ok: true, user: { id: user.id, username: user.username, role: user.role } }, { 'set-cookie': sessionCookie(req, session.token, session.maxAge) });
  }

  async function register(req, res) {
    requireCsrf(req);
    if (!hub.settings.registrationEnabled || !hub.accounts.adminCount()) throw httpError(403, 'registration_disabled', 'Les inscriptions sont fermées.');
    const body = await readJson(req, 64 * 1024);
    const ip = clientIp(req);
    const wait = hub.ipThrottle.wait(`reg:${ip}`);
    if (wait > 0) throw httpError(429, 'too_many', 'Trop d’essais, réessayez plus tard.');
    hub.ipThrottle.fail(`reg:${ip}`);
    const user = await hub.accounts.create({ username: String(body.username || ''), password: String(body.password || ''), role: 'user' });
    const session = hub.accounts.open(user.id, { userAgent: req.headers['user-agent'], ip });
    return json(req, res, 200, { ok: true, user: { id: user.id, username: user.username, role: user.role } }, { 'set-cookie': sessionCookie(req, session.token, session.maxAge) });
  }

  let initializing = false;
  async function setupInit(req, res) {
    requireCsrf(req);
    if (hub.accounts.count() > 0 || initializing) throw httpError(403, 'already_initialized', 'Le serveur est déjà configuré.');
    // Deux requêtes simultanées ne doivent pas créer deux administrateurs : la création du compte est lente (scrypt).
    initializing = true;
    let user;
    try {
      const body = await readJson(req, 64 * 1024);
      if (config.setupToken && body.token !== config.setupToken) throw httpError(403, 'bad_token', 'Code d’installation incorrect.');
      user = await hub.accounts.create({ username: String(body.username || ''), password: String(body.password || ''), role: 'admin' });
      const patch = { serverName: body.serverName || 'control' };
      if (Array.isArray(body.libraryFolders) && body.libraryFolders.length) {
        const folders = [];
        for (const dir of body.libraryFolders.slice(0, 20)) {
          try {
            const resolved = path.resolve(String(dir));
            if ((await fs.promises.stat(resolved)).isDirectory()) folders.push(resolved);
          } catch { /* dossier absent : ignoré, l'administrateur pourra le corriger */ }
        }
        patch.libraryFolders = folders;
      }
      if (body.tmdbKey) patch.tmdbKey = String(body.tmdbKey);
      hub.patchSettings(patch);
    } finally {
      initializing = false;
    }
    const session = hub.accounts.open(user.id, { userAgent: req.headers['user-agent'], ip: clientIp(req) });
    return json(req, res, 200, { ok: true }, { 'set-cookie': sessionCookie(req, session.token, session.maxAge) });
  }

  // ─── Événements en direct (SSE) ────────────────────────────────────────────
  function events(req, res, user, url) {
    const clientId = String(url.searchParams.get('c') || '').replace(/[^a-z0-9]/gi, '').slice(0, 32) || crypto.randomBytes(6).toString('hex');
    const entry = hub.entry(user);
    res.writeHead(200, {
      'content-type': 'text/event-stream',
      'cache-control': 'no-cache, no-transform',
      connection: 'keep-alive',
      'x-accel-buffering': 'no',
      'x-content-type-options': 'nosniff',
    });
    res.write('retry: 2000\n\n');
    // Un client qui se reconnecte remplace son ancienne connexion.
    entry.clients.get(clientId)?.end();
    hub.addClient(entry, clientId, res);
    res.write('event: ready\ndata: null\n\n');
    req.on('close', () => hub.removeClient(entry, clientId, res));
  }

  // ─── Médias ────────────────────────────────────────────────────────────────
  async function media(req, res, user, pathname) {
    const entry = hub.entry(user);
    if (pathname.startsWith('/media/')) {
      const id = Number(pathname.slice(7));
      const file = Number.isInteger(id) ? entry.core.library.trackPath(id) : null;
      if (!file || !hub.isInsideRoots(file)) return send(res, 404, 'Not found', { 'content-type': 'text/plain' });
      return sendFile(req, res, file, { cache: MEDIA_CACHE });
    }
    if (pathname.startsWith('/cover/')) {
      const name = pathname.slice(7);
      if (!/^[0-9a-f]{20}(-s)?\.jpg$/.test(name)) return send(res, 404, 'Not found', { 'content-type': 'text/plain' });
      return sendFile(req, res, path.join(hub.coversDir, name), { type: 'image/jpeg', cache: 'private, max-age=31536000, immutable' });
    }
    const id = Number(pathname.slice(8));
    const file = Number.isInteger(id) ? await entry.core.library.videoThumb(id, hub.tools.path('ffmpeg')) : null;
    if (!file) return send(res, 404, 'Not found', { 'content-type': 'text/plain' });
    return sendFile(req, res, file, { type: 'image/jpeg', cache: 'private, max-age=3600' });
  }

  // ─── Fichiers de l'application ─────────────────────────────────────────────
  async function assets(req, res, pathname) {
    let root = RENDERER_DIR;
    let rel = pathname;
    if (pathname.startsWith('/web/')) {
      root = WEB_DIR;
      rel = pathname.slice(4);
    } else if (!/^\/(js|styles|assets)\//.test(pathname)) {
      return send(res, 404, 'Not found', { 'content-type': 'text/plain' });
    }
    const file = safeJoin(root, rel);
    if (!file) return send(res, 404, 'Not found', { 'content-type': 'text/plain' });
    const cache = /\.(woff2?|ttf|png|jpe?g|svg|ico)$/i.test(file) ? 'public, max-age=86400' : 'no-cache';
    return sendFile(req, res, file, { cache });
  }

  // ─── Routeur ───────────────────────────────────────────────────────────────
  async function route(req, res) {
    const url = new URL(req.url, 'http://localhost');
    const { pathname } = url;
    const method = req.method;
    const headers = { 'referrer-policy': 'same-origin', 'x-frame-options': 'DENY' };
    if (isHttps(req)) headers['strict-transport-security'] = 'max-age=15552000';
    for (const [k, v] of Object.entries(headers)) res.setHeader(k, v);

    if (pathname === '/api/health') return json(req, res, 200, { ok: true, name: 'control', version: pkg.version });

    // Premier lancement : tant qu'aucun compte n'existe, tout mène à l'assistant.
    const initialized = hub.accounts.count() > 0;
    if (pathname === '/api/setup/status') {
      return json(req, res, 200, { initialized, serverName: hub.settings.serverName, tokenRequired: !!config.setupToken, folders: hub.settings.libraryFolders, registrationEnabled: hub.settings.registrationEnabled });
    }
    if (pathname === '/api/setup/init' && method === 'POST') return setupInit(req, res);
    if (pathname === '/setup') return initialized ? redirect(res, '/') : staticPage(req, res, 'setup.html');

    if (pathname === '/api/auth/login' && method === 'POST') return login(req, res);
    if (pathname === '/api/auth/register' && method === 'POST') return register(req, res);
    if (pathname === '/api/auth/logout' && method === 'POST') {
      requireCsrf(req);
      hub.accounts.close(parseCookies(req.headers.cookie)[COOKIE]);
      return json(req, res, 200, { ok: true }, { 'set-cookie': sessionCookie(req, '', 0) });
    }
    if (pathname === '/manifest.webmanifest') {
      const manifest = JSON.parse(await fs.promises.readFile(path.join(WEB_DIR, 'manifest.webmanifest'), 'utf8'));
      manifest.name = hub.settings.serverName;
      manifest.short_name = hub.settings.serverName.slice(0, 12);
      return json(req, res, 200, manifest, { 'content-type': 'application/manifest+json', 'cache-control': 'no-cache' });
    }
    if (pathname === '/sw.js') return sendFile(req, res, path.join(WEB_DIR, 'sw.js'), { cache: 'no-cache', headers: { 'service-worker-allowed': '/' } });
    if (/^\/(js|styles|assets|web)\//.test(pathname) && (method === 'GET' || method === 'HEAD')) return assets(req, res, pathname);

    if (pathname === '/login') {
      if (!initialized) return redirect(res, '/setup');
      return userOf(req) ? redirect(res, '/') : staticPage(req, res, 'login.html');
    }

    // Tout le reste demande un compte connecté.
    const user = userOf(req);
    if (!user) {
      if (!initialized && pathname === '/') return redirect(res, '/setup');
      if (pathname === '/' || pathname === '/admin') return redirect(res, '/login');
      return json(req, res, 401, { ok: false, error: { code: 'unauthorized', message: 'Connexion requise.' } });
    }

    if (pathname === '/api/auth/me') return json(req, res, 200, { ok: true, user: { id: user.id, username: user.username, role: user.role } });
    if (pathname === '/' || pathname === '/index.html') {
      const entry = hub.entry(user);
      const html = await indexPage(user, entry);
      return sendBuffer(req, res, 200, Buffer.from(html), 'text/html; charset=utf-8', { 'content-security-policy': appCsp(), 'cache-control': 'no-store' });
    }
    if (pathname === '/admin') return user.role === 'admin' ? staticPage(req, res, 'admin.html') : redirect(res, '/');

    if (pathname === '/api/events' && method === 'GET') return events(req, res, user, url);
    if ((pathname.startsWith('/api/rpc/') || pathname.startsWith('/api/send/')) && method === 'POST') {
      requireCsrf(req);
      const send_ = pathname.startsWith('/api/send/');
      const channel = decodeURIComponent(pathname.slice(send_ ? 10 : 9));
      const body = await readJson(req);
      const args = Array.isArray(body.args) ? body.args : [];
      const clientId = String(req.headers['x-client'] || '').slice(0, 32);
      const entry = hub.entry(user);
      if (send_) {
        sendCommand(hub, entry, user, channel, args, clientId);
        return send(res, 204, null);
      }
      const result = await invoke(hub, entry, user, channel, args, clientId);
      return json(req, res, 200, { ok: true, result: result === undefined ? null : result });
    }
    if (/^\/(media|cover|vthumb)\//.test(pathname) && (method === 'GET' || method === 'HEAD')) return media(req, res, user, pathname);

    return json(req, res, 404, { ok: false, error: { code: 'not_found', message: 'Introuvable.' } });
  }

  const server = http.createServer((req, res) => {
    route(req, res).catch((err) => {
      if (res.headersSent) {
        res.destroy();
        return;
      }
      const status = err.status || 500;
      if (status >= 500) console.error(`[${req.method} ${req.url}]`, err);
      json(req, res, status, { ok: false, error: { code: err.code || 'error', message: status >= 500 ? 'Erreur du serveur.' : err.message } }).catch(() => res.destroy());
    });
  });
  // Les flux de lecture restent ouverts longtemps : pas de délai global sur les requêtes.
  server.requestTimeout = 0;
  server.headersTimeout = 30 * 1000;
  server.keepAliveTimeout = 65 * 1000;
  return server;
}

module.exports = { createApp, MIME };

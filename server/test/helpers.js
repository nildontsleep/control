'use strict';

// Outils communs aux tests du serveur : serveur de test, client HTTP à cookie, fichiers audio minuscules.

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { loadConfig } = require('../config');
const { Hub } = require('../hub');
const { createApp } = require('../app');

/** Un petit fichier WAV (silence) : de quoi être analysé et servi. */
function wav(file, seconds = 1) {
  const rate = 8000;
  const samples = rate * seconds;
  const buf = Buffer.alloc(44 + samples * 2);
  buf.write('RIFF', 0); buf.writeUInt32LE(36 + samples * 2, 4); buf.write('WAVEfmt ', 8);
  buf.writeUInt32LE(16, 16); buf.writeUInt16LE(1, 20); buf.writeUInt16LE(1, 22); buf.writeUInt32LE(rate, 24); buf.writeUInt32LE(rate * 2, 28);
  buf.writeUInt16LE(2, 32); buf.writeUInt16LE(16, 34); buf.write('data', 36); buf.writeUInt32LE(samples * 2, 40);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, buf);
}

async function boot(overrides = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'control-web-'));
  const music = path.join(root, 'music');
  wav(path.join(music, 'Artist A', 'Album 1', '01 - One.wav'));
  wav(path.join(music, 'Artist A', 'Album 1', '02 - Two.wav'), 2);
  wav(path.join(music, 'Artist B', '03 - Three.wav'));
  const config = { ...loadConfig({}), port: 0, host: '127.0.0.1', dataDir: path.join(root, 'data'), libraryFolders: [music], rescanMinutes: 1000, ...overrides };
  const hub = new Hub(config);
  if (!overrides.noCommon) hub.patchSettings({ libraryFolders: [music] }); // dossiers communs : tous les comptes voient cette musique
  const server = createApp(hub, config);
  hub.start();
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  const close = async () => {
    server.closeAllConnections?.();
    await new Promise((resolve) => server.close(resolve));
    hub.close();
    fs.rmSync(root, { recursive: true, force: true });
  };
  return { base, hub, music, root, close };
}

/** Client minimal : garde le cookie de session et envoie l'en-tête anti-CSRF. */
function client(base) {
  let cookie = '';
  const request = async (method, url, body, extra = {}) => {
    const res = await fetch(base + url, {
      method,
      redirect: 'manual',
      headers: { ...(body !== undefined ? { 'content-type': 'application/json' } : {}), 'x-control': '1', ...(cookie ? { cookie } : {}), ...extra },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const set = res.headers.get('set-cookie');
    if (set) cookie = set.split(';')[0].endsWith('=') ? '' : set.split(';')[0];
    return res;
  };
  const json = async (method, url, body, extra) => {
    const res = await request(method, url, body, extra);
    return { status: res.status, body: await res.json().catch(() => null), headers: res.headers };
  };
  return {
    request,
    json,
    rpc: async (channel, ...args) => {
      const r = await json('POST', `/api/rpc/${encodeURIComponent(channel)}`, { args });
      if (r.status !== 200) throw Object.assign(new Error(r.body?.error?.message || String(r.status)), { status: r.status });
      return r.body.result;
    },
    /** Appel dans la bibliothèque d'un autre compte (en-tête x-library). */
    rpcIn: async (libraryId, channel, ...args) => {
      const r = await json('POST', `/api/rpc/${encodeURIComponent(channel)}`, { args }, { 'x-library': libraryId });
      if (r.status !== 200) throw Object.assign(new Error(r.body?.error?.message || String(r.status)), { status: r.status, code: r.body?.error?.code });
      return r.body.result;
    },
    get cookie() { return cookie; },
  };
}

const until = async (fn, ms = 15000) => {
  const end = Date.now() + ms;
  for (;;) {
    const v = await fn();
    if (v) return v;
    if (Date.now() > end) throw new Error('délai dépassé');
    await new Promise((r) => setTimeout(r, 100));
  }
};

// ─── Mise à jour : faux GitHub, archives de fausses versions ────────────────────
const http = require('node:http');
const net = require('node:net');
const crypto = require('node:crypto');
const { collect } = require('../bundle');
const { pack } = require('../tar');

/** L'archive d'une version fictive : le code actuel, avec la version demandée (et au besoin un serveur cassé). */
function fakeBundle(version, { broken = false, engines, extra = [] } = {}) {
  const files = collect().map((f) => {
    if (f.name === 'package.json') {
      const pkg = JSON.parse(f.data.toString('utf8'));
      pkg.version = version;
      if (engines) pkg.engines = { node: engines };
      return { ...f, data: Buffer.from(JSON.stringify(pkg, null, 2)) };
    }
    if (broken && f.name === 'server/index.js') return { ...f, data: Buffer.from("throw new Error('version cassée (essai)');\n") };
    return f;
  });
  const data = pack([...files, ...extra]);
  return { version, data, sha256: crypto.createHash('sha256').update(data).digest('hex') };
}

/** Un GitHub de poche : `releases/latest` et le téléchargement de l'archive. `release` est modifiable à tout moment. */
async function mockGithub() {
  const state = { release: null, hits: [] };
  const server = http.createServer((req, res) => {
    state.hits.push(req.url);
    const rel = state.release;
    if (req.url === '/releases/latest' && rel) {
      const port = server.address().port;
      res.writeHead(200, { 'content-type': 'application/json' });
      return res.end(JSON.stringify({
        tag_name: `v${rel.version}`,
        name: `control ${rel.version}`,
        body: 'Nouveautés de la version.',
        html_url: `http://127.0.0.1:${port}/release`,
        draft: false,
        prerelease: false,
        assets: rel.noAsset ? [] : [{
          name: `control-web-${rel.version}.tgz`,
          size: rel.data.length,
          browser_download_url: `http://127.0.0.1:${port}/asset/${rel.version}.tgz`,
          digest: rel.noDigest ? null : `sha256:${rel.digestOverride || rel.sha256}`,
        }],
      }));
    }
    if (rel && req.url === `/asset/${rel.version}.tgz`) {
      res.writeHead(200, { 'content-type': 'application/gzip', 'content-length': rel.data.length });
      return res.end(rel.data);
    }
    res.writeHead(404).end();
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  return {
    url: `http://127.0.0.1:${server.address().port}/releases/latest`,
    state,
    publish: (bundle, flags = {}) => { state.release = { ...bundle, ...flags }; },
    close: () => new Promise((resolve) => { server.closeAllConnections?.(); server.close(resolve); }),
  };
}

const freePort = () => new Promise((resolve) => {
  const s = net.createServer().listen(0, '127.0.0.1', () => { const { port } = s.address(); s.close(() => resolve(port)); });
});

module.exports = { wav, boot, client, until, fakeBundle, mockGithub, freePort };

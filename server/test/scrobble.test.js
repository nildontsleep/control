'use strict';

// Scrobbling : règle de comptage, ListenBrainz, Last.fm (signature), file de reprise, intégration dans le serveur.

const test = require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { DatabaseSync } = require('node:sqlite');
const { Scrobbler, counts } = require('../../src/main/scrobble');
const { boot, client, until } = require('./helpers');

const md5 = (s) => crypto.createHash('md5').update(s, 'utf8').digest('hex');

/** Une bibliothèque minuscule en mémoire, de quoi décrire un morceau. */
function library() {
  const db = new DatabaseSync(':memory:');
  db.exec('CREATE TABLE tracks (id INTEGER PRIMARY KEY, title TEXT, artist TEXT, album TEXT, album_artist TEXT, track_no INTEGER, duration REAL, kind TEXT)');
  db.prepare("INSERT INTO tracks VALUES (1, 'Lune', 'Aya', 'Nuits', 'Aya', 1, 200, 'audio'), (2, 'Clip', 'Aya', '', '', NULL, 100, 'video'), (3, 'Sans artiste', '', '', '', NULL, 100, 'audio')").run();
  return { db };
}

/** Un faux réseau : enregistre les appels et répond selon le service. */
function network(handler) {
  const calls = [];
  const fetch = async (url, init = {}) => {
    const call = { url: String(url), method: init.method || 'GET', headers: init.headers || {}, body: init.body instanceof URLSearchParams ? Object.fromEntries(init.body) : init.body ? JSON.parse(init.body) : null };
    calls.push(call);
    const out = await handler(call);
    return { ok: out.status === undefined || out.status < 400, status: out.status || 200, headers: { get: (k) => out.headers?.[k] ?? null }, json: async () => out.body ?? {} };
  };
  return { fetch, calls };
}

const tmp = () => fs.mkdtempSync(path.join(os.tmpdir(), 'control-scrobble-'));

test('une écoute compte : 30 s de morceau au moins, la moitié écoutée ou 4 minutes', () => {
  assert.equal(counts(200, 100), true);
  assert.equal(counts(200, 99), false);
  assert.equal(counts(1000, 240), true, 'plafonné à 4 minutes');
  assert.equal(counts(1000, 239), false);
  assert.equal(counts(29, 29), false, 'trop court');
  assert.equal(counts(0, 50), false);
});

test('ListenBrainz : jeton vérifié, en écoute, écoute envoyée avec les bonnes informations', async () => {
  const dir = tmp();
  const net = network((c) => (c.url.endsWith('/validate-token') ? { body: { valid: true, user_name: 'nil' } } : { body: { status: 'ok' } }));
  const events = [];
  const s = new Scrobbler({ dir, library: library(), emit: (ch, p) => events.push([ch, p.pending]), fetch: net.fetch });
  await assert.rejects(s.connectListenBrainz('x', null), /invalide/);
  const status = await s.connectListenBrainz('abcdef12345-token', null);
  assert.deepEqual([status.listenbrainz.connected, status.listenbrainz.user], [true, 'nil']);
  assert.equal(net.calls[0].headers.authorization, 'Token abcdef12345-token');
  assert.equal(fs.statSync(path.join(dir, 'scrobble.json')).mode & 0o777, 0o600);

  s.started(7, 1);
  s.started(8, 2); // une vidéo : ignorée
  s.started(9, 3); // sans artiste : ignoré
  await until(() => net.calls.some((c) => c.body?.listen_type === 'playing_now'));
  const now = net.calls.find((c) => c.body?.listen_type === 'playing_now').body.payload[0].track_metadata;
  assert.deepEqual([now.artist_name, now.track_name, now.release_name, now.additional_info.duration_ms], ['Aya', 'Lune', 'Nuits', 200000]);
  assert.equal(net.calls.filter((c) => c.body?.listen_type === 'playing_now').length, 1);

  s.ended(7, 50); // trop court : pas d'écoute
  assert.equal(s.status().pending, 0);
  s.started(10, 1);
  s.ended(10, 150);
  await until(() => net.calls.some((c) => c.body?.listen_type === 'single'));
  const single = net.calls.find((c) => c.body?.listen_type === 'single').body.payload[0];
  assert.equal(single.track_metadata.track_name, 'Lune');
  assert.ok(single.listened_at > 1700000000);
  await until(() => s.status().pending === 0);
  s.close();
});

test('Last.fm : appels signés (md5), autorisation en deux temps, écoute envoyée', async () => {
  const dir = tmp();
  const net = network((c) => {
    if (c.body.method === 'auth.getToken') return { body: { token: 'TOKEN1' } };
    if (c.body.method === 'auth.getSession') return { body: { session: { name: 'nil', key: 'SESSIONKEY' } } };
    return { body: { scrobbles: {} } };
  });
  const s = new Scrobbler({ dir, library: library(), emit() {}, keys: () => ({ key: 'APIKEY', secret: 'SECRET' }), fetch: net.fetch });
  const start = await s.lastfmStart();
  assert.equal(start.token, 'TOKEN1');
  assert.match(start.url, /last\.fm\/api\/auth\/\?api_key=APIKEY&token=TOKEN1/);
  const sig = (call) => {
    const { api_sig: got, format, ...rest } = call.body;
    assert.equal(format, 'json');
    return got === md5(Object.keys(rest).sort().map((k) => `${k}${rest[k]}`).join('') + 'SECRET');
  };
  assert.equal(sig(net.calls[0]), true, 'signature de auth.getToken');
  const status = await s.lastfmFinish('TOKEN1');
  assert.deepEqual([status.lastfm.connected, status.lastfm.user], [true, 'nil']);
  assert.equal(sig(net.calls[1]), true, 'signature de auth.getSession');

  s.started(1, 1);
  await until(() => net.calls.some((c) => c.body.method === 'track.updateNowPlaying'));
  s.ended(1, 120);
  await until(() => net.calls.some((c) => c.body.method === 'track.scrobble'));
  const scrobble = net.calls.find((c) => c.body.method === 'track.scrobble');
  assert.deepEqual([scrobble.body['artist[0]'], scrobble.body['track[0]'], scrobble.body['album[0]'], scrobble.body.sk], ['Aya', 'Lune', 'Nuits', 'SESSIONKEY']);
  assert.equal(sig(scrobble), true);
  s.close();
});

test('file de reprise : réseau coupé = écoute gardée puis renvoyée ; session refusée = abandon et reconnexion demandée', async () => {
  const dir = tmp();
  let up = false;
  let refused = false;
  const net = network((c) => {
    if (c.url.endsWith('/validate-token')) return { body: { valid: true, user_name: 'nil' } };
    if (refused) return { status: 401 };
    if (!up) throw new Error('réseau coupé');
    return { body: { status: 'ok' } };
  });
  let s = new Scrobbler({ dir, library: library(), emit() {}, fetch: net.fetch });
  await s.connectListenBrainz('abcdef12345-token', null);
  s.started(1, 1);
  s.ended(1, 150);
  await until(() => net.calls.some((c) => c.body?.listen_type === 'single'));
  await new Promise((r) => setTimeout(r, 50));
  assert.equal(s.status().pending, 1, 'l’écoute attend le retour du réseau');
  s.close();

  // Après un redémarrage : la file est relue, et envoyée dès que le réseau revient.
  up = true;
  s = new Scrobbler({ dir, library: library(), emit() {}, fetch: net.fetch });
  assert.equal(s.status().pending, 1);
  await s.flush();
  assert.equal(s.status().pending, 0);

  // Jeton refusé : l'écoute n'est pas gardée indéfiniment, l'état demande de se reconnecter.
  refused = true;
  s.started(2, 1);
  s.ended(2, 150);
  await until(() => s.status().listenbrainz.error);
  assert.match(s.status().listenbrainz.error, /refusé/);
  assert.equal(s.status().pending, 0);
  assert.equal(s.disconnect('listenbrainz').listenbrainz.connected, false);
  s.close();
});

test('serveur : les écoutes comptées par le lecteur partent vers le service du compte, adresse personnalisée publique seulement', async (t) => {
  const s = await boot({ noCommon: true });
  t.after(s.close);
  const admin = client(s.base);
  await admin.json('POST', '/api/setup/init', { username: 'nil', password: 'long enough password', libraryFolders: [s.music] });
  const tracks = await until(async () => { const l = await admin.rpc('lib:tracks'); return l.length === 3 ? l : null; });
  const user = s.hub.accounts.list()[0];
  const core = s.hub.entry(user).core;
  core.db.prepare("UPDATE tracks SET artist = 'Aya' WHERE id = ?").run(tracks[0].id); // les fichiers de test n'ont pas de tags
  const scrobbler = core.scrobbler;
  const net = network((c) => (c.url.endsWith('/validate-token') ? { body: { valid: true, user_name: 'nil' } } : { body: { status: 'ok' } }));
  scrobbler.fetch = net.fetch;

  assert.deepEqual(await admin.rpc('scrobble:status'), { listenbrainz: { connected: false, user: null, url: null, error: null }, lastfm: { connected: false, user: null, error: null, configured: false, keysFromServer: false }, pending: 0 });
  const bad = await admin.rpc('scrobble:connect-listenbrainz', 'abcdef12345-token', 'http://127.0.0.1:9/');
  assert.equal(bad.ok, false);
  assert.match(bad.message, /réseau du serveur/);
  assert.equal((await admin.rpc('scrobble:connect-listenbrainz', 'abcdef12345-token', null)).ok, true);
  const lastfm = await admin.rpc('scrobble:lastfm-start');
  assert.equal(lastfm.ok, false, 'sans clés d’API : refusé');

  // Lecture d'un morceau de 1 s, écouté en entier : trop court pour compter (moins de 30 s), mais « en écoute » part.
  const playId = await admin.rpc('lib:play-start', tracks[0].id, 1);
  await admin.json('POST', '/api/send/lib%3Aplay-end', { args: [playId, 1, true] });
  await until(() => net.calls.some((c) => c.body?.listen_type === 'playing_now'));
  assert.equal(net.calls.some((c) => c.body?.listen_type === 'single'), false);

  // L'administrateur règle l'application Last.fm : disponible pour les comptes, sans que le secret revienne.
  await admin.rpc('srv:settings', { lastfmKey: 'APIKEY', lastfmSecret: 'SECRET' });
  const shown = await admin.rpc('srv:settings:get');
  assert.deepEqual([shown.lastfmKey, shown.lastfmSecret, shown.hasLastfmSecret], ['APIKEY', undefined, true]);
  assert.equal((await admin.rpc('scrobble:status')).lastfm.configured, true);

  // Les jetons restent dans un fichier à part, jamais dans les réglages envoyés à la page.
  const page = await (await admin.request('GET', '/')).text();
  assert.ok(!page.includes('abcdef12345-token'));
  assert.ok(fs.existsSync(path.join(s.root, 'data', 'users', user.id, 'scrobble.json')));
});

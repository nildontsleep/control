'use strict';

// Tests du serveur web : démarrage, comptes, droits, médias, événements. `node --test server/test`

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const http = require('node:http');
const { wav, boot, client, until } = require('./helpers');

test('premier lancement : tout mène à l’assistant, qui crée l’administrateur une seule fois', async (t) => {
  const s = await boot();
  t.after(s.close);
  const c = client(s.base);
  assert.equal((await c.json('GET', '/api/setup/status')).body.initialized, false);
  assert.equal((await c.request('GET', '/')).headers.get('location'), '/setup');
  assert.equal((await c.request('GET', '/setup')).status, 200);

  const weak = await c.json('POST', '/api/setup/init', { username: 'nil', password: 'court' });
  assert.equal(weak.status, 400);
  const bad = await c.json('POST', '/api/setup/init', { username: 'x', password: 'long enough password' });
  assert.equal(bad.status, 400);
  const ok = await c.json('POST', '/api/setup/init', { serverName: 'Mon serveur', username: 'nil', password: 'long enough password' });
  assert.equal(ok.status, 200);
  assert.match(ok.headers.get('set-cookie'), /control_session=.+HttpOnly/);
  assert.match(ok.headers.get('set-cookie'), /SameSite=Lax/);
  assert.equal((await c.json('POST', '/api/setup/init', { username: 'other', password: 'long enough password' })).status, 403);
  assert.equal((await c.request('GET', '/')).status, 200);
  assert.equal((await c.request('GET', '/setup')).headers.get('location'), '/');
});

test('code d’installation exigé quand il est défini', async (t) => {
  const s = await boot({ setupToken: 'sesame' });
  t.after(s.close);
  const c = client(s.base);
  assert.equal((await c.json('POST', '/api/setup/init', { username: 'nil', password: 'long enough password' })).status, 403);
  assert.equal((await c.json('POST', '/api/setup/init', { username: 'nil', password: 'long enough password', token: 'sesame' })).status, 200);
});

test('connexion : mauvais mot de passe, limitation des essais, déconnexion', async (t) => {
  const s = await boot();
  t.after(s.close);
  const admin = client(s.base);
  await admin.json('POST', '/api/setup/init', { username: 'nil', password: 'long enough password' });

  const c = client(s.base);
  assert.equal((await c.json('POST', '/api/rpc/lib:tracks', { args: [] })).status, 401);
  assert.equal((await c.request('GET', '/')).headers.get('location'), '/login');
  assert.equal((await c.json('POST', '/api/auth/login', { username: 'nil', password: 'wrong password' })).status, 401);
  const good = await c.json('POST', '/api/auth/login', { username: 'NIL', password: 'long enough password' }); // le nom ne dépend pas de la casse
  assert.equal(good.status, 200);
  assert.equal((await c.json('GET', '/api/auth/me')).body.user.username, 'nil');
  await c.json('POST', '/api/auth/logout', {});
  assert.equal((await c.json('GET', '/api/auth/me')).status, 401);

  const attacker = client(s.base);
  for (let i = 0; i < 8; i++) await attacker.json('POST', '/api/auth/login', { username: 'nil', password: `guess ${i}` });
  const blocked = await attacker.json('POST', '/api/auth/login', { username: 'nil', password: 'long enough password' });
  assert.equal(blocked.status, 429);
  assert.ok(Number(blocked.headers.get('retry-after')) > 0);
});

test('les requêtes qui modifient quelque chose exigent l’en-tête anti-CSRF', async (t) => {
  const s = await boot();
  t.after(s.close);
  const c = client(s.base);
  await c.json('POST', '/api/setup/init', { username: 'nil', password: 'long enough password' });
  const res = await fetch(`${s.base}/api/rpc/lib:scan`, { method: 'POST', headers: { 'content-type': 'application/json', cookie: c.cookie }, body: '{"args":[]}' });
  assert.equal(res.status, 403);
});

test('bibliothèque, médias avec Range, pochettes et page d’accueil', async (t) => {
  const s = await boot();
  t.after(s.close);
  const c = client(s.base);
  await c.json('POST', '/api/setup/init', { username: 'nil', password: 'long enough password' });

  const tracks = await until(async () => {
    const list = await c.rpc('lib:tracks');
    return list.length === 3 ? list : null;
  });
  assert.deepEqual(tracks.map((x) => x.title).sort(), ['01 - One', '02 - Two', '03 - Three']);

  const id = tracks[0].id;
  const full = await c.request('GET', `/media/${id}`);
  assert.equal(full.status, 200);
  assert.equal(full.headers.get('content-type'), 'audio/wav');
  assert.equal(full.headers.get('accept-ranges'), 'bytes');
  const size = Number(full.headers.get('content-length'));
  assert.ok(size > 1000);
  const part = await c.request('GET', `/media/${id}`, undefined, { range: 'bytes=10-109' });
  assert.equal(part.status, 206);
  assert.equal(part.headers.get('content-range'), `bytes 10-109/${size}`);
  assert.equal((await part.arrayBuffer()).byteLength, 100);
  const tail = await c.request('GET', `/media/${id}`, undefined, { range: 'bytes=-50' });
  assert.equal((await tail.arrayBuffer()).byteLength, 50);
  assert.equal((await c.request('GET', `/media/${id}`, undefined, { range: `bytes=${size + 5}-` })).status, 416);
  assert.equal((await c.request('GET', '/media/99999')).status, 404);
  assert.equal((await c.request('GET', '/media/abc')).status, 404);
  assert.equal((await c.request('GET', '/cover/../../etc/passwd')).status, 404);

  const page = await (await c.request('GET', '/')).text();
  assert.match(page, /<script id="boot" type="application\/json">/);
  assert.match(page, /\/web\/shim\.js/);
  const boot_ = JSON.parse(/<script id="boot" type="application\/json">(.*?)<\/script>/s.exec(page)[1]);
  assert.equal(boot_.user.username, 'nil');
  assert.equal(boot_.caps.manageServer, true);
  assert.ok(boot_.settings.commonFolders.length);

  // Jamais de fichier hors des dossiers de l'application ou de la bibliothèque.
  assert.equal((await c.request('GET', '/js/../../package.json')).status, 404);
  assert.equal((await c.request('GET', '/js/%2e%2e/%2e%2e/package.json')).status, 404);
  assert.equal((await c.request('GET', '/web/../../server/config.js')).status, 404);
  assert.equal((await c.request('GET', '/js/app.js')).status, 200);
});

test('chaque compte a sa propre écoute, et ses droits', async (t) => {
  const s = await boot();
  t.after(s.close);
  const admin = client(s.base);
  await admin.json('POST', '/api/setup/init', { username: 'nil', password: 'long enough password' });
  const tracks = await until(async () => {
    const list = await admin.rpc('lib:tracks');
    return list.length === 3 ? list : null;
  });
  const playId = await admin.rpc('lib:play-start', tracks[0].id, 1);
  await admin.json('POST', '/api/send/lib%3Aplay-end', { args: [playId, 40, true] });
  const created = await admin.rpc('srv:users:create', { username: 'ami', password: 'another password' });
  assert.equal(created.role, 'user');
  assert.equal((await admin.json('POST', '/api/rpc/srv:users:create', { args: [{ username: 'ami', password: 'another password' }] })).status, 409);

  const ami = client(s.base);
  assert.equal((await ami.json('POST', '/api/auth/login', { username: 'ami', password: 'another password' })).status, 200);
  // Sa bibliothèque est copiée de celle de l'administrateur, sans ses écoutes.
  const mine = await until(async () => {
    const list = await ami.rpc('lib:tracks');
    return list.length === 3 ? list : null;
  });
  assert.equal(mine.every((x) => x.playCount === 0), true);
  assert.equal((await admin.rpc('lib:tracks')).find((x) => x.id === tracks[0].id).playCount, 1);
  const today = new Date().toISOString().slice(0, 10);
  assert.equal((await ami.rpc('stats:get', { from: today, to: today })).data.music.plays, 0);
  assert.equal((await admin.rpc('stats:get', { from: today, to: today })).data.music.plays, 1);

  for (const [channel, args] of [['tags:write', [[]]], ['tools:install', []], ['srv:users:list', []], ['srv:add-folder', ['/tmp']], ['lib:remove-folder', ['/x']], ['lyrics:save-file', [mine[0].id]], ['lib:remove-tracks', [[mine[0].id], true]], ['dl:add', [{}]], ['lib:add-paths', [['/etc/passwd']]]]) {
    await assert.rejects(ami.rpc(channel, ...args), (err) => err.status === 403, channel);
  }
  // Ce qui reste permis : ses propres playlists et réglages.
  const pl = await ami.rpc('pl:create', 'Mes favoris', 'manual', null, [mine[0].id]);
  assert.ok(pl > 0);
  assert.equal((await admin.rpc('lib:playlists')).playlists.length, 0);

  // Les réglages du serveur ne passent pas par les réglages d'un compte ordinaire.
  await ami.json('POST', '/api/send/settings%3Aset', { args: [{ libraryFolders: ['/etc'], theme: 'light' }] });
  const page = await (await ami.request('GET', '/')).text();
  const boot_ = JSON.parse(/<script id="boot" type="application\/json">(.*?)<\/script>/s.exec(page)[1]);
  assert.equal(boot_.settings.theme, 'light');
  assert.deepEqual(boot_.settings.commonFolders, [s.music]);
  assert.deepEqual(boot_.settings.libraryFolders, []); // les dossiers propres au compte ne se règlent pas par les réglages
  assert.equal(boot_.caps.editFiles, false);

  // Compte désactivé : sa session est coupée.
  await admin.rpc('srv:users:update', created.id, { disabled: true });
  assert.equal((await ami.json('GET', '/api/auth/me')).status, 401);
  assert.equal((await ami.json('POST', '/api/auth/login', { username: 'ami', password: 'another password' })).status, 401);
  // Le dernier administrateur ne peut ni se rétrograder ni se supprimer.
  const me = (await admin.json('GET', '/api/auth/me')).body.user;
  await assert.rejects(admin.rpc('srv:users:update', me.id, { role: 'user' }), (err) => err.status === 409);
  await assert.rejects(admin.rpc('srv:users:delete', me.id), (err) => err.status === 409);
});

test('inscription : fermée par défaut, ouverte par l’administrateur', async (t) => {
  const s = await boot();
  t.after(s.close);
  const admin = client(s.base);
  await admin.json('POST', '/api/setup/init', { username: 'nil', password: 'long enough password' });
  const anon = client(s.base);
  assert.equal((await anon.json('POST', '/api/auth/register', { username: 'newbie', password: 'some password 1' })).status, 403);
  await admin.rpc('srv:settings', { registrationEnabled: true });
  const ok = await anon.json('POST', '/api/auth/register', { username: 'newbie', password: 'some password 1' });
  assert.equal(ok.status, 200);
  assert.equal(ok.body.user.role, 'user');
  assert.equal((await anon.json('POST', '/api/auth/register', { username: 'newbie', password: 'some password 1' })).status, 409);
});

test('événements en direct (SSE) : analyse de la bibliothèque et réglages', async (t) => {
  const s = await boot();
  t.after(s.close);
  const c = client(s.base);
  await c.json('POST', '/api/setup/init', { username: 'nil', password: 'long enough password' });
  await until(async () => (await c.rpc('lib:tracks')).length === 3);

  const events = [];
  const req = http.get(`${s.base}/api/events?c=abc123`, { headers: { cookie: c.cookie } }, (res) => {
    assert.equal(res.headers['content-type'], 'text/event-stream');
    let buf = '';
    res.on('data', (d) => {
      buf += d;
      let i;
      while ((i = buf.indexOf('\n\n')) >= 0) {
        const frame = buf.slice(0, i);
        buf = buf.slice(i + 2);
        const name = /^event: (.+)$/m.exec(frame)?.[1];
        if (name) events.push({ name, data: JSON.parse(/^data: (.*)$/m.exec(frame)[1]) });
      }
    });
  });
  t.after(() => req.destroy());
  await until(() => events.some((e) => e.name === 'ready'));

  // L'auteur d'un réglage ne reçoit pas son propre écho ; un autre onglet, si.
  await fetch(`${s.base}/api/send/settings%3Aset`, { method: 'POST', headers: { 'content-type': 'application/json', 'x-control': '1', 'x-client': 'abc123', cookie: c.cookie }, body: JSON.stringify({ args: [{ volume: 0.3 }] }) });
  await fetch(`${s.base}/api/send/settings%3Aset`, { method: 'POST', headers: { 'content-type': 'application/json', 'x-control': '1', 'x-client': 'autre', cookie: c.cookie }, body: JSON.stringify({ args: [{ volume: 0.6 }] }) });
  await until(() => events.some((e) => e.name === 'settings:changed' && e.data.volume === 0.6));
  assert.equal(events.some((e) => e.name === 'settings:changed' && e.data.volume === 0.3), false);

  // Une nouvelle musique apparaît dans les dossiers : la bibliothèque le signale.
  wav(path.join(s.music, 'Artist C', '04 - Four.wav'));
  await c.rpc('lib:scan');
  await until(() => events.some((e) => e.name === 'lib:changed'));
  assert.equal((await c.rpc('lib:tracks')).length, 4);
});

test('dossiers du serveur : ajout, retrait et navigation réservés à l’administrateur', async (t) => {
  const s = await boot();
  t.after(s.close);
  const c = client(s.base);
  await c.json('POST', '/api/setup/init', { username: 'nil', password: 'long enough password' });
  const extra = path.join(s.root, 'more');
  wav(path.join(extra, 'Artist Z', '09 - Nine.wav'));
  await assert.rejects(c.rpc('srv:add-folder', path.join(s.root, 'absent')), (err) => err.status === 400);
  await assert.rejects(c.rpc('srv:add-folder', path.join(extra, 'Artist Z', '09 - Nine.wav')), (err) => err.status === 400);
  const folders = await c.rpc('srv:add-folder', extra);
  assert.equal(folders.length, 2);
  await until(async () => (await c.rpc('lib:tracks')).length === 4);
  const listing = await c.rpc('srv:browse', s.root);
  assert.ok(listing.dirs.includes('music') && listing.dirs.includes('more'));
  await c.rpc('srv:remove-folder', extra);
  await until(async () => (await c.rpc('lib:tracks')).length === 3);
});

test('deux créations simultanées du premier compte : une seule réussit', async (t) => {
  const s = await boot();
  t.after(s.close);
  const a = client(s.base);
  const b = client(s.base);
  const [x, y] = await Promise.all([
    a.json('POST', '/api/setup/init', { username: 'first', password: 'long enough password' }),
    b.json('POST', '/api/setup/init', { username: 'second', password: 'long enough password' }),
  ]);
  assert.deepEqual([x.status, y.status].sort(), [200, 403]);
  assert.equal(s.hub.accounts.count(), 1);
});

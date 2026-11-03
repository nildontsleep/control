'use strict';

// Envoi de musique depuis le navigateur : nom nettoyé, dossier du compte, médias seulement, droits, taille.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { cleanSegment, cleanDir } = require('../upload');
const { boot, client, until, wav } = require('./helpers');

const PASSWORD = 'long enough password';

test('noms et dossiers : chemins, caractères interdits et noms réservés neutralisés', () => {
  assert.equal(cleanSegment('01 - Titre: Remix?.mp3', { keepExt: true }), '01 - Titre_ Remix_.mp3');
  assert.equal(cleanSegment('../../etc/passwd.mp3'), '_.._etc_passwd.mp3');
  assert.equal(cleanSegment('  .caché.mp3  '), 'caché.mp3');
  assert.equal(cleanSegment('CON.mp3'), null);
  assert.equal(cleanSegment('..'), null);
  assert.equal(cleanSegment(''), null);
  assert.ok(cleanSegment(`${'a'.repeat(300)}.flac`, { keepExt: true }).endsWith('.flac'));
  assert.ok(cleanSegment(`${'a'.repeat(300)}.flac`, { keepExt: true }).length <= 180);
  assert.deepEqual(cleanDir('Album/../Disc 1/./x'), ['Album', 'Disc 1', 'x'].slice(0, 3));
  assert.deepEqual(cleanDir('a/b/c/d/e/f'), ['a', 'b', 'c', 'd']);
  assert.deepEqual(cleanDir('../..'), []);
});

test('envoi : le fichier arrive dans le dossier du compte et entre dans sa bibliothèque', async (t) => {
  const s = await boot({ noCommon: true });
  t.after(s.close);
  const admin = client(s.base);
  await admin.json('POST', '/api/setup/init', { username: 'nil', password: PASSWORD });
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'control-up-'));
  t.after(() => fs.rmSync(tmp, { recursive: true, force: true }));
  wav(path.join(tmp, 'a.wav'), 2);
  const bytes = fs.readFileSync(path.join(tmp, 'a.wav'));
  const send = (name, body = bytes, { dir, headers = {}, who = admin } = {}) => fetch(`${s.base}/api/upload?name=${encodeURIComponent(name)}${dir ? `&dir=${encodeURIComponent(dir)}` : ''}`, {
    method: 'POST', headers: { 'x-control': '1', cookie: who.cookie, 'content-type': 'application/octet-stream', ...headers }, body,
  });

  assert.equal((await fetch(`${s.base}/api/upload?name=x.wav`, { method: 'POST', body: bytes })).status, 401, 'connexion requise');
  assert.equal((await fetch(`${s.base}/api/upload?name=x.wav`, { method: 'POST', headers: { cookie: admin.cookie }, body: bytes })).status, 403, 'en-tête anti-CSRF requis');

  const res = await send('Mon titre: essai.wav', bytes, { dir: 'Album/../Disque 1' });
  assert.equal(res.status, 200);
  const body = await res.json();
  assert.equal(body.result.name, 'Mon titre_ essai.wav');
  assert.equal(body.result.trackIds.length, 1);
  const user = s.hub.accounts.list()[0];
  const expected = path.join(s.hub.downloadDirFor(user), 'Envois', 'Album', 'Disque 1', 'Mon titre_ essai.wav'); // « .. » est écarté, pas suivi
  assert.ok(fs.existsSync(expected), expected);
  assert.deepEqual(fs.readdirSync(path.dirname(expected)).filter((n) => n.endsWith('.part')), [], 'pas de fichier partiel');
  const tracks = await until(async () => { const l = await admin.rpc('lib:tracks'); return l.length === 1 ? l : null; });
  assert.equal(tracks[0].path, expected);

  // Le même nom ne remplace rien.
  const again = await (await send('Mon titre: essai.wav', bytes, { dir: 'Album/../Disque 1' })).json();
  assert.equal(again.result.name, 'Mon titre_ essai (2).wav');
  assert.equal((await admin.rpc('lib:tracks')).length, 2);

  // Refus : autre type de fichier, contenu qui n'est pas un média, fichier vide, trop gros.
  assert.equal((await send('script.sh', Buffer.from('echo hi'))).status, 415);
  const fake = await send('faux.mp3', Buffer.from('ceci n’est pas de la musique'));
  assert.equal(fake.status, 415);
  assert.ok(!fs.existsSync(path.join(s.hub.downloadDirFor(user), 'Envois', 'faux.mp3')), 'le faux fichier est supprimé');
  assert.equal((await send('vide.wav', Buffer.alloc(0))).status, 400);
  await admin.rpc('srv:settings', { uploadMaxMb: 1 });
  assert.equal((await send('gros.wav', Buffer.alloc(2 * 1048576), { headers: { 'content-length': String(2 * 1048576) } })).status, 413);
  assert.ok(!fs.readdirSync(path.join(s.hub.downloadDirFor(user), 'Envois')).some((n) => n.startsWith('gros')));
});

test('envoi : réservé à l’administrateur tant qu’il ne l’ouvre pas aux autres comptes', async (t) => {
  const s = await boot({ noCommon: true });
  t.after(s.close);
  const admin = client(s.base);
  await admin.json('POST', '/api/setup/init', { username: 'nil', password: PASSWORD });
  await admin.rpc('srv:users:create', { username: 'ami', password: PASSWORD });
  const ami = client(s.base);
  await ami.json('POST', '/api/auth/login', { username: 'ami', password: PASSWORD });
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'control-up-'));
  t.after(() => fs.rmSync(tmp, { recursive: true, force: true }));
  wav(path.join(tmp, 'a.wav'), 1);
  const bytes = fs.readFileSync(path.join(tmp, 'a.wav'));
  const send = (who) => fetch(`${s.base}/api/upload?name=a.wav`, { method: 'POST', headers: { 'x-control': '1', cookie: who.cookie }, body: bytes });

  const page = async (who) => JSON.parse(/<script id="boot" type="application\/json">(.*?)<\/script>/s.exec(await (await who.request('GET', '/')).text())[1]).caps;
  assert.equal((await page(ami)).upload, false);
  assert.equal((await send(ami)).status, 403);
  await admin.rpc('srv:settings', { allowUserUploads: true });
  assert.equal((await page(ami)).upload, true);
  assert.deepEqual([(await page(ami)).uploadMaxMb], [500]);
  assert.equal((await send(ami)).status, 200);
  // Chacun dans son propre dossier : le morceau est dans la bibliothèque d'ami, pas dans celle de l'administrateur.
  await until(async () => (await ami.rpc('lib:tracks')).length === 1);
  assert.equal((await admin.rpc('lib:tracks')).length, 0);
});

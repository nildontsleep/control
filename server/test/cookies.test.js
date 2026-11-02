'use strict';

// cookies.txt d'un compte (version web) : vérification du format, droits, fichier gardé sur le serveur, usage par yt-dlp.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const cookies = require('../cookies');
const { Downloads } = require('../../src/main/downloads');
const { boot, client } = require('./helpers');

const PASSWORD = 'long enough password';
const row = (domain, name, value) => [domain, 'TRUE', '/', 'TRUE', '1893456000', name, value].join('\t');
const SAMPLE = [
  '# Netscape HTTP Cookie File',
  '# https://curl.haxx.se/rfc/cookie_spec.html',
  '',
  row('.youtube.com', 'SID', 'abc'),
  `#HttpOnly_${row('.google.com', '__Secure-1PSID', 'def')}`,
  row('www.youtube.com', 'PREF', 'f6=40000000'),
].join('\n');

test('cookies.txt : format Netscape accepté et remis en forme, le reste refusé', () => {
  const ok = cookies.normalize(SAMPLE);
  assert.equal(ok.count, 3);
  assert.deepEqual(ok.domains, ['google.com', 'www.youtube.com', 'youtube.com']);
  assert.ok(ok.text.startsWith('# Netscape HTTP Cookie File\n'));
  assert.equal(ok.text.match(/Netscape HTTP Cookie File/g).length, 1);
  assert.ok(ok.text.includes('#HttpOnly_.google.com\t'), 'les cookies HttpOnly sont gardés');

  // Sans l'en-tête, avec des fins de ligne Windows et un BOM : accepté et normalisé.
  const loose = cookies.normalize(`﻿${[row('.youtube.com', 'SID', 'abc')].join('\r\n')}\r\n`);
  assert.ok(loose.text.startsWith('# Netscape HTTP Cookie File\n'));
  assert.equal(loose.count, 1);

  for (const [bad, why] of [
    ['', 'vide'],
    ['   \n', 'espaces'],
    ['# Netscape HTTP Cookie File\n# rien\n', 'aucun cookie'],
    ['.youtube.com TRUE / TRUE 1893456000 SID abc', 'espaces au lieu de tabulations'],
    [`${row('.youtube.com', 'SID', 'abc')}\nn'importe quoi`, 'ligne qui n’est pas un cookie'],
    [`${row('.youtube.com', 'SID', 'abc')}\0`, 'octet nul'],
    [row('.youtube.com', 'SID', 'x'.repeat(cookies.MAX_BYTES)), 'trop gros'],
    [{ not: 'text' }, 'pas du texte'],
  ]) {
    assert.throws(() => cookies.normalize(bad), (err) => err.status === 400, why);
  }
});

test('cookies.txt : envoyé par le compte, gardé sur le serveur, séparé d’un compte à l’autre', async (t) => {
  const s = await boot({ noCommon: true });
  t.after(s.close);
  const admin = client(s.base);
  await admin.json('POST', '/api/setup/init', { username: 'nil', password: PASSWORD, libraryFolders: [s.music] });
  await admin.rpc('srv:users:create', { username: 'ami', password: PASSWORD });
  const ami = client(s.base);
  await ami.json('POST', '/api/auth/login', { username: 'ami', password: PASSWORD });

  assert.deepEqual(await admin.rpc('dl:cookies-status'), { present: false });
  await assert.rejects(admin.rpc('dl:set-cookies', 'pas des cookies'), (err) => err.status === 400);

  const status = await admin.rpc('dl:set-cookies', SAMPLE);
  assert.equal(status.present, true);
  assert.equal(status.count, 3);
  assert.ok(status.domains.includes('youtube.com'));
  assert.ok(!JSON.stringify(status).includes('abc'), 'le contenu ne revient jamais au navigateur');

  const adminId = (await admin.json('GET', '/api/auth/me')).body.user.id;
  const file = path.join(s.root, 'data', 'users', adminId, 'cookies.txt');
  assert.ok(fs.existsSync(file));
  if (process.platform !== 'win32') assert.equal(fs.statSync(file).mode & 0o777, 0o600, 'lisible par le serveur seulement');
  assert.ok(!fs.readFileSync(file, 'utf8').includes('\r'));

  // Les téléchargements sont réservés à l'administrateur tant qu'il ne les ouvre pas : les cookies suivent.
  await assert.rejects(ami.rpc('dl:set-cookies', SAMPLE), (err) => err.status === 403);
  await assert.rejects(ami.rpc('dl:cookies-status'), (err) => err.status === 403);
  await admin.rpc('srv:settings', { allowUserDownloads: true });
  assert.deepEqual(await ami.rpc('dl:cookies-status'), { present: false }, 'le fichier de l’administrateur n’est pas le sien');
  await ami.rpc('dl:set-cookies', row('.youtube.com', 'SID', 'autre'));
  assert.equal((await ami.rpc('dl:cookies-status')).count, 1);
  assert.equal((await admin.rpc('dl:cookies-status')).count, 3);

  await admin.rpc('dl:clear-cookies');
  assert.equal(fs.existsSync(file), false);
  assert.deepEqual(await admin.rpc('dl:cookies-status'), { present: false });
  assert.equal((await ami.rpc('dl:cookies-status')).count, 1, 'celui de l’autre compte reste');

  // Ces fichiers ne passent pas par les réglages : un chemin envoyé par le navigateur est ignoré.
  await admin.json('POST', '/api/send/settings%3Aset', { args: [{ dl: { cookiesFile: '/etc/passwd' } }] });
  const page = await (await admin.request('GET', '/')).text();
  const boot_ = JSON.parse(/<script id="boot" type="application\/json">(.*?)<\/script>/s.exec(page)[1]);
  assert.ok(!boot_.settings.dl?.cookiesFile);
});

test('cookies.txt : yt-dlp reçoit --cookies quand le compte en a envoyé un', { skip: process.platform === 'win32' }, async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'control-cookies-'));
  try {
    const log = path.join(dir, 'args.txt');
    const fake = path.join(dir, 'yt-dlp');
    fs.writeFileSync(fake, `#!/bin/sh\nprintf '%s\\n' "$@" > '${log}'\necho '{"title":"Essai","uploader":"Moi"}'\n`, { mode: 0o755 });
    const cookiesFile = path.join(dir, 'cookies.txt');
    const downloads = new Downloads({
      tools: { path: () => fake, env: () => process.env },
      library: {},
      store: { get: () => ({}) },
      file: path.join(dir, 'downloads.json'),
      emit() {},
      cookiesFile,
    });
    await downloads.info('https://www.youtube.com/watch?v=x');
    assert.ok(!fs.readFileSync(log, 'utf8').includes('--cookies'), 'pas de fichier : pas de cookies');
    cookies.save(cookiesFile, SAMPLE);
    await downloads.info('https://www.youtube.com/watch?v=x');
    const args = fs.readFileSync(log, 'utf8').split('\n');
    assert.equal(args[args.indexOf('--cookies') + 1], cookiesFile);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

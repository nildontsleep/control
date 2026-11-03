'use strict';

// Sauvegardes : archive (sans secrets), chiffrement, découpe, envoi au webhook, planification.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const http = require('node:http');
const { Readable } = require('node:stream');
const { DatabaseSync } = require('node:sqlite');
const { encrypt, decrypt, scrubSettings, WEBHOOK } = require('../backup');
const { unpack } = require('../tar');
const { boot, client, until } = require('./helpers');

const PASSWORD = 'long enough password';

/** Un Discord de poche : reçoit les messages d'un webhook (JSON + fichier joint). */
async function discordMock() {
  const messages = [];
  const state = { limited: 0 };
  const server = http.createServer(async (req, res) => {
    if (state.limited > 0) {
      state.limited--;
      res.writeHead(429, { 'content-type': 'application/json' });
      return res.end(JSON.stringify({ retry_after: 0.01 }));
    }
    const form = await new Response(Readable.toWeb(req), { headers: { 'content-type': req.headers['content-type'] } }).formData();
    const file = form.get('files[0]');
    messages.push({ url: req.url, payload: JSON.parse(form.get('payload_json')), name: file?.name || null, data: file ? Buffer.from(await file.arrayBuffer()) : null });
    res.writeHead(200, { 'content-type': 'application/json' });
    res.end('{"id":"1"}');
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  return { url: `http://127.0.0.1:${server.address().port}/api/webhooks/1`, messages, state, close: () => new Promise((resolve) => { server.closeAllConnections?.(); server.close(resolve); }) };
}

test('chiffrement : aller-retour, mauvaise phrase et fichier abîmé refusés', () => {
  const data = Buffer.from('contenu de la sauvegarde '.repeat(1000));
  const blob = encrypt(data, 'phrase secrète');
  assert.ok(!blob.includes('contenu'));
  assert.deepEqual(decrypt(blob, 'phrase secrète'), data);
  assert.throws(() => decrypt(blob, 'autre phrase'), /incorrecte/);
  const torn = Buffer.from(blob);
  torn[torn.length - 3] ^= 1;
  assert.throws(() => decrypt(torn, 'phrase secrète'), /incorrecte|abîmé/);
  assert.throws(() => decrypt(Buffer.from('autre chose'), 'x'), /pas une sauvegarde/);
});

test('réglages d’un compte : les clés et jetons ne partent pas dans la sauvegarde', () => {
  const out = JSON.parse(scrubSettings(JSON.stringify({ settings: { theme: 'light', media: { tmdbKey: 'K', other: 1 }, update: { token: 'T', auto: true }, scrobble: { listenbrainz: { token: 'LB' } } } })));
  assert.deepEqual(out.settings, { theme: 'light', media: { other: 1 }, update: { auto: true } });
});

test('adresse de webhook : seulement Discord', () => {
  for (const ok of ['https://discord.com/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyzABCDEF-_12', 'https://ptb.discord.com/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyz']) assert.match(ok, WEBHOOK);
  for (const bad of ['http://discord.com/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyz', 'https://evil.example/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyz', 'https://discord.com.evil.example/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyz', 'https://discord.com/api/webhooks/12/short']) assert.doesNotMatch(bad, WEBHOOK);
});

test('sauvegarde : archive chiffrée envoyée au webhook, sans secrets, avec les écoutes de chaque compte', async (t) => {
  const discord = await discordMock();
  t.after(discord.close);
  const s = await boot({ noCommon: true, backupOptions: { allowAnyUrl: true } });
  t.after(s.close);
  const admin = client(s.base);
  await admin.json('POST', '/api/setup/init', { username: 'nil', password: PASSWORD, libraryFolders: [s.music] });
  const ami = (await admin.rpc('srv:users:create', { username: 'ami', password: PASSWORD })).id;
  const amiClient = client(s.base);
  await amiClient.json('POST', '/api/auth/login', { username: 'ami', password: PASSWORD });
  const tracks = await until(async () => { const l = await admin.rpc('lib:tracks'); return l.length === 3 ? l : null; });
  await admin.rpc('lib:play-start', tracks[0].id, 1);
  await admin.rpc('pl:create', 'À garder', 'manual', null, [tracks[1].id]);
  await admin.rpc('dl:set-cookies', ['.youtube.com', 'TRUE', '/', 'TRUE', '1893456000', 'SID', 'secretcookie'].join('\t'));
  await admin.rpc('srv:settings', { updateToken: 'ghp_secret', tmdbKey: 'tmdb_secret' });

  // Réglages : l'adresse invalide est refusée, les secrets ne reviennent jamais au navigateur.
  await assert.rejects(admin.rpc('srv:settings', { backupWebhook: 'ftp://exemple.org/x' }), (err) => err.status === 400);
  await assert.rejects(amiClient.rpc('srv:backup:run'), (err) => err.status === 403);
  await admin.rpc('srv:settings', { backupWebhook: discord.url, backupPassphrase: 'phrase de test', backupEvery: 'daily', backupHour: 3 });
  const shown = await admin.rpc('srv:settings:get');
  assert.deepEqual([shown.hasBackupWebhook, shown.hasBackupPassphrase, shown.backupWebhook, shown.backupPassphrase], [true, true, undefined, undefined]);
  assert.ok(!JSON.stringify(shown).includes('phrase de test'));

  // Discord limite d'abord (429) : l'envoi est retenté.
  discord.state.limited = 1;
  const status = await admin.rpc('srv:backup:run');
  assert.equal(status.last.ok, true);
  assert.equal(status.last.accounts, 2);
  assert.equal(status.last.encrypted, true);
  assert.equal(discord.messages.length, 1);
  assert.match(discord.messages[0].payload.content, /Sauvegarde/);
  assert.match(discord.messages[0].payload.content, /chiffrée/);
  assert.deepEqual(discord.messages[0].payload.allowed_mentions, { parse: [] });
  assert.match(discord.messages[0].name, /^control-backup-\d{12}\.tgz\.enc$/);

  // L'archive : se déchiffre avec la phrase, contient les bases, pas les secrets.
  const archive = unpack(decrypt(discord.messages[0].data, 'phrase de test')); // le contenu chiffré est l'archive .tgz
  const names = archive.map((f) => f.name);
  assert.ok(names.includes('MANIFEST.json') && names.includes('server.db'));
  assert.equal(names.filter((n) => n.endsWith('/library.db')).length, 2, 'une base par compte');
  assert.ok(!names.some((n) => /cookies/.test(n)), 'pas de cookies YouTube');
  const adminId = (await admin.json('GET', '/api/auth/me')).body.user.id;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'control-bk-'));
  t.after(() => fs.rmSync(tmp, { recursive: true, force: true }));
  const open = (name) => {
    const file = path.join(tmp, name.replace(/\W/g, '_'));
    fs.writeFileSync(file, archive.find((f) => f.name === name).data);
    return new DatabaseSync(file, { readOnly: true });
  };
  const serverDb = open('server.db');
  assert.equal(serverDb.prepare('SELECT COUNT(*) AS n FROM sessions').get().n, 0, 'sans les sessions');
  assert.equal(serverDb.prepare('SELECT COUNT(*) AS n FROM users').get().n, 2);
  const kept = JSON.parse(serverDb.prepare("SELECT value FROM kv WHERE key = 'settings'").get().value);
  for (const secret of ['updateToken', 'tmdbKey', 'backupWebhook', 'backupPassphrase']) assert.equal(kept[secret], undefined, secret);
  assert.equal(kept.backupEvery, 'daily');
  assert.equal(JSON.stringify(archive.map((f) => f.data.toString('latin1'))).includes('secretcookie'), false);
  const library = open(`users/${adminId}/library.db`);
  assert.equal(library.prepare('SELECT COUNT(*) AS n FROM plays').get().n, 1, 'les écoutes sont sauvegardées');
  assert.equal(library.prepare('SELECT name FROM playlists').get().name, 'À garder');
  const manifest = JSON.parse(archive.find((f) => f.name === 'MANIFEST.json').data);
  assert.deepEqual(manifest.accounts.map((a) => a.username).sort(), ['ami', 'nil']);
  assert.ok(manifest.accounts.some((a) => a.id === ami));

  // Une copie reste aussi sur le disque du serveur.
  assert.ok(fs.readdirSync(path.join(s.root, 'data', 'backups')).some((n) => /^control-backup-.*\.tgz\.enc$/.test(n)));

  // Test du webhook.
  assert.equal(await admin.rpc('srv:backup:test'), true);
  assert.match(discord.messages[1].payload.content, /fonctionne/);
});

test('sauvegarde : découpée en parties qui se recollent, non chiffrée sans phrase', async (t) => {
  const s = await boot({ noCommon: true, backupOptions: { allowAnyUrl: true } });
  t.after(s.close);
  const admin = client(s.base);
  await admin.json('POST', '/api/setup/init', { username: 'nil', password: PASSWORD, libraryFolders: [s.music] });
  await until(async () => (await admin.rpc('lib:tracks')).length === 3);
  const built = await s.hub.backups.build({ partBytes: 4096 });
  assert.ok(built.parts.length > 1, 'plusieurs parties');
  assert.ok(built.parts.every((p, i) => p.data.length <= 4096 && p.name === `${built.name}.part${String(i + 1).padStart(2, '0')}`));
  assert.equal(built.encrypted, false);
  const joined = Buffer.concat(built.parts.map((p) => p.data));
  assert.equal(joined.length, built.size);
  assert.ok(unpack(joined).some((f) => f.name === 'server.db'));
  await assert.rejects(s.hub.backups.run(), (err) => err.status === 400, 'sans webhook, rien n’est envoyé');
});

test('sauvegarde planifiée : due à l’heure choisie, une fois par jour ou par semaine, nouvel essai après un échec', async (t) => {
  const s = await boot({ noCommon: true, backupOptions: { allowAnyUrl: true } });
  t.after(s.close);
  const b = s.hub.backups;
  const at = (iso) => new Date(iso);
  assert.equal(b.due(at('2026-11-03T05:00:00')), false, 'rien sans webhook');
  s.hub.patchSettings({ backupWebhook: 'http://127.0.0.1:1/api/webhooks/1', backupEvery: 'daily', backupHour: 3 });
  assert.equal(b.due(at('2026-11-03T02:30:00')), false, 'avant l’heure choisie');
  assert.equal(b.due(at('2026-11-03T03:05:00')), true);

  const save = (state) => s.hub.db.prepare("INSERT INTO kv (key, value) VALUES ('backup_state', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value").run(JSON.stringify(state));
  save({ at: at('2026-11-03T03:05:00').getTime(), ok: true });
  assert.equal(b.due(at('2026-11-03T15:00:00')), false, 'déjà faite aujourd’hui');
  assert.equal(b.due(at('2026-11-04T02:00:00')), false);
  assert.equal(b.due(at('2026-11-04T03:10:00')), true, 'le lendemain à l’heure');

  s.hub.patchSettings({ backupEvery: 'weekly' });
  assert.equal(b.due(at('2026-11-04T03:10:00')), false);
  assert.equal(b.due(at('2026-11-10T03:10:00')), true, 'une semaine plus tard');

  s.hub.patchSettings({ backupEvery: 'daily' });
  save({ at: at('2026-11-04T03:10:00').getTime(), ok: false, error: 'x' });
  assert.equal(b.due(at('2026-11-04T03:40:00')), false, 'pas de nouvel essai avant une heure');
  assert.equal(b.due(at('2026-11-04T04:20:00')), true);
  s.hub.patchSettings({ backupEvery: 'off' });
  assert.equal(b.due(at('2026-11-20T04:20:00')), false);
});

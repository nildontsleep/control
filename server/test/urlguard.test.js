'use strict';

// Liens de téléchargement acceptés par le serveur : publics, en http(s) ; jamais le réseau interne.

const test = require('node:test');
const assert = require('node:assert/strict');
const { checkUrl, isPrivateAddress } = require('../../src/main/urlguard');
const { boot, client } = require('./helpers');

const PUBLIC = async () => [{ address: '93.184.216.34', family: 4 }];

test('adresses privées, locales et réservées reconnues (IPv4, IPv6, IPv4 encapsulée)', () => {
  for (const ip of ['127.0.0.1', '10.1.2.3', '172.16.0.1', '172.31.255.255', '192.168.1.1', '169.254.169.254', '100.64.0.1', '0.0.0.0', '224.0.0.1', '::1', '::', 'fe80::1', 'fd12:3456::1', '::ffff:127.0.0.1', '::ffff:10.0.0.1', 'not-an-ip']) {
    assert.equal(isPrivateAddress(ip), true, ip);
  }
  for (const ip of ['8.8.8.8', '1.1.1.1', '93.184.216.34', '172.32.0.1', '2606:4700:4700::1111']) assert.equal(isPrivateAddress(ip), false, ip);
});

test('liens : seulement http(s) public', async () => {
  assert.equal(await checkUrl('  https://www.youtube.com/watch?v=abc  ', { resolve: PUBLIC }), 'https://www.youtube.com/watch?v=abc');
  for (const [bad, why] of [
    ['file:///etc/passwd', 'fichier local'],
    ['ftp://exemple.org/a', 'autre protocole'],
    ['javascript:alert(1)', 'script'],
    ['http://127.0.0.1:5180/api/health', 'boucle locale'],
    ['http://localhost/x', 'localhost (résolu en 127.0.0.1)'],
    ['http://2130706433/', 'IPv4 écrite en entier'],
    ['http://0x7f.1/', 'IPv4 en hexadécimal'],
    ['http://[::1]/', 'IPv6 locale'],
    ['http://[::ffff:7f00:1]/', 'IPv4 encapsulée dans de l’IPv6'],
    ['http://169.254.169.254/latest/meta-data/', 'service de métadonnées'],
    ['http://user:pass@exemple.org/', 'identifiants dans l’adresse'],
    ['', 'vide'],
    [42, 'pas du texte'],
    [`https://exemple.org/${'a'.repeat(3000)}`, 'trop long'],
  ]) {
    await assert.rejects(checkUrl(bad, { resolve: async (h) => (h === 'localhost' ? [{ address: '127.0.0.1', family: 4 }] : PUBLIC()) }), Error, why);
  }
  // Un nom qui pointe (aussi) vers une adresse interne est refusé, un nom introuvable aussi.
  await assert.rejects(checkUrl('https://interne.exemple.org/', { resolve: async () => [{ address: '93.184.216.34' }, { address: '10.0.0.5' }] }), /réseau du serveur/);
  await assert.rejects(checkUrl('https://nexiste.pas/', { resolve: async () => { throw new Error('ENOTFOUND'); } }), /introuvable/);
});

test('serveur : dl:add et dl:info refusent un lien interne avant de lancer yt-dlp', async (t) => {
  const s = await boot();
  t.after(s.close);
  const admin = client(s.base);
  await admin.json('POST', '/api/setup/init', { username: 'nil', password: 'long enough password', libraryFolders: [s.music] });
  const info = await admin.rpc('dl:info', 'http://127.0.0.1:5180/api/health');
  assert.equal(info.ok, false);
  assert.match(info.error, /réseau du serveur/);
  await assert.rejects(admin.rpc('dl:add', { url: 'file:///etc/passwd', title: 'x' }), /http et https/);
  await assert.rejects(admin.rpc('dl:add', 'pas un objet'), (err) => err.status === 400 && /invalide/.test(err.message));
  assert.equal((await admin.rpc('dl:list')).length, 0, 'rien n’est mis en file');
});

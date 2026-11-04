'use strict';

// API compatible Subsonic / OpenSubsonic : authentification, parcours, recherche, flux, pochettes, playlists, favoris, écoutes.

const test = require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { toXml } = require('../subsonic');
const { boot, client, until } = require('./helpers');

const PASSWORD = 'long enough password';
const md5 = (s) => crypto.createHash('md5').update(s).digest('hex');

/** Un client Subsonic : signe ses requêtes avec le jeton md5(secret + sel), comme les applications du commerce. */
function subsonic(base, username, secret) {
  const auth = () => {
    const salt = crypto.randomBytes(5).toString('hex');
    return { u: username, t: md5(secret + salt), s: salt, v: '1.16.1', c: 'essai' };
  };
  const url = (method, params = {}) => {
    const q = new URLSearchParams(auth());
    for (const [k, v] of Object.entries(params)) for (const x of [].concat(v)) q.append(k, x);
    return `${base}/rest/${method}.view?${q}`;
  };
  return {
    url,
    raw: (method, params, init) => fetch(url(method, params), init),
    async call(method, params = {}) {
      const res = await fetch(url(method, { f: 'json', ...params }));
      return (await res.json())['subsonic-response'];
    },
  };
}

async function house(t) {
  const s = await boot({ noCommon: true });
  t.after(s.close);
  const nil = client(s.base);
  await nil.json('POST', '/api/setup/init', { username: 'nil', password: PASSWORD, libraryFolders: [s.music] });
  const tracks = await until(async () => { const l = await nil.rpc('lib:tracks'); return l.length === 3 ? l : null; });
  const user = s.hub.accounts.list()[0];
  const core = s.hub.entry(user).core;
  // Les fichiers de test n'ont pas de tags : on leur en donne.
  const meta = [['Lune', 'Aya', 'Nuits', 2023, 1, 'Pop'], ['Soleil', 'Aya', 'Nuits', 2023, 2, 'Pop'], ['Perihelie', 'Kiro', 'Orbit', 2022, 1, 'Electro']];
  const sorted = [...tracks].sort((a, b) => a.id - b.id);
  const cover = 'a'.repeat(20);
  fs.mkdirSync(s.hub.coversDir, { recursive: true });
  fs.writeFileSync(path.join(s.hub.coversDir, `${cover}.jpg`), Buffer.from('/9j/4AAQSkZJRgABAQEASABIAAD/2wBD', 'base64'));
  sorted.forEach((tr, i) => core.db.prepare('UPDATE tracks SET title = ?, artist = ?, album = ?, year = ?, track_no = ?, genre = ?, cover = ? WHERE id = ?').run(...meta[i].slice(0, 6), i < 2 ? cover : null, tr.id));
  core.library.changed('tracks');
  const secret = (await nil.rpc('srv:me:subsonic-reveal', PASSWORD)).secret;
  return { s, nil, core, sorted, secret, sub: subsonic(s.base, 'nil', secret), cover };
}

test('XML : valeurs simples en attributs, objets et tableaux en éléments, caractères échappés', () => {
  const xml = toXml('album', { id: 'al-1', name: 'Nuits & <jours>', song: [{ id: '1', title: 'A"B' }, { id: '2' }], extra: { x: 1 }, vide: null });
  assert.equal(xml, '<album id="al-1" name="Nuits &amp; &lt;jours&gt;"><song id="1" title="A&quot;B"/><song id="2"/><extra x="1"/></album>');
  assert.equal(toXml('lyrics', { artist: 'Aya', value: 'Ligne 1\nLigne 2' }), '<lyrics artist="Aya">Ligne 1\nLigne 2</lyrics>');
});

test('authentification : mot de passe d’application, jeton, clé d’API, refus et limitation', async (t) => {
  const { s, nil, secret, sub } = await house(t);
  const noSecret = subsonic(s.base, 'nil', 'pas-le-bon');
  assert.deepEqual((await noSecret.call('ping')).error?.code, 40);

  // Le mot de passe d'application ne se révèle qu'avec le mot de passe du compte.
  await assert.rejects(nil.rpc('srv:me:subsonic-reveal', 'faux'), (err) => err.status === 403);
  assert.equal((await nil.rpc('srv:me:subsonic')).hasSecret, true);

  const ok = await sub.call('ping');
  assert.deepEqual([ok.status, ok.version, ok.type, ok.openSubsonic], ['ok', '1.16.1', 'control', true]);
  assert.equal((await fetch(`${s.base}/rest/ping?u=nil&p=${secret}&v=1.16.1&c=x&f=json`).then((r) => r.json()))['subsonic-response'].status, 'ok', 'mot de passe en clair');
  const hex = Buffer.from(secret).toString('hex');
  assert.equal((await fetch(`${s.base}/rest/ping?u=nil&p=enc:${hex}&v=1.16.1&c=x&f=json`).then((r) => r.json()))['subsonic-response'].status, 'ok', 'mot de passe en hexadécimal');
  assert.equal((await fetch(`${s.base}/rest/ping?u=nil&p=${PASSWORD}&v=1.16.1&c=x&f=json`).then((r) => r.json()))['subsonic-response'].error.code, 40, 'le mot de passe du compte n’ouvre pas l’API');
  assert.equal((await fetch(`${s.base}/rest/ping?u=nil&t=${md5('faux' + 'sel')}&s=sel&v=1.16.1&c=x&f=json`).then((r) => r.json()))['subsonic-response'].error.code, 40);

  // Clé d'API (OpenSubsonic) : même secret, sans nom d'utilisateur ; les deux ensemble sont refusés.
  assert.equal((await fetch(`${s.base}/rest/ping?apiKey=${secret}&c=x&f=json`).then((r) => r.json()))['subsonic-response'].status, 'ok');
  assert.equal((await fetch(`${s.base}/rest/tokenInfo?apiKey=${secret}&c=x&f=json`).then((r) => r.json()))['subsonic-response'].tokenInfo.username, 'nil');
  assert.equal((await fetch(`${s.base}/rest/ping?apiKey=${secret}&u=nil&c=x&f=json`).then((r) => r.json()))['subsonic-response'].error.code, 43);
  assert.equal((await fetch(`${s.base}/rest/ping?apiKey=mauvaise&c=x&f=json`).then((r) => r.json()))['subsonic-response'].error.code, 44);
  // Pas de cookie de session : l'API ne s'ouvre pas avec la connexion du navigateur.
  assert.equal((await fetch(`${s.base}/rest/ping?c=x&f=json`, { headers: { cookie: nil.cookie } }).then((r) => r.json()))['subsonic-response'].error.code, 10);

  // XML par défaut, paramètre c obligatoire hors ping, méthode inconnue.
  const xml = await sub.raw('getLicense').then((r) => r.text());
  assert.match(xml, /^<\?xml version="1.0" encoding="UTF-8"\?>\n<subsonic-response xmlns="http:\/\/subsonic.org\/restapi" status="ok" version="1.16.1" type="control"/);
  assert.match(xml, /<license valid="true"/);
  assert.equal((await fetch(`${s.base}/rest/getArtists?u=nil&p=${secret}&f=json`).then((r) => r.json()))['subsonic-response'].error.code, 10);
  assert.equal((await sub.call('nimporte')).error.code, 0);
  assert.deepEqual((await sub.call('getOpenSubsonicExtensions')).openSubsonicExtensions.map((e) => e.name), ['apiKeyAuthentication', 'formPost', 'songLyrics']);

  // Révocation puis renouvellement : l'ancien secret ne marche plus.
  const renewed = (await nil.rpc('srv:me:subsonic-renew', PASSWORD)).secret;
  assert.notEqual(renewed, secret);
  assert.equal((await sub.call('ping')).error.code, 40);
  assert.equal((await subsonic(s.base, 'nil', renewed).call('ping')).status, 'ok');
  await nil.rpc('srv:me:subsonic-revoke');
  assert.equal((await subsonic(s.base, 'nil', renewed).call('ping')).error.code, 40);

  // L'administrateur peut couper l'API ; trop d'essais ratés sont limités.
  const again = (await nil.rpc('srv:me:subsonic-reveal', PASSWORD)).secret;
  await nil.rpc('srv:settings', { subsonicEnabled: false });
  assert.equal((await subsonic(s.base, 'nil', again).call('ping')).error.code, 50);
  await nil.rpc('srv:settings', { subsonicEnabled: true });
  for (let i = 0; i < 30; i++) await fetch(`${s.base}/rest/ping?u=nil&p=non&c=x&f=json`);
  assert.match((await subsonic(s.base, 'nil', again).call('ping')).error.message, /Trop d’essais/);
});

test('parcours : artistes, albums, morceaux, genres, listes, recherche', async (t) => {
  const { sorted, sub } = await house(t);
  assert.deepEqual((await sub.call('getMusicFolders')).musicFolders.musicFolder, [{ id: 1, name: 'Bibliothèque' }]);

  const artists = (await sub.call('getArtists')).artists;
  assert.deepEqual(artists.index.map((i) => i.name), ['A', 'K']);
  const aya = artists.index[0].artist[0];
  assert.deepEqual([aya.name, aya.albumCount], ['Aya', 1]);
  assert.match(aya.id, /^ar-/);
  assert.equal((await sub.call('getIndexes')).indexes.index[0].artist[0].name, 'Aya');

  const artist = (await sub.call('getArtist', { id: aya.id })).artist;
  assert.deepEqual([artist.name, artist.album.length, artist.album[0].name, artist.album[0].songCount, artist.album[0].year], ['Aya', 1, 'Nuits', 2, 2023]);
  const album = (await sub.call('getAlbum', { id: artist.album[0].id })).album;
  assert.deepEqual(album.song.map((x) => x.title), ['Lune', 'Soleil'], 'dans l’ordre des pistes');
  const song = album.song[0];
  assert.deepEqual([song.id, song.album, song.artist, song.track, song.genre, song.suffix, song.contentType, song.type, song.isDir], [String(sorted[0].id), 'Nuits', 'Aya', 1, 'Pop', 'wav', 'audio/wav', 'music', false]);
  assert.ok(song.coverArt.startsWith('cv-') && song.duration >= 1 && song.path.startsWith('Aya/Nuits/'));
  assert.equal((await sub.call('getSong', { id: song.id })).song.title, 'Lune');
  assert.equal((await sub.call('getSong', { id: 99999 })).error.code, 70);
  assert.equal((await sub.call('getAlbum', { id: 'al-inconnu' })).error.code, 70);

  // Navigation par dossiers : artiste → albums → morceaux.
  const dir = (await sub.call('getMusicDirectory', { id: aya.id })).directory;
  assert.deepEqual([dir.name, dir.child[0].isDir, dir.child[0].title], ['Aya', true, 'Nuits']);
  assert.equal((await sub.call('getMusicDirectory', { id: dir.child[0].id })).directory.child.length, 2);

  assert.deepEqual((await sub.call('getGenres')).genres.genre.map((g) => [g.value, g.songCount, g.albumCount]), [['Electro', 1, 1], ['Pop', 2, 1]]);
  assert.deepEqual((await sub.call('getAlbumList2', { type: 'alphabeticalByName' })).albumList2.album.map((a) => a.name), ['Nuits', 'Orbit']);
  assert.deepEqual((await sub.call('getAlbumList2', { type: 'byYear', fromYear: 2022, toYear: 2022 })).albumList2.album.map((a) => a.name), ['Orbit']);
  assert.deepEqual((await sub.call('getAlbumList2', { type: 'byGenre', genre: 'pop' })).albumList2.album.map((a) => a.name), ['Nuits']);
  assert.equal((await sub.call('getAlbumList2', { type: 'random', size: 1 })).albumList2.album.length, 1);
  assert.equal((await sub.call('getAlbumList', { type: 'newest' })).albumList.album[0].isDir, true);
  assert.equal((await sub.call('getRandomSongs', { size: 5 })).randomSongs.song.length, 3);
  assert.deepEqual((await sub.call('getSongsByGenre', { genre: 'Electro' })).songsByGenre.song.map((x) => x.title), ['Perihelie']);

  const found = (await sub.call('search3', { query: 'lun', songCount: 5 })).searchResult3;
  assert.deepEqual([found.song.map((x) => x.title), found.artist.length, found.album.length], [['Lune'], 0, 0]);
  assert.deepEqual((await sub.call('search3', { query: 'aya' })).searchResult3.artist.map((x) => x.name), ['Aya']);
  assert.equal((await sub.call('search2', { query: 'orbit' })).searchResult2.album[0].title, 'Orbit');
  assert.equal((await sub.call('search3', { query: '' })).searchResult3.song.length, 3, 'requête vide : tout (les clients s’en servent pour tout lister)');
  assert.equal((await sub.call('getArtistInfo2', { id: aya.id })).artistInfo2 !== undefined, true);
  assert.equal((await sub.call('getScanStatus')).scanStatus.count, 3);
  assert.equal((await sub.call('getUser', { username: 'nil' })).user.streamRole, true);
});

test('médias : flux avec Range, pochettes, accès limité aux fichiers de la bibliothèque', async (t) => {
  const { s, sorted, sub, cover } = await house(t);
  const full = await sub.raw('stream', { id: sorted[0].id });
  assert.equal(full.status, 200);
  assert.equal(full.headers.get('content-type'), 'audio/wav');
  const bytes = Buffer.from(await full.arrayBuffer());
  const part = await sub.raw('stream', { id: sorted[0].id }, { headers: { range: 'bytes=0-99' } });
  assert.equal(part.status, 206);
  assert.equal((await part.arrayBuffer()).byteLength, 100);
  assert.equal(part.headers.get('content-range'), `bytes 0-99/${bytes.length}`);
  const dl = await sub.raw('download', { id: sorted[0].id });
  assert.match(dl.headers.get('content-disposition'), /attachment; filename=".*\.wav"/);
  // Un identifiant qui n'est pas de cette bibliothèque : erreur Subsonic (HTTP 200, code 70), jamais un fichier.
  const missing = await sub.raw('stream', { id: 424242, f: 'json' });
  assert.equal((await missing.json())['subsonic-response'].error.code, 70);

  const art = await sub.raw('getCoverArt', { id: `cv-${cover}`, size: 100 });
  assert.equal(art.status, 200);
  assert.equal(art.headers.get('content-type'), 'image/jpeg');
  assert.equal((await sub.raw('getCoverArt', { id: String(sorted[0].id) })).status, 200, 'par morceau');
  assert.equal((await sub.raw('getCoverArt', { id: 'cv-../../etc/passwd', f: 'json' }).then((r) => r.json()))['subsonic-response'].error.code, 70);
  assert.equal((await sub.raw('getCoverArt', { id: String(sorted[2].id), f: 'json' }).then((r) => r.json()))['subsonic-response'].error.code, 70, 'sans pochette');

  // En POST (formPost) aussi.
  const q = new URLSearchParams({ f: 'json', c: 'x', v: '1.16.1', u: 'nil', ...Object.fromEntries(new URL(sub.url('ping')).searchParams) });
  const posted = await fetch(`${s.base}/rest/ping`, { method: 'POST', headers: { 'content-type': 'application/x-www-form-urlencoded' }, body: q });
  assert.equal((await posted.json())['subsonic-response'].status, 'ok');
});

test('playlists, favoris et écoutes comptées', async (t) => {
  const { s, nil, core, sorted, sub } = await house(t);
  const created = (await sub.call('createPlaylist', { name: 'Route', songId: [sorted[0].id, sorted[2].id] })).playlist;
  assert.deepEqual([created.name, created.songCount, created.entry.map((x) => x.title)], ['Route', 2, ['Lune', 'Perihelie']]);
  assert.ok(created.id.startsWith('pl-'));
  const listed = (await sub.call('getPlaylists')).playlists.playlist;
  assert.deepEqual(listed.map((p) => [p.name, p.owner, p.songCount]), [['Route', 'nil', 2]]);
  await sub.call('updatePlaylist', { playlistId: created.id, name: 'Route 66', comment: 'Pour rouler', songIdToAdd: [sorted[1].id], songIndexToRemove: [0] });
  const updated = (await sub.call('getPlaylist', { id: created.id })).playlist;
  assert.deepEqual([updated.name, updated.comment, updated.entry.map((x) => x.title)], ['Route 66', 'Pour rouler', ['Perihelie', 'Soleil']]);
  assert.equal((await nil.rpc('lib:playlists')).playlists[0].name, 'Route 66', 'la playlist est aussi celle de control');
  assert.equal((await sub.call('deletePlaylist', { id: created.id })).status, 'ok');
  assert.equal((await sub.call('getPlaylist', { id: created.id })).error.code, 70);

  // Favoris : star / unstar passent par la playlist « Favoris ».
  assert.deepEqual((await sub.call('getStarred2')).starred2.song, []);
  await sub.call('star', { id: [sorted[0].id, sorted[1].id] });
  await sub.call('star', { id: sorted[0].id }); // déjà étoilé : pas de doublon
  assert.deepEqual((await sub.call('getStarred2')).starred2.song.map((x) => x.title), ['Lune', 'Soleil']);
  assert.equal((await nil.rpc('lib:playlists')).playlists.find((p) => p.name === 'Favoris') !== undefined, true);
  await sub.call('unstar', { id: sorted[0].id });
  assert.deepEqual((await sub.call('getStarred', {})).starred.song.map((x) => x.title), ['Soleil']);

  // Écoutes : « en cours » ne compte pas, « terminée » compte comme une écoute entière et part vers les services connectés.
  const calls = [];
  core.scrobbler.fetch = async (url, init) => { calls.push({ url: String(url), body: init?.body ? JSON.parse(init.body) : null }); return { ok: true, status: 200, headers: { get: () => null }, json: async () => ({ valid: true, user_name: 'nil', status: 'ok' }) }; };
  await nil.rpc('scrobble:connect-listenbrainz', 'abcdef12345-token', null);
  core.db.prepare("UPDATE tracks SET duration = 200 WHERE id = ?").run(sorted[0].id);
  core.library.changed('tracks');
  await sub.call('scrobble', { id: sorted[0].id, submission: 'false' });
  assert.equal((await nil.rpc('lib:tracks')).find((x) => x.id === sorted[0].id).playCount, 0);
  const when = Date.now() - 60000;
  await sub.call('scrobble', { id: sorted[0].id, time: when });
  assert.equal((await nil.rpc('lib:tracks')).find((x) => x.id === sorted[0].id).playCount, 1);
  const today = new Date().toISOString().slice(0, 10);
  assert.equal((await nil.rpc('stats:get', { from: today, to: today })).data.music.plays, 1, 'compte dans les statistiques');
  await until(() => calls.some((c) => c.body?.listen_type === 'single'));
  assert.equal(calls.find((c) => c.body?.listen_type === 'single').body.payload[0].listened_at, Math.floor(when / 1000));
  assert.ok(calls.some((c) => c.body?.listen_type === 'playing_now'));

  // Paroles : texte synchronisé converti en lignes avec leur temps (millisecondes).
  core.lyrics.get = async () => ({ synced: '[00:01.50] Bonjour\n[01:03.00] Monde', plain: 'Bonjour\nMonde' });
  const lyrics = (await sub.call('getLyricsBySongId', { id: sorted[0].id })).lyricsList.structuredLyrics[0];
  assert.deepEqual([lyrics.synced, lyrics.line], [true, [{ start: 1500, value: 'Bonjour' }, { start: 63000, value: 'Monde' }]]);
  assert.equal(s.hub.subsonic.methods.getnowplaying() !== undefined, true);
});

test('journal des connexions : la raison exacte de chaque refus, visible de l’administrateur seulement', async (t) => {
  const s = await boot({ noCommon: true });
  t.after(s.close);
  const nil = client(s.base);
  await nil.json('POST', '/api/setup/init', { username: 'nil', password: PASSWORD, libraryFolders: [s.music] });
  const ping = (query, init) => fetch(`${s.base}/rest/ping.view?${query}&v=1.13.0&c=Amperfy&f=json`, init).then((r) => r.json());

  // Compte sans mot de passe d'application, puis avec : ce que fait une application qui essaie le jeton puis le mot de passe simple.
  await ping('u=nil&p=n-importe-quoi');
  const secret = (await nil.rpc('srv:me:subsonic-reveal', PASSWORD)).secret;
  await ping(`u=nil&p=${encodeURIComponent(PASSWORD)}`); // le mot de passe du COMPTE : classique
  await ping('u=nil&t=00000000000000000000000000000000&s=abc'); // jeton faux
  await ping('u=inconnu&p=x');
  await ping(`u=NIL&p=${secret}`, { headers: { 'user-agent': 'Amperfy/1.0' } }); // casse de l'identifiant ignorée : accepté

  const log = await nil.rpc('srv:subsonic:log');
  const reasons = log.map((e) => e.reason);
  assert.match(reasons[0], /connexion acceptée/, 'la plus récente d’abord');
  assert.ok(log[0].ok && log[0].client === 'Amperfy' && log[0].user === 'nil' && log[0].agent.startsWith('Amperfy'));
  assert.ok(reasons.some((r) => /pas encore de mot de passe d’application/.test(r)), 'avant la création du secret');
  assert.ok(reasons.some((r) => /mot de passe du COMPTE/.test(r)), 'mot de passe du compte reconnu');
  assert.ok(reasons.some((r) => /jeton incorrect/.test(r)));
  assert.ok(reasons.some((r) => /introuvable/.test(r)));
  // Rien de secret dans le journal.
  assert.ok(!JSON.stringify(log).includes(secret) && !JSON.stringify(log).includes(PASSWORD));
  // Et le client, lui, ne reçoit que le message générique (jamais « c'est le mot de passe du compte »).
  const generic = await ping(`u=nil&p=${encodeURIComponent(PASSWORD)}`);
  assert.equal(generic['subsonic-response'].error.message, 'Nom d’utilisateur ou mot de passe d’application incorrect.');

  // Réservé à l'administrateur.
  await nil.rpc('srv:users:create', { username: 'ami', password: PASSWORD });
  const ami = client(s.base);
  await ami.json('POST', '/api/auth/login', { username: 'ami', password: PASSWORD });
  await assert.rejects(ami.rpc('srv:subsonic:log'), (err) => err.status === 403);
});

test('limiteur d’essais : par adresse réelle derrière un proxy, pas par adresse du proxy', async (t) => {
  const s = await boot({ noCommon: true, trustProxy: true });
  t.after(s.close);
  const nil = client(s.base);
  await nil.json('POST', '/api/setup/init', { username: 'nil', password: PASSWORD, libraryFolders: [s.music] });
  const secret = (await nil.rpc('srv:me:subsonic-reveal', PASSWORD)).secret;
  const call = (ip, pass) => fetch(`${s.base}/rest/ping?u=nil&p=${pass}&v=1.16.1&c=x&f=json`, { headers: { 'x-forwarded-for': ip } }).then((r) => r.json()).then((j) => j['subsonic-response']);
  for (let i = 0; i < 30; i++) await call('203.0.113.7', 'non');
  assert.match((await call('203.0.113.7', secret)).error.message, /Trop d’essais/, 'l’attaquant est bloqué');
  assert.equal((await call('198.51.100.20', secret)).status, 'ok', 'un autre client derrière le même proxy ne l’est pas');
});

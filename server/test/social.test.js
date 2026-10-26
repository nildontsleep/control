'use strict';

// Bibliothèques par compte, partages, lecture seule, profils, playlists personnalisées, comparaison de récaps.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { PNG } = require('pngjs');
const { wav, boot, client, until } = require('./helpers');
const { compare } = require('../compare');

const PASSWORD = 'long enough password';

/** Un PNG de couleur unie, en data URL (de quoi tester les envois d'images). */
function pngDataUrl(width = 24, height = 16, rgb = [200, 60, 90]) {
  const png = new PNG({ width, height });
  for (let i = 0; i < width * height; i++) png.data.set([...rgb, 255], i * 4);
  return `data:image/png;base64,${PNG.sync.write(png).toString('base64')}`;
}

/** Un serveur sans dossier commun : nil (administrateur, 3 morceaux à lui), ami (1 morceau), zoe (rien). */
async function house(t, extra = {}) {
  const s = await boot({ noCommon: true, ...extra });
  t.after(s.close);
  const nil = client(s.base);
  await nil.json('POST', '/api/setup/init', { username: 'nil', password: PASSWORD, libraryFolders: [s.music] });
  const ids = {};
  for (const name of ['ami', 'zoe']) {
    ids[name] = (await nil.rpc('srv:users:create', { username: name, password: PASSWORD })).id;
  }
  const amiMusic = path.join(s.root, 'ami-music');
  wav(path.join(amiMusic, 'Band', 'Song.wav'));
  await nil.rpc('srv:users:add-folder', ids.ami, amiMusic);
  const ami = client(s.base);
  await ami.json('POST', '/api/auth/login', { username: 'ami', password: PASSWORD });
  const zoe = client(s.base);
  await zoe.json('POST', '/api/auth/login', { username: 'zoe', password: PASSWORD });
  const me = await nil.json('GET', '/api/auth/me');
  ids.nil = me.body.user.id;
  return { s, nil, ami, zoe, ids, amiMusic };
}

test('chaque compte a sa propre bibliothèque ; l’administrateur attribue les dossiers', async (t) => {
  const { s, nil, ami, zoe, ids } = await house(t);
  await until(async () => (await nil.rpc('lib:tracks')).length === 3);
  await until(async () => (await ami.rpc('lib:tracks')).length === 1);
  assert.equal((await zoe.rpc('lib:tracks')).length, 0);

  // Un compte ordinaire ne choisit pas ses dossiers : ce serait lire n'importe quoi sur le serveur.
  await assert.rejects(ami.rpc('srv:me:add-folder', s.music), (err) => err.status === 403);
  await assert.rejects(ami.rpc('srv:users:add-folder', ids.ami, s.music), (err) => err.status === 403);
  await ami.json('POST', '/api/send/settings%3Aset', { args: [{ libraryFolders: [s.music], downloadDir: '/tmp' }] });
  const detail = await nil.rpc('srv:users:detail', ids.ami);
  assert.equal(detail.folders.length, 1);

  // Les fichiers servis sont ceux de son propre compte.
  const mine = (await ami.rpc('lib:tracks'))[0];
  assert.equal((await ami.request('GET', `/media/${mine.id}`)).status, 200);
  const others = await nil.rpc('lib:tracks');
  const high = Math.max(...others.map((x) => x.id));
  assert.equal((await ami.request('GET', `/media/${high}`)).status, 404);

  // Retirer un dossier de la liste d'un compte le vide de ses morceaux (les fichiers restent).
  await nil.rpc('srv:users:remove-folder', ids.ami, detail.folders[0]);
  await until(async () => (await ami.rpc('lib:tracks')).length === 0);
  assert.ok(fs.existsSync(detail.folders[0]));
});

test('dossier perso automatique et dossiers communs', async (t) => {
  const { s, nil, zoe, ids } = await house(t);
  const usersRoot = path.join(s.root, 'users-root');
  fs.mkdirSync(usersRoot);
  await nil.rpc('srv:settings', { usersRoot });
  const created = await nil.rpc('srv:users:create', { username: 'lea', password: PASSWORD });
  assert.ok(fs.statSync(path.join(usersRoot, 'lea')).isDirectory());
  assert.deepEqual((await nil.rpc('srv:users:detail', created.id)).folders, [path.join(usersRoot, 'lea')]);

  // Pour un compte existant, à la demande.
  await nil.rpc('srv:users:personal-folder', ids.zoe);
  assert.ok(fs.existsSync(path.join(usersRoot, 'zoe')));

  // Un dossier commun apparaît chez tout le monde, sans changer les dossiers propres.
  const shared = path.join(s.root, 'shared');
  wav(path.join(shared, 'Commun', 'Tune.wav'));
  await nil.rpc('srv:add-folder', shared);
  await until(async () => (await zoe.rpc('lib:tracks')).length === 1);
});

test('partage : on ne voit que ce qui est partagé, en lecture seule', async (t) => {
  const { s, nil, ami, zoe, ids } = await house(t);
  await until(async () => (await nil.rpc('lib:tracks')).length === 3);

  // Rien n'est partagé : refus, et aucune fuite dans la liste des bibliothèques.
  await assert.rejects(ami.rpcIn(ids.nil, 'lib:tracks'), (err) => err.status === 403);
  const none = await ami.rpc('social:others');
  assert.equal(none.find((p) => p.username === 'nil').access.browse, false);

  // nil partage avec ami seulement.
  await nil.rpc('social:set-grants', [{ viewerId: ids.ami, browse: true, compare: false }]);
  const view = await ami.rpcIn(ids.nil, 'lib:tracks');
  assert.equal(view.length, 3);
  assert.ok(view.every((x) => !path.isAbsolute(x.path) && x.playCount === 0), 'chemins relatifs, pas de compteurs');
  await assert.rejects(zoe.rpcIn(ids.nil, 'lib:tracks'), (err) => err.status === 403);

  // Les fichiers : seulement avec ?lib=, et jamais hors des dossiers propres du propriétaire.
  const id = view[0].id;
  const direct = await ami.request('GET', `/media/${id}`);
  assert.equal(direct.status === 200 ? (await direct.arrayBuffer()).byteLength : 0, direct.status === 200 ? (await (await ami.request('GET', `/media/${id}`)).arrayBuffer()).byteLength : 0);
  const shared = await ami.request('GET', `/media/${id}?lib=${ids.nil}`, undefined, { range: 'bytes=0-9' });
  assert.equal(shared.status, 206);
  assert.equal((await zoe.request('GET', `/media/${id}?lib=${ids.nil}`)).status, 403);

  // Lecture seule : ni playlists, ni tags, ni suppression ; les écoutes ne sont pas comptées.
  await assert.rejects(ami.rpcIn(ids.nil, 'pl:create', 'Intruse'), (err) => err.status === 403 && err.code === 'read_only');
  await assert.rejects(ami.rpcIn(ids.nil, 'lib:remove-tracks', [id], true), (err) => err.status === 403);
  assert.equal(await ami.rpcIn(ids.nil, 'lib:play-start', id, 10), 0);
  assert.equal((await nil.rpc('lib:tracks')).find((x) => x.id === id).playCount, 0);

  // La page d'accueil annonce la vue ; une bibliothèque non partagée ramène à la sienne.
  const page = await (await ami.request('GET', `/?lib=${ids.nil}`)).text();
  const boot_ = JSON.parse(/<script id="boot" type="application\/json">(.*?)<\/script>/s.exec(page)[1]);
  assert.equal(boot_.viewing.username, 'nil');
  assert.equal(boot_.caps.readOnly, true);
  assert.equal(boot_.libraries.some((p) => p.username === 'nil'), true);
  const denied = await zoe.request('GET', `/?lib=${ids.nil}`);
  assert.equal(denied.status, 302);

  // Tout le monde ('*') ; l'administrateur parcourt tout, mais pas les statistiques.
  await ami.rpc('social:set-grants', [{ viewerId: '*', browse: true }]);
  assert.equal((await zoe.rpcIn(ids.ami, 'lib:tracks')).length, 1);
  const adminView = (await nil.rpc('social:others')).find((p) => p.username === 'zoe');
  assert.deepEqual(adminView.access, { browse: true, compare: false });

  // Retirer le partage coupe l'accès aussitôt.
  await nil.rpc('social:set-grants', []);
  await assert.rejects(ami.rpcIn(ids.nil, 'lib:tracks'), (err) => err.status === 403);
});

test('playlists personnalisées : description, pochette, visibilité', async (t) => {
  const { nil, ami, ids } = await house(t);
  await until(async () => (await nil.rpc('lib:tracks')).length === 3);
  const tracks = await nil.rpc('lib:tracks');
  const pl = await nil.rpc('pl:create', 'Ma sélection', 'manual', null, [tracks[0].id, tracks[1].id]);
  assert.equal(await nil.rpc('pl:update', pl, { name: 'Été', description: 'Pour la route', shared: true }), true);
  const cover = await nil.rpc('pl:set-cover', pl, { data: pngDataUrl() });
  assert.equal(cover.ok, true);
  assert.match(cover.cover, /^[0-9a-f]{20}$/);
  assert.equal((await nil.request('GET', `/cover/${cover.cover}.jpg`)).status, 200);
  const listed = (await nil.rpc('lib:playlists')).playlists[0];
  assert.deepEqual([listed.name, listed.description, listed.shared, listed.cover], ['Été', 'Pour la route', true, cover.cover]);

  // Une image qui n'en est pas une est refusée ; null revient à la mosaïque.
  assert.equal((await nil.rpc('pl:set-cover', pl, { data: 'data:image/png;base64,AAAA' })).ok, false);
  assert.equal((await nil.rpc('pl:set-cover', pl, { data: 'pas une image' })).ok, false);
  assert.equal((await nil.rpc('pl:set-cover', pl, { trackId: 999999 })).ok, false);
  assert.equal((await nil.rpc('pl:set-cover', pl, null)).cover, null);

  // Visible des autres seulement si partagée, et seulement avec le droit de parcourir.
  const secret = await nil.rpc('pl:create', 'Perso');
  await nil.rpc('social:set-grants', [{ viewerId: ids.ami, browse: true }]);
  const seen = await ami.rpcIn(ids.nil, 'lib:playlists');
  assert.deepEqual(seen.playlists.map((p) => p.name), ['Été']);
  assert.equal(seen.items.length, 2);
  assert.equal(seen.playlists.some((p) => p.id === secret), false);
  const profile = await ami.rpc('social:profile', ids.nil);
  assert.deepEqual(profile.playlists.map((p) => [p.name, p.count]), [['Été', 2]]);
});

test('profil : nom affiché, présentation, couleur, avatar', async (t) => {
  const { ami, nil, ids } = await house(t);
  const saved = await ami.rpc('social:set-profile', { displayName: '  Amélie ', bio: 'Du rock, du jazz.', color: '#ff6a5f' });
  assert.deepEqual([saved.displayName, saved.name, saved.bio, saved.color], ['Amélie', 'Amélie', 'Du rock, du jazz.', '#ff6a5f']);
  await assert.rejects(ami.rpc('social:set-profile', { color: 'rouge' }), (err) => err.status === 400);

  const withAvatar = await ami.rpc('social:set-profile', { avatar: { data: pngDataUrl(80, 40) } });
  assert.match(withAvatar.avatar, /^[0-9a-f]{20}$/);
  const res = await nil.request('GET', `/avatar/${withAvatar.avatar}.jpg`);
  assert.equal(res.status, 200);
  assert.equal(res.headers.get('content-type'), 'image/jpeg');
  await assert.rejects(ami.rpc('social:set-profile', { avatar: { data: 'data:image/png;base64,AAAA' } }), (err) => err.status === 400);
  assert.equal((await ami.rpc('social:set-profile', { avatar: null })).avatar, null);

  // Les autres comptes voient le nom affiché.
  const seen = (await nil.rpc('social:others')).find((p) => p.id === ids.ami);
  assert.equal(seen.name, 'Amélie');
  assert.equal((await ami.json('GET', '/api/auth/me')).body.user.name, 'Amélie');
});

test('comparer ses récaps : seulement avec la permission, et sans fuite de l’historique', async (t) => {
  const { nil, ami, ids } = await house(t);
  await assert.rejects(ami.rpc('social:compare', ids.nil, {}), (err) => err.status === 403);
  await assert.rejects(ami.rpc('social:compare', ami_id(ids), {}), (err) => err.status === 400);

  await nil.rpc('social:set-grants', [{ viewerId: ids.ami, browse: false, compare: true }]);
  const out = await ami.rpc('social:compare', ids.nil, { from: '2000-01-01', to: '2999-12-31' });
  assert.equal(out.with.username, 'nil');
  assert.equal(typeof out.score, 'number');
  assert.ok(out.me.topArtists && out.other.topArtists);
  assert.equal(JSON.stringify(out).includes('played_at'), false);
  // Comparer ne donne pas accès à la bibliothèque.
  await assert.rejects(ami.rpcIn(ids.nil, 'lib:tracks'), (err) => err.status === 403);
  const profile = await ami.rpc('social:profile', ids.nil);
  assert.equal(profile.library, null);
  assert.ok(profile.stats && profile.stats.year);
});

function ami_id(ids) { return ids.ami; }

test('comparaison : goûts communs, indice de ressemblance', () => {
  const artist = (name, plays) => ({ name, plays, seconds: plays * 200, cover: null });
  const track = (title, artistName, plays) => ({ title, artist: artistName, album: '', cover: null, plays });
  const stats = (artists, tracks, genres, plays) => ({
    range: { from: '2026-01-01', to: '2026-12-31' },
    years: [2026],
    streak: { current: 1, longest: 3 },
    music: { plays, seconds: plays * 200, tracks: tracks.length, artists: artists.length, albums: 1, completed: plays, topArtists: artists, topTracks: tracks, topAlbums: [], genres, byWeekday: [0, 0, 0, 0, 0, 0, 0], byHour: Array(24).fill(0) },
    video: { seconds: 0, titles: 0, finished: 0 },
  });
  const a = stats([artist('Daft Punk', 10), artist('Björk', 4)], [track('One More Time', 'Daft Punk', 6)], [{ name: 'Électro', plays: 10, seconds: 1 }], 14);
  const b = stats([artist('daft punk', 8), artist('Mono', 2)], [track('one more time', 'DAFT PUNK', 2)], [{ name: 'electro', plays: 8, seconds: 1 }], 10);
  const same = compare(a, b);
  assert.equal(same.shared.artists[0].name, 'Daft Punk', 'les noms se recoupent sans tenir compte de la casse ni des accents');
  assert.equal(same.shared.tracks.length, 1);
  assert.equal(same.shared.genres.length, 1);
  assert.ok(same.score > 40 && same.score < 100, `score ${same.score}`);

  const alone = compare(a, stats([artist('Mono', 5)], [track('Ghost', 'Mono', 5)], [{ name: 'Classique', plays: 5, seconds: 1 }], 5));
  assert.equal(alone.shared.artists.length, 0);
  assert.equal(alone.score, 0);
  assert.equal(compare(a, stats([], [], [], 0)).score, 0, 'rien à comparer : 0');
  assert.equal(compare(a, a).score, 100);
});

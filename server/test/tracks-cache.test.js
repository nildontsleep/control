'use strict';

// La liste des morceaux d'un compte : construite à part, gardée prête, refaite dès que la bibliothèque change.

const test = require('node:test');
const assert = require('node:assert/strict');
const { boot, client, until } = require('./helpers');

test('lib:tracks : réponse gardée en cache tant que rien ne change, refaite après une écoute ou une playlist', async (t) => {
  const s = await boot();
  t.after(s.close);
  const admin = client(s.base);
  await admin.json('POST', '/api/setup/init', { username: 'nil', password: 'long enough password', libraryFolders: [s.music] });
  const tracks = await until(async () => { const l = await admin.rpc('lib:tracks'); return l.length === 3 ? l : null; });
  assert.deepEqual(Object.keys(tracks[0]).sort(), ['addedAt', 'album', 'albumArtist', 'artist', 'bitrate', 'codec', 'cover', 'discNo', 'duration', 'genre', 'height', 'id', 'kind', 'lastPlayed', 'loudness', 'path', 'playCount', 'sourceUrl', 'size', 'title', 'trackNo', 'truePeak', 'width', 'year'].sort());

  // Sans autre changement : la même réponse, sans refaire la requête.
  await until(async () => { const before = { ...s.hub.cacheStats }; await admin.rpc('lib:tracks'); return s.hub.cacheStats.hits > before.hits; });
  const misses = s.hub.cacheStats.misses;
  const again = await admin.rpc('lib:tracks');
  assert.equal(s.hub.cacheStats.misses, misses);
  assert.deepEqual(again, await admin.rpc('lib:tracks'));

  // Une écoute change le compteur : la liste est refaite.
  const play = await admin.rpc('lib:play-start', tracks[0].id, 1);
  assert.ok(play > 0);
  const after = await admin.rpc('lib:tracks');
  assert.ok(s.hub.cacheStats.misses > misses);
  assert.equal(after.find((x) => x.id === tracks[0].id).playCount, 1);

  // Un client qui ne sait pas lire le gzip reçoit le même contenu décompressé.
  const raw = await fetch(`${s.base}/api/rpc/lib%3Atracks`, { method: 'POST', headers: { 'content-type': 'application/json', 'x-control': '1', 'accept-encoding': 'identity', cookie: admin.cookie }, body: '{"args":[]}' });
  assert.equal(raw.headers.get('content-encoding'), null);
  assert.equal((await raw.json()).result.length, 3);

  // Une playlist aussi modifie la base : elle ne sert jamais une liste périmée.
  const hits = s.hub.cacheStats.hits;
  await admin.rpc('pl:create', 'Essai', 'manual', null, [tracks[1].id]);
  await admin.rpc('lib:tracks');
  assert.equal(s.hub.cacheStats.hits, hits, 'après un changement : un nouveau calcul');
});

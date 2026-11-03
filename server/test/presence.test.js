'use strict';

// « En écoute » : montré seulement à ceux qui ont un partage de bibliothèque avec le compte, et seulement s'il l'a choisi.

const test = require('node:test');
const assert = require('node:assert/strict');
const { boot, client, until } = require('./helpers');

const PASSWORD = 'long enough password';

test('en écoute : désactivé par défaut, visible des seuls comptes partagés, temps réel, fin et expiration', async (t) => {
  const s = await boot({ noCommon: true });
  t.after(s.close);
  const nil = client(s.base);
  await nil.json('POST', '/api/setup/init', { username: 'nil', password: PASSWORD, libraryFolders: [s.music] });
  const ids = {};
  for (const name of ['ami', 'zoe']) ids[name] = (await nil.rpc('srv:users:create', { username: name, password: PASSWORD })).id;
  const ami = client(s.base);
  await ami.json('POST', '/api/auth/login', { username: 'ami', password: PASSWORD });
  const zoe = client(s.base);
  await zoe.json('POST', '/api/auth/login', { username: 'zoe', password: PASSWORD });
  ids.nil = (await nil.json('GET', '/api/auth/me')).body.user.id;
  const tracks = await until(async () => { const l = await nil.rpc('lib:tracks'); return l.length === 3 ? l : null; });
  const listenersOf = async (who) => Object.fromEntries((await who.rpc('social:others')).map((p) => [p.username, p.listening]));

  // Partage de la bibliothèque avec ami seulement.
  await nil.rpc('social:set-grants', [{ viewerId: ids.ami, browse: true, compare: false }]);
  const amiEntry = s.hub.entry(s.hub.accounts.byId(ids.ami));
  const events = [];
  const emit = amiEntry.emit;
  amiEntry.emit = (channel, payload, ...rest) => { events.push([channel, payload]); return emit.call(amiEntry, channel, payload, ...rest); };

  // Par défaut rien n'est montré.
  assert.equal((await nil.rpc('social:me')).shareListening, false);
  await nil.rpc('lib:play-start', tracks[0].id, 5);
  assert.deepEqual((await listenersOf(ami)).nil, null);

  // L'option activée : le morceau suivant apparaît pour ami, pas pour zoe, avec ce qu'il faut pour l'afficher.
  assert.equal(await nil.rpc('social:set-listening', true), true);
  const playId = await nil.rpc('lib:play-start', tracks[1].id, 5);
  const seen = (await listenersOf(ami)).nil;
  assert.deepEqual([seen.trackId, seen.title, seen.duration], [tracks[1].id, tracks[1].title, tracks[1].duration]);
  assert.equal((await listenersOf(zoe)).nil, null, 'zoe n’a pas de partage : rien');
  assert.deepEqual((await listenersOf(nil)).nil, undefined, 'on ne se voit pas soi-même dans la liste');
  assert.ok(events.some(([ch, p]) => ch === 'social:listening' && p.id === ids.nil && p.now?.title === tracks[1].title), 'l’événement part en direct');

  // « Tout le monde » : zoe voit aussi.
  await nil.rpc('social:set-grants', [{ viewerId: '*', browse: true, compare: false }]);
  assert.equal((await listenersOf(zoe)).nil.title, tracks[1].title);
  await nil.rpc('social:set-grants', [{ viewerId: ids.ami, browse: true, compare: false }]);
  assert.equal((await listenersOf(zoe)).nil, null);

  // La fin de l'écoute efface ; l'écoute d'un autre morceau n'efface pas celle-ci.
  await nil.json('POST', '/api/send/lib%3Aplay-end', { args: [playId + 1000, 1, true] });
  assert.ok((await listenersOf(ami)).nil, 'la fin d’une autre écoute ne touche pas celle-ci');
  await nil.json('POST', '/api/send/lib%3Aplay-end', { args: [playId, 5, true] });
  assert.equal((await listenersOf(ami)).nil, null);
  assert.ok(events.some(([ch, p]) => ch === 'social:listening' && p.now === null));

  // Expiration : un morceau dont la pause dépasse sa durée disparaît.
  await nil.rpc('lib:play-start', tracks[2].id, 5);
  assert.ok((await listenersOf(ami)).nil);
  s.hub.presence.now.get(ids.nil).expiresAt = Date.now() - 1;
  s.hub.presence.sweep();
  assert.equal((await listenersOf(ami)).nil, null);

  // Couper l'option efface aussitôt.
  await nil.rpc('lib:play-start', tracks[0].id, 5);
  assert.ok((await listenersOf(ami)).nil);
  await nil.rpc('social:set-listening', false);
  assert.equal((await listenersOf(ami)).nil, null);
  await nil.rpc('lib:play-start', tracks[0].id, 5);
  assert.equal((await listenersOf(ami)).nil, null, 'option coupée : plus rien ne remonte');

  // L'administrateur ne voit pas non plus ce qu'on n'a pas partagé avec lui : seulement ce qui est partagé.
  await nil.rpc('social:set-listening', true);
  const adminSeesAmi = (await listenersOf(nil)).ami;
  assert.equal(adminSeesAmi, null);
});

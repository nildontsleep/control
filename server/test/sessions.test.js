'use strict';

// Sessions ouvertes d'un compte : les voir, en fermer une, fermer toutes les autres ; jamais celles d'un autre compte.

const test = require('node:test');
const assert = require('node:assert/strict');
const { boot, client } = require('./helpers');

const PASSWORD = 'long enough password';

test('sessions : liste, fermeture d’une session, des autres sessions, cloisonnement entre comptes', async (t) => {
  const s = await boot();
  t.after(s.close);
  const phone = client(s.base);
  await phone.json('POST', '/api/setup/init', { username: 'nil', password: PASSWORD, libraryFolders: [s.music] });
  const laptop = client(s.base);
  await laptop.request('POST', '/api/auth/login', { username: 'nil', password: PASSWORD }, { 'user-agent': 'Mozilla/5.0 (X11; Linux x86_64) Firefox/130.0' });
  const tablet = client(s.base);
  await tablet.request('POST', '/api/auth/login', { username: 'nil', password: PASSWORD }, { 'user-agent': 'Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) Safari/605.1.15' });

  const list = await laptop.rpc('srv:me:sessions');
  assert.equal(list.length, 3);
  assert.equal(list.filter((x) => x.current).length, 1);
  assert.match(list.find((x) => x.current).userAgent, /Firefox/);
  assert.ok(list.every((x) => /^[0-9a-f]{16}$/.test(x.id) && x.lastSeen > 0));

  // Un autre compte ne voit ni ne ferme les sessions du premier.
  await phone.rpc('srv:users:create', { username: 'ami', password: PASSWORD });
  const ami = client(s.base);
  await ami.json('POST', '/api/auth/login', { username: 'ami', password: PASSWORD });
  assert.equal((await ami.rpc('srv:me:sessions')).length, 1);
  assert.equal(await ami.rpc('srv:me:revoke-session', list[0].id), false);
  assert.equal((await laptop.rpc('srv:me:sessions')).length, 3);

  // Fermer la session de la tablette : elle perd l'accès, les autres restent.
  const tabletId = list.find((x) => /iPad/.test(x.userAgent)).id;
  assert.equal(await laptop.rpc('srv:me:revoke-session', tabletId), true);
  await assert.rejects(tablet.rpc('lib:tracks'), (err) => err.status === 401);
  assert.equal((await laptop.rpc('srv:me:sessions')).length, 2);

  // Fermer toutes les autres : seule celle du portable reste.
  assert.equal(await laptop.rpc('srv:me:revoke-others'), 1);
  await assert.rejects(phone.rpc('lib:tracks'), (err) => err.status === 401);
  assert.deepEqual((await laptop.rpc('srv:me:sessions')).map((x) => x.current), [true]);
});

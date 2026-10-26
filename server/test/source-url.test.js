'use strict';

// Adresses de serveurs saisies dans l'application de bureau (src/main/source-url.js).

const test = require('node:test');
const assert = require('node:assert/strict');
const { candidates } = require('../../src/main/source-url');

test('une adresse complète est gardée telle quelle (origine seulement)', () => {
  assert.deepEqual(candidates('https://music.example.com/'), ['https://music.example.com']);
  assert.deepEqual(candidates('http://192.168.1.20:5180/admin?x=1#y'), ['http://192.168.1.20:5180']);
  assert.deepEqual(candidates('http://moi:secret@nas.local:5180'), ['http://nas.local:5180']);
});

test('sans schéma : https d’abord pour un nom de domaine, http d’abord pour une adresse locale', () => {
  assert.deepEqual(candidates('music.example.com'), ['https://music.example.com', 'http://music.example.com']);
  assert.deepEqual(candidates('192.168.1.20:5180'), ['http://192.168.1.20:5180', 'https://192.168.1.20:5180']);
  assert.deepEqual(candidates('localhost:5180'), ['http://localhost:5180', 'https://localhost:5180']);
  assert.deepEqual(candidates('nas'), ['http://nas', 'https://nas']);
  assert.deepEqual(candidates('maison.local'), ['http://maison.local', 'https://maison.local']);
  assert.deepEqual(candidates('[::1]:5180'), ['http://[::1]:5180', 'https://[::1]:5180']);
});

test('adresses refusées', () => {
  assert.throws(() => candidates(''), /adresse/i);
  assert.throws(() => candidates('   '), /adresse/i);
  assert.throws(() => candidates('ftp://example.com'), /http/);
  assert.throws(() => candidates('file:///etc/passwd'), /http/);
});

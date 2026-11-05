'use strict';

// Découpage des étiquettes d'artistes (« A, B feat. C @pseudo ») : une fiche par artiste, sans doublon ni pseudo.

const test = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const { pathToFileURL } = require('node:url');

const load = () => import(pathToFileURL(path.join(__dirname, '../../src/renderer/js/artists.js')).href);

test('séparateurs, doublons, pseudos et variantes de la vraie bibliothèque', async () => {
  const { splitArtists } = await load();
  const cases = [
    ['COEURCO', ['COEURCO']],
    ['Ptite Soeur, neophron, esone', ['Ptite Soeur', 'neophron', 'esone']],
    ['COEURCO, Bi2l, COEURCO', ['COEURCO', 'Bi2l']],
    ['COEURCO, COEURCO @1coeurco, S1 @s1_ski', ['COEURCO', 'S1']],
    ['COEURCO, COEURCO ig @1coeurco, Bi (t3rmina) ig @t3rminadoo', ['COEURCO', 'Bi (t3rmina)']],
    ['COEURCO, COEURCO (@1coeurco), Bi (t3rmina) - (@t3rminadoo)', ['COEURCO', 'Bi (t3rmina)']],
    ['COEURCO, Idée Noire., COEURCO, Idée Noire.', ['COEURCO', 'Idée Noire.']],
    ['Ecolierperver， Ziontogolais', ['Ecolierperver', 'Ziontogolais']],
    ['ELIESG， BU$HI', ['ELIESG', 'BU$HI']],
    ['Ptite Soeur, neophron, Rosaliedu38, rosaliedu38', ['Ptite Soeur', 'neophron', 'Rosaliedu38']],
    ['Aya feat. Kiro', ['Aya', 'Kiro']],
    ['Aya (ft. Kiro; Zoé)', ['Aya', 'Kiro', 'Zoé']],
    ['Simon & Garfunkel', ['Simon & Garfunkel']],
    ['nil メ', ['nil メ']],
    ['', []],
    [null, []],
  ];
  for (const [input, expected] of cases) assert.deepEqual(splitArtists(input), expected, JSON.stringify(input));
});

test('clé de regroupement : sans casse, accents ni point final', async () => {
  const { artistKey } = await load();
  assert.equal(artistKey('Idée Noire.'), artistKey('idee noire'));
  assert.equal(artistKey('  COEURCO '), 'coeurco');
});

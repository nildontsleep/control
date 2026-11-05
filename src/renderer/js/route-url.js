// Adresse d'une vue : #/album/xxxx, #/playlist/12… Le routeur de l'application garde son propre historique ; l'adresse suit
// la vue affichée (liens partageables, rechargement sur la même vue) et sert à rouvrir une vue depuis un lien.

import { lib } from './data.js';

// Vues qu'on sait mettre en adresse. Les autres (réglages, feuilles…) ne sont pas des vues.
const NAMED = new Set(['songs', 'albums', 'artists', 'videos', 'downloads', 'watch', 'anime', 'mylist', 'stats', 'people', 'profile', 'compare',
  'playlist', 'album', 'artist', 'search', 'title', 'shelf', 'watching']);
const NUMERIC = new Set(['playlist']);

/** Identifiant court et stable d'un album : sa clé contient le dossier du serveur, qu'on ne met pas dans une adresse. */
export function albumId(key) {
  let h1 = 0x811c9dc5;
  let h2 = 0x01000193;
  for (let i = 0; i < key.length; i++) {
    h1 = Math.imul(h1 ^ key.charCodeAt(i), 0x01000193);
    h2 = Math.imul(h2 + key.charCodeAt(i), 0x85ebca6b) ^ (h2 >>> 13);
  }
  return ((h1 >>> 0).toString(36) + (h2 >>> 0).toString(36)).slice(0, 12);
}

/** { name, param } → '#/name/param', ou null si cette vue n'a pas d'adresse. */
export function routeToHash(route) {
  if (!route || !NAMED.has(route.name)) return null;
  let param = route.param;
  if (route.name === 'album' && typeof param === 'string') param = albumId(param);
  if (param === null || param === undefined || param === '') return `#/${route.name}`;
  if (typeof param === 'object') return null;
  return `#/${route.name}/${encodeURIComponent(String(param))}`;
}

/** '#/album/xxxx' → { name, param } (bibliothèque chargée requise pour les albums), ou null. */
export function hashToRoute(hash) {
  const m = /^#\/([a-z]+)(?:\/(.*))?$/.exec(String(hash || ''));
  if (!m || !NAMED.has(m[1])) return null;
  const name = m[1];
  let param = m[2] === undefined ? null : decodeURIComponent(m[2]);
  if (name === 'album' && param) {
    const album = lib.albums.find((a) => albumId(a.key) === param);
    if (!album) return null;
    param = album.key;
  } else if (NUMERIC.has(name)) {
    if (!/^\d+$/.test(param || '')) return null;
    param = Number(param);
  } else if (name === 'artist' && param && !lib.artistByKey.has(param)) return null;
  if (!param && ['album', 'artist', 'playlist', 'title', 'shelf', 'watching', 'compare'].includes(name)) return null;
  return { name, param };
}

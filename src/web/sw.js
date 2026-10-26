// Service worker : garde l'enveloppe de l'application (scripts, styles, polices) pour un démarrage instantané et un
// écran correct hors connexion. La musique, les pochettes et les appels au serveur ne passent jamais par le cache.
const VERSION = 'control-shell-1';
const SHELL = /^\/(js|styles|assets|web)\//;

self.addEventListener('install', (event) => {
  self.skipWaiting();
  event.waitUntil(caches.open(VERSION));
});

self.addEventListener('activate', (event) => {
  event.waitUntil((async () => {
    for (const key of await caches.keys()) if (key !== VERSION) await caches.delete(key);
    await self.clients.claim();
  })());
});

self.addEventListener('fetch', (event) => {
  const { request } = event;
  if (request.method !== 'GET') return;
  const url = new URL(request.url);
  if (url.origin !== self.location.origin || !SHELL.test(url.pathname)) return;
  // Réseau d'abord (le code change à chaque mise à jour), le cache en secours.
  event.respondWith((async () => {
    const cache = await caches.open(VERSION);
    try {
      const fresh = await fetch(request);
      if (fresh.ok) cache.put(request, fresh.clone());
      return fresh;
    } catch (err) {
      const hit = await cache.match(request);
      if (hit) return hit;
      throw err;
    }
  })());
});

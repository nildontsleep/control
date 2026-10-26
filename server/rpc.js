'use strict';

// Les commandes que la page envoie au serveur : les mêmes canaux que l'IPC d'Electron (le noyau les enregistre tels
// quels), plus quelques canaux « srv: » propres au serveur. Chaque appel passe par un contrôle des droits.

const { httpError } = require('./auth');

const ADMIN_ONLY = new Set([
  'lib:remove-folder', 'tools:install', 'tools:update', 'tags:write', 'tags:undo', 'lyrics:save-file',
  'srv:browse', 'srv:add-folder', 'srv:remove-folder', 'srv:settings', 'srv:users:list', 'srv:users:create',
  'srv:users:update', 'srv:users:password', 'srv:users:delete', 'srv:rescan',
]);
const DOWNLOADS = new Set(['dl:add', 'dl:info', 'dl:search', 'dl:cancel', 'dl:retry', 'dl:remove', 'dl:clear']);
const DENIED = new Set(['lib:add-paths']); // des chemins du poste de l'utilisateur ne veulent rien dire pour un serveur
// Hôtes de pochettes que le serveur accepte de télécharger (résultats de la recherche de pochettes).
const COVER_HOSTS = [/(^|\.)mzstatic\.com$/i, /(^|\.)dzcdn\.net$/i, /(^|\.)deezer\.com$/i, /(^|\.)coverartarchive\.org$/i, /(^|\.)archive\.org$/i];
// Réglages qui décrivent le serveur : seul l'administrateur y touche, et ils s'appliquent à tous.
const SERVER_KEYS = ['libraryFolders', 'downloadDir'];

function allowed(hub, user, channel, args) {
  if (DENIED.has(channel)) return 'Indisponible sur la version web.';
  if (ADMIN_ONLY.has(channel) && user.role !== 'admin') return 'Réservé à l’administrateur.';
  if (DOWNLOADS.has(channel) && user.role !== 'admin' && !hub.settings.allowUserDownloads) return 'Les téléchargements sont réservés à l’administrateur.';
  if (channel === 'lib:remove-tracks' && args[1] && user.role !== 'admin') return 'Seul l’administrateur peut supprimer des fichiers.';
  return null;
}

/** Ajoute les canaux propres au serveur et remplace ceux dont le sens change quand la bibliothèque est partagée. */
function install(hub, entry, registry) {
  const { handlers, listeners } = registry;
  const publicUser = (u) => ({ id: u.id, username: u.username, role: u.role, disabled: !!u.disabled, createdAt: u.createdAt ?? u.created_at, lastLogin: u.lastLogin ?? u.last_login });

  // Les dossiers sont ceux du serveur : un retrait passe par l'administrateur et vaut pour tout le monde.
  handlers.set('lib:remove-folder', async (e, folder) => hub.removeFolder(String(folder || '')));

  const baseSet = listeners.get('settings:set');
  listeners.set('settings:set', (e, patch) => {
    if (!patch || typeof patch !== 'object' || Array.isArray(patch)) return;
    const clean = { ...patch };
    const server = {};
    for (const key of SERVER_KEYS) {
      if (!(key in clean)) continue;
      if (e.user.role === 'admin') server[key] = clean[key];
      delete clean[key];
    }
    if (clean.dl && typeof clean.dl === 'object') {
      clean.dl = { ...clean.dl };
      delete clean.dl.cookiesFile; // chemin du poste de l'utilisateur
    }
    if (Object.keys(server).length) hub.patchSettings(server);
    if (Object.keys(clean).length) baseSet(e, clean);
  });

  const baseCover = handlers.get('tags:stage-cover');
  handlers.set('tags:stage-cover', (e, src) => {
    const { url, data } = src && typeof src === 'object' ? src : {};
    if (typeof data === 'string' && data) return baseCover(e, { data });
    let host = null;
    try {
      const u = new URL(url);
      if (u.protocol === 'https:') host = u.hostname;
    } catch { /* adresse invalide */ }
    if (!host || !COVER_HOSTS.some((re) => re.test(host))) return { ok: false, message: 'Source de pochette refusée.' };
    return baseCover(e, { url });
  });

  // ─── Serveur ───────────────────────────────────────────────────────────────
  handlers.set('srv:info', (e) => ({
    name: hub.settings.serverName,
    role: e.user.role,
    registrationEnabled: hub.settings.registrationEnabled,
    allowUserDownloads: hub.settings.allowUserDownloads,
    folders: hub.settings.libraryFolders,
    downloadDir: hub.settings.downloadDir,
    hasTmdbKey: !!hub.settings.tmdbKey,
    users: e.user.role === 'admin' ? hub.accounts.count() : undefined,
  }));
  handlers.set('srv:browse', (e, dir) => hub.browse(dir));
  handlers.set('srv:add-folder', (e, dir) => hub.addFolder(dir));
  handlers.set('srv:remove-folder', (e, dir) => hub.removeFolder(String(dir || '')));
  handlers.set('srv:rescan', () => { hub.rescanAll(); return true; });
  handlers.set('srv:settings', (e, patch) => {
    hub.patchSettings(patch && typeof patch === 'object' ? patch : {});
    return { ...hub.settings };
  });
  handlers.set('srv:settings:get', () => ({ ...hub.settings }));

  handlers.set('srv:users:list', () => hub.accounts.list());
  handlers.set('srv:users:create', async (e, body) => publicUser(await hub.accounts.create({ username: body?.username, password: body?.password, role: body?.role })));
  handlers.set('srv:users:update', (e, id, body) => {
    if (id === e.user.id && body && (body.disabled || (body.role && body.role !== 'admin'))) throw httpError(409, 'self', 'Vous ne pouvez pas retirer vos propres droits.');
    hub.accounts.update(String(id), body || {});
    if (body?.disabled) hub.dropUser(String(id));
    return true;
  });
  handlers.set('srv:users:password', async (e, id, password) => { await hub.accounts.setPassword(String(id), password); hub.dropUser(String(id)); return true; });
  handlers.set('srv:users:delete', (e, id) => {
    if (id === e.user.id) throw httpError(409, 'self', 'Vous ne pouvez pas supprimer votre propre compte.');
    hub.accounts.remove(String(id));
    hub.forgetUser(String(id));
    return true;
  });
  handlers.set('srv:me:password', async (e, current, next) => {
    const user = await hub.accounts.check(e.user.username, current);
    if (!user) throw httpError(403, 'wrong_password', 'Mot de passe actuel incorrect.');
    await hub.accounts.setPassword(e.user.id, next);
    return true;
  });
}

/** Les commandes d'un noyau : le noyau s'y enregistre, puis le serveur ajoute et corrige les siennes. */
function createRegistry(hub, entry) {
  const registry = {
    handlers: new Map(),
    listeners: new Map(),
    handle(channel, fn) { this.handlers.set(channel, fn); },
    on(channel, fn) { this.listeners.set(channel, fn); },
  };
  const forced = () => ({ libraryFolders: hub.settings.libraryFolders, downloadDir: hub.settings.downloadDir });
  entry.core.register(registry, {
    scanRoots: () => hub.roots(),
    applySettingEffects: () => {},
    emitOthers: (e, channel, payload) => entry.emit(channel, payload, e.sender),
    onReset: () => {
      entry.core.store.patch(forced());
      entry.emit('settings:changed', forced());
    },
  });
  install(hub, entry, registry);
  return registry;
}

async function invoke(hub, entry, user, channel, args, clientId) {
  const fn = entry.registry.handlers.get(channel);
  if (!fn) throw httpError(404, 'unknown_channel', `Commande inconnue : ${channel}`);
  const refusal = allowed(hub, user, channel, args);
  if (refusal) throw httpError(403, 'forbidden', refusal);
  return fn({ sender: clientId, user }, ...args);
}

function send(hub, entry, user, channel, args, clientId) {
  const fn = entry.registry.listeners.get(channel);
  if (!fn) throw httpError(404, 'unknown_channel', `Commande inconnue : ${channel}`);
  const refusal = allowed(hub, user, channel, args);
  if (refusal) throw httpError(403, 'forbidden', refusal);
  fn({ sender: clientId, user }, ...args);
}

module.exports = { createRegistry, invoke, send, allowed };

'use strict';

// Les commandes que la page envoie au serveur : les mêmes canaux que l'IPC d'Electron (le noyau les enregistre tels
// quels), plus quelques canaux « srv: » propres au serveur. Chaque appel passe par un contrôle des droits.

const { httpError } = require('./auth');
const { compare } = require('./compare');

const ADMIN_ONLY = new Set([
  'lib:remove-folder', 'tools:install', 'tools:update', 'tags:write', 'tags:undo', 'lyrics:save-file',
  'srv:browse', 'srv:add-folder', 'srv:remove-folder', 'srv:settings', 'srv:users:list', 'srv:users:create',
  'srv:users:update', 'srv:users:password', 'srv:users:delete', 'srv:rescan', 'srv:me:add-folder',
  'srv:users:detail', 'srv:users:add-folder', 'srv:users:remove-folder', 'srv:users:personal-folder', 'srv:users:set-grants',
]);
const DOWNLOADS = new Set(['dl:add', 'dl:info', 'dl:search', 'dl:cancel', 'dl:retry', 'dl:remove', 'dl:clear']);
const DENIED = new Set(['lib:add-paths']); // des chemins du poste de l'utilisateur ne veulent rien dire pour un serveur
// Hôtes de pochettes que le serveur accepte de télécharger (résultats de la recherche de pochettes).
const COVER_HOSTS = [/(^|\.)mzstatic\.com$/i, /(^|\.)dzcdn\.net$/i, /(^|\.)deezer\.com$/i, /(^|\.)coverartarchive\.org$/i, /(^|\.)archive\.org$/i];
// Réglages qui décrivent le serveur ou les dossiers de ce compte : ils se règlent par les commandes du serveur, pas par les réglages.
const SERVER_KEYS = ['libraryFolders', 'commonFolders', 'downloadDir'];
// Dans la bibliothèque d'un autre compte (lecture seule) : rien de ce qui la modifierait ou la ferait modifier.
const READ_ONLY = (channel) => /^(pl|tags):/.test(channel) || ['lib:remove-tracks', 'lib:remove-folder', 'lyrics:set', 'lyrics:save-file'].includes(channel);

function allowed(hub, user, channel, args) {
  if (DENIED.has(channel)) return 'Indisponible sur la version web.';
  if (ADMIN_ONLY.has(channel) && user.role !== 'admin') return 'Réservé à l’administrateur.';
  if (DOWNLOADS.has(channel) && user.role !== 'admin' && !hub.settings.allowUserDownloads) return 'Les téléchargements sont réservés à l’administrateur.';
  if (channel === 'lib:remove-tracks' && args[1] && user.role !== 'admin') return 'Seul l’administrateur peut supprimer des fichiers.';
  return null;
}

/** Les playlists qu'un compte a rendues visibles aux autres : { id, name, description, cover, count, … }. */
function publicPlaylists(entry) {
  const data = entry.core.library.playlists();
  const counts = new Map();
  for (const item of data.items) counts.set(item.playlistId, (counts.get(item.playlistId) || 0) + 1);
  return data.playlists.filter((p) => p.shared).map((p) => ({ ...p, count: counts.get(p.id) || 0 }));
}

/** Ajoute les canaux propres au serveur et remplace ceux dont le sens change quand la bibliothèque est partagée. */
function install(hub, entry, registry) {
  const { handlers, listeners } = registry;
  const publicUser = (u) => ({ id: u.id, username: u.username, role: u.role, disabled: !!u.disabled, createdAt: u.createdAt ?? u.created_at, lastLogin: u.lastLogin ?? u.last_login });

  // Les dossiers sont attribués par l'administrateur : un compte ne règle que les siens via ces commandes.
  handlers.set('lib:remove-folder', async (e, folder) => hub.removeOwnFolder(e.user.id, String(folder || '')));
  handlers.set('srv:me:add-folder', (e, dir) => hub.addOwnFolder(e.user.id, dir));

  const baseSet = listeners.get('settings:set');
  listeners.set('settings:set', (e, patch) => {
    if (!patch || typeof patch !== 'object' || Array.isArray(patch)) return;
    const clean = { ...patch };
    for (const key of SERVER_KEYS) delete clean[key];
    if (clean.dl && typeof clean.dl === 'object') {
      clean.dl = { ...clean.dl };
      delete clean.dl.cookiesFile; // chemin du poste de l'utilisateur
    }
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
    folders: hub.ownFolders(e.user.id),
    commonFolders: hub.commonFolders(),
    downloadDir: hub.downloadDirFor(e.user),
    usersRoot: hub.settings.usersRoot,
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
  handlers.set('srv:users:create', async (e, body) => {
    const user = await hub.accounts.create({ username: body?.username, password: body?.password, role: body?.role });
    if (hub.settings.usersRoot) await hub.ensurePersonalFolder(user).catch((err) => console.error('Dossier du compte :', err.message));
    return publicUser(user);
  });
  handlers.set('srv:users:detail', (e, id) => {
    const user = hub.accounts.byId(String(id));
    if (!user) throw httpError(404, 'not_found', 'Compte introuvable.');
    return { folders: hub.ownFolders(user.id), grants: hub.social.grantsOf(user.id), downloadDir: hub.downloadDirFor(user) };
  });
  handlers.set('srv:users:add-folder', (e, id, dir) => hub.addOwnFolder(String(id), dir));
  handlers.set('srv:users:remove-folder', (e, id, dir) => hub.removeOwnFolder(String(id), String(dir || '')));
  handlers.set('srv:users:personal-folder', async (e, id) => {
    const user = hub.accounts.byId(String(id));
    if (!user) throw httpError(404, 'not_found', 'Compte introuvable.');
    return hub.ensurePersonalFolder(user);
  });
  handlers.set('srv:users:set-grants', (e, id, list) => hub.social.setGrants(String(id), list));
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
  // ─── Profils, partages, comparaison ────────────────────────────────────────
  const ownerOf = (id) => {
    const owner = hub.accounts.byId(String(id || ''));
    if (!owner || owner.disabled) throw httpError(404, 'not_found', 'Compte introuvable.');
    return owner;
  };
  handlers.set('social:me', (e) => ({
    profile: hub.social.profile(hub.accounts.byId(e.user.id)),
    grants: hub.social.grantsOf(e.user.id),
    folders: hub.ownFolders(e.user.id),
    commonFolders: hub.commonFolders(),
  }));
  handlers.set('social:set-profile', (e, patch) => hub.social.setProfile(e.user.id, patch && typeof patch === 'object' ? patch : {}));
  handlers.set('social:set-grants', (e, list) => hub.social.setGrants(e.user.id, list));
  handlers.set('social:others', (e) => hub.social.others(e.user));
  handlers.set('social:profile', async (e, id) => {
    const owner = ownerOf(id);
    const access = hub.social.access(e.user, owner);
    const out = { profile: hub.social.profile(owner), access, library: null, playlists: [], stats: null };
    if (access.browse) {
      const other = hub.entry(owner);
      const roots = hub.ownerRoots(owner);
      const tracks = other.core.library.tracks().filter((t) => t.kind === 'audio' && require('./hub').Hub.inside(roots, t.path));
      out.library = { tracks: tracks.length, albums: new Set(tracks.map((t) => `${t.album}|${t.albumArtist || t.artist}`)).size, artists: new Set(tracks.map((t) => t.albumArtist || t.artist)).size };
      out.playlists = publicPlaylists(other).map(({ id: pid, name, description, cover, count }) => ({ id: pid, name, description, cover, count }));
    }
    if (access.compare) {
      const year = new Date().getFullYear();
      const stats = (owner.id === e.user.id ? entry : hub.entry(owner)).core.stats.get({ from: `${year}-01-01`, to: `${year}-12-31`, limit: 5 });
      out.stats = { year, plays: stats.music.plays, seconds: Math.round(stats.music.seconds || 0), topArtists: stats.music.topArtists.slice(0, 3).map((a) => a.name), streak: stats.streak };
    }
    return out;
  });
  handlers.set('social:compare', async (e, id, range) => {
    const owner = ownerOf(id);
    if (owner.id === e.user.id) throw httpError(400, 'self', 'Choisissez un autre compte.');
    if (!hub.social.access(e.user, owner).compare) throw httpError(403, 'forbidden', 'Ce compte ne partage pas ses statistiques avec vous.');
    const span = { from: String(range?.from || ''), to: String(range?.to || ''), limit: 100 };
    const mine = entry.core.stats.get(span);
    const theirs = hub.entry(owner).core.stats.get(span);
    return { with: hub.social.profile(owner), years: mine.years, months: mine.months, ...compare(mine, theirs) };
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
  entry.core.register(registry, {
    scanRoots: () => hub.rootsFor(entry.id),
    applySettingEffects: () => {},
    emitOthers: (e, channel, payload) => entry.emit(channel, payload, e.sender),
    onReset: () => hub.mirror(entry),
  });
  install(hub, entry, registry);
  return registry;
}

/**
 * La bibliothèque d'un autre compte, vue en lecture seule : ses morceaux (sans les dossiers communs, avec des chemins
 * relatifs), ses playlists publiques et les paroles. Les écoutes ne sont pas comptées, rien ne peut être modifié.
 * view = { owner, entry } (voir app.js : resolveView).
 */
function foreign(hub, user, view, channel, args) {
  const { Hub } = require('./hub');
  const owner = view.owner;
  const roots = hub.ownerRoots(owner);
  if (channel === 'lib:tracks') {
    return view.entry.core.library.tracks()
      .filter((t) => Hub.inside(roots, t.path))
      .map((t) => ({ ...t, path: Hub.relative(roots, t.path), playCount: 0, lastPlayed: null, sourceUrl: null }));
  }
  if (channel === 'lib:playlists') {
    const playlists = publicPlaylists(view.entry);
    const ids = new Set(playlists.map((p) => p.id));
    return { playlists, items: view.entry.core.library.playlists().items.filter((i) => ids.has(i.playlistId)) };
  }
  if (channel === 'lyrics:get') return view.entry.registry.handlers.get('lyrics:get')({ sender: '', user }, ...args);
  if (channel === 'lib:play-start') return 0;
  if (READ_ONLY(channel)) throw httpError(403, 'read_only', `Bibliothèque de ${owner.username} : lecture seule.`);
  return undefined;
}
const FOREIGN_HANDLED = new Set(['lib:tracks', 'lib:playlists', 'lyrics:get', 'lib:play-start']);

async function invoke(hub, entry, user, channel, args, clientId, view = null) {
  if (view) {
    if (FOREIGN_HANDLED.has(channel)) return foreign(hub, user, view, channel, args);
    if (READ_ONLY(channel)) foreign(hub, user, view, channel, args);
  }
  const fn = entry.registry.handlers.get(channel);
  if (!fn) throw httpError(404, 'unknown_channel', `Commande inconnue : ${channel}`);
  const refusal = allowed(hub, user, channel, args);
  if (refusal) throw httpError(403, 'forbidden', refusal);
  return fn({ sender: clientId, user }, ...args);
}

function send(hub, entry, user, channel, args, clientId, view = null) {
  if (view) {
    if (channel === 'lib:play-end') return; // pas d'écoute comptée dans la bibliothèque d'un autre
    if (READ_ONLY(channel)) foreign(hub, user, view, channel, args);
  }
  const fn = entry.registry.listeners.get(channel);
  if (!fn) throw httpError(404, 'unknown_channel', `Commande inconnue : ${channel}`);
  const refusal = allowed(hub, user, channel, args);
  if (refusal) throw httpError(403, 'forbidden', refusal);
  fn({ sender: clientId, user }, ...args);
}

module.exports = { createRegistry, invoke, send, allowed };

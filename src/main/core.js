'use strict';

// Le noyau d'un utilisateur : réglages, base de données, bibliothèque, tags, paroles, statistiques, téléchargements,
// films et séries. L'application de bureau en crée un seul ; le serveur web, un par compte.
// Rien ici ne touche à Electron : les fenêtres, boîtes de dialogue et le protocole app:// restent dans main.js, et le
// serveur apporte ses propres équivalents.

const fs = require('node:fs');
const path = require('node:path');
const { Store, isObj } = require('./store');
const { open: openDb } = require('./db');
const { Library } = require('./library');
const { Tools } = require('./tools');
const { Downloads } = require('./downloads');
const { Tags } = require('./tags');
const { Stats } = require('./stats');
const { Lyrics } = require('./lyrics');
const { MediaService } = require('./media');

class Core {
  /**
   * dir : dossier de données de cet utilisateur (réglages, base, vignettes, téléchargements en cours).
   * coversDir : pochettes (identifiées par leur contenu : elles peuvent être partagées entre utilisateurs).
   * tools : outils déjà installés (partagés) ; sinon créés dans dir/bin.
   * emit(channel, payload) : envoie un événement à tous les clients de ce noyau.
   * defaultDownloadDir : dossier des téléchargements quand le réglage est vide.
   * mediaStore : réglages vus par le service films/séries (le serveur y ajoute sa clé TMDB sans l'exposer).
   */
  constructor({ dir, coversDir, tools, emit, defaultDownloadDir, mediaStore }) {
    this.dir = dir;
    this.emit = emit;
    this.defaultDownloadDir = defaultDownloadDir;
    fs.mkdirSync(dir, { recursive: true });
    this.store = new Store(path.join(dir, 'settings.json'));
    this.db = openDb(path.join(dir, 'library.db'));
    this.library = new Library({ db: this.db, coversDir: coversDir || path.join(dir, 'covers'), thumbsDir: path.join(dir, 'thumbs'), emit });
    this.tags = new Tags({ library: this.library });
    this.stats = new Stats(this.db);
    this.lyrics = new Lyrics({ db: this.db, library: this.library, tags: this.tags });
    this.tools = tools || new Tools({ binDir: path.join(dir, 'bin'), tmpDir: path.join(dir, 'tmp'), emit });
    this.downloads = new Downloads({ tools: this.tools, library: this.library, store: this.store, file: path.join(dir, 'downloads.json'), emit });
    this.downloads.defaultDir = defaultDownloadDir;
    this.media = new MediaService({ db: this.db, store: mediaStore || this.store, emit });
  }

  /** Ferme ce que le noyau garde ouvert (serveur : utilisateur inactif). */
  close() {
    this.downloads.killAll();
    this.store.flush();
    try { this.db.close(); } catch { /* déjà fermée */ }
  }

  /**
   * Enregistre les commandes qui ne dépendent ni d'une fenêtre ni d'une boîte de dialogue.
   * ipc : { handle(channel, (event, ...args) => any), on(channel, (event, ...args) => void) } — ipcMain convient.
   * host : { scanRoots(), applySettingEffects(patch), emitOthers(event, channel, payload), onReset() }
   */
  register(ipc, host) {
    const { store, library, lyrics, stats, tags, tools, downloads, media, emit } = this;
    const handle = (channel, fn) => ipc.handle(channel, (event, ...args) => fn(event, ...args));

    // ─── Réglages ─────────────────────────────────────────────────────────────
    ipc.on('settings:set', (e, patch) => {
      if (!isObj(patch)) return;
      store.patch(patch);
      host.applySettingEffects(patch);
      host.emitOthers(e, 'settings:changed', patch);
    });
    ipc.on('settings:reset', () => {
      store.reset();
      emit('settings:reset', null);
      host.onReset();
    });

    // ─── Bibliothèque ─────────────────────────────────────────────────────────
    handle('lib:tracks', () => library.tracks());
    handle('lib:playlists', () => library.playlists());
    handle('lib:scan', () => library.scan(host.scanRoots()));
    handle('lib:add-paths', (e, paths) => library.addPaths((paths || []).filter((p) => typeof p === 'string')));
    handle('lib:remove-folder', async (e, folder) => {
      const folders = (store.get('libraryFolders', []) || []).filter((f) => f !== folder);
      store.patch({ libraryFolders: folders });
      emit('settings:changed', { libraryFolders: folders });
      host.applySettingEffects({ libraryFolders: folders }, { scan: false });
      // Retire de la bibliothèque les morceaux de ce dossier (les fichiers restent intacts).
      const prefix = path.resolve(folder).toLowerCase() + path.sep;
      const ids = library.tracks().filter((t) => t.path.toLowerCase().startsWith(prefix)).map((t) => t.id);
      if (ids.length) await library.removeTracks(ids);
      return folders;
    });
    handle('lib:remove-tracks', (e, ids, deleteFiles) => library.removeTracks(ids, { deleteFiles: !!deleteFiles }));
    handle('lib:play-start', (e, id, duration) => library.playStart(id, duration));
    ipc.on('lib:play-end', (e, playId, seconds, completed) => library.playEnd(playId, seconds, completed));

    // ─── Paroles ──────────────────────────────────────────────────────────────
    const lyr = (fn) => async (e, id, ...rest) => {
      if (!Number.isInteger(id)) return { ok: false, message: 'Morceau invalide.' };
      try {
        return { ok: true, data: await fn(id, ...rest) };
      } catch (err) {
        return { ok: false, message: String(err?.message || err), code: err?.code || null };
      }
    };
    handle('lyrics:get', lyr((id, refresh) => lyrics.get(id, { refresh: !!refresh })));
    handle('lyrics:set', lyr((id, text) => lyrics.set(id, String(text || '').slice(0, 200000))));
    handle('lyrics:save-file', lyr((id) => lyrics.saveSidecar(id)));

    // ─── Statistiques ─────────────────────────────────────────────────────────
    handle('stats:get', (e, range) => {
      try {
        return { ok: true, data: stats.get({ from: String(range?.from || ''), to: String(range?.to || '') }) };
      } catch (err) {
        return { ok: false, message: String(err?.message || err) };
      }
    });

    // ─── Éditeur de tags ──────────────────────────────────────────────────────
    const wrap = async (fn) => {
      try {
        return { ok: true, data: await fn() };
      } catch (err) {
        return { ok: false, message: String(err?.message || err) };
      }
    };
    const ids = (list) => (Array.isArray(list) ? list.filter(Number.isInteger).slice(0, 5000) : []);
    handle('tags:read', (e, list) => wrap(() => tags.read(ids(list))));
    handle('tags:write', (e, edits) => wrap(() => tags.write((Array.isArray(edits) ? edits : []).filter((x) => x && Number.isInteger(x.id)))));
    handle('tags:undo', () => wrap(() => tags.undo()));
    handle('tags:stage-cover', (e, src) => wrap(() => tags.stageCover(src && typeof src === 'object' ? src : {})));
    handle('tags:search-covers', (e, query) => wrap(() => tags.searchCovers({
      artist: String(query?.artist || '').slice(0, 200),
      album: String(query?.album || '').slice(0, 200),
    })));

    // ─── Playlists ────────────────────────────────────────────────────────────
    handle('pl:create', (e, name, kind, rules, trackIds) => {
      const id = library.createPlaylist(name, kind, rules);
      if (trackIds?.length) library.addToPlaylist(id, trackIds);
      return id;
    });
    handle('pl:rename', (e, id, name) => library.renamePlaylist(id, name));
    handle('pl:rules', (e, id, rules) => library.setPlaylistRules(id, rules));
    handle('pl:delete', (e, id) => library.deletePlaylist(id));
    handle('pl:add', (e, id, trackIds) => library.addToPlaylist(id, trackIds));
    handle('pl:remove', (e, entryIds) => library.removeFromPlaylist(entryIds));
    handle('pl:reorder', (e, id, entryIds) => library.reorderPlaylist(id, entryIds));

    // ─── Outils et téléchargements ────────────────────────────────────────────
    handle('tools:status', () => tools.status());
    handle('tools:install', async () => {
      try {
        await tools.install();
        return { ok: true };
      } catch (err) {
        return { ok: false, error: err.message };
      }
    });
    handle('tools:update', () => tools.update());
    handle('dl:list', () => downloads.list());
    handle('dl:info', async (e, url) => {
      try {
        return { ok: true, info: await downloads.info(url) };
      } catch (err) {
        return { ok: false, error: err.message };
      }
    });
    handle('dl:add', (e, opts) => downloads.add(opts));
    handle('dl:search', async (e, source, query) => {
      try {
        return { ok: true, results: await downloads.search(String(source || ''), String(query || '')) };
      } catch (err) {
        return { ok: false, error: String(err?.message || err) };
      }
    });
    ipc.on('dl:cancel', (e, id) => downloads.cancel(id));
    ipc.on('dl:retry', (e, id) => downloads.retry(id));
    ipc.on('dl:remove', (e, id) => downloads.remove(id));
    ipc.on('dl:clear', () => downloads.clearFinished());

    // ─── Films, séries et animés ──────────────────────────────────────────────
    media.register(ipc);
  }
}

module.exports = { Core };

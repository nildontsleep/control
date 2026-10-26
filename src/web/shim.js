// Pont navigateur : fournit `window.control`, le même objet que le préchargement d'Electron expose à l'interface,
// mais branché sur le serveur (appels HTTP, événements en direct par SSE) et sur les API du navigateur.
// Chargé avant tout le reste ; l'état de démarrage (réglages, compte, droits) est dans <script id="boot">.
(() => {
  'use strict';

  const boot = JSON.parse(document.getElementById('boot').textContent);
  const clientId = boot.clientId;

  // ─── Appels au serveur ─────────────────────────────────────────────────────
  let leaving = false;
  const toLogin = () => {
    if (leaving) return;
    const next = encodeURIComponent(location.pathname + location.search);
    location.replace(`/login?next=${next}`);
  };

  async function call(kind, channel, args) {
    let res;
    try {
      res = await fetch(`/api/${kind}/${encodeURIComponent(channel)}`, {
        method: 'POST',
        headers: { 'content-type': 'application/json', 'x-control': '1', 'x-client': clientId },
        body: JSON.stringify({ args }),
        credentials: 'same-origin',
        keepalive: kind === 'send',
      });
    } catch (err) {
      throw new Error('Serveur injoignable.');
    }
    if (res.status === 401) {
      toLogin();
      throw new Error('Connexion requise.');
    }
    if (kind === 'send') return undefined;
    let body = null;
    try {
      body = await res.json();
    } catch { /* réponse vide ou illisible */ }
    if (!res.ok || !body?.ok) {
      const err = new Error(body?.error?.message || `Erreur ${res.status}`);
      err.code = body?.error?.code;
      throw err;
    }
    return body.result;
  }
  const invoke = (channel) => (...args) => call('rpc', channel, args);
  const send = (channel) => (...args) => { call('send', channel, args).catch(() => {}); };

  // ─── Événements du serveur ─────────────────────────────────────────────────
  const listeners = new Map(); // canal → Set de fonctions
  const source = new EventSource(`/api/events?c=${clientId}`);
  let opened = false;
  const dispatch = (channel, payload) => (listeners.get(channel) || []).forEach((fn) => { try { fn(payload); } catch (err) { console.error(err); } });
  const known = new Set();
  function listen(channel) {
    return (callback) => {
      if (!listeners.has(channel)) listeners.set(channel, new Set());
      listeners.get(channel).add(callback);
      if (!known.has(channel)) {
        known.add(channel);
        source.addEventListener(channel, (ev) => {
          let payload = null;
          try { payload = JSON.parse(ev.data); } catch { /* pas de données */ }
          dispatch(channel, payload);
        });
      }
      return () => listeners.get(channel)?.delete(callback);
    };
  }
  // Après une coupure, on a pu manquer des changements : la bibliothèque se recharge.
  source.addEventListener('ready', () => {
    if (opened) dispatch('lib:changed', { what: 'tracks' });
    opened = true;
  });

  // ─── Fenêtre : plein écran et zoom, à la manière du navigateur ────────────
  let emulatedFullscreen = false;
  const fsState = () => ({ maximized: false, fullscreen: !!document.fullscreenElement || emulatedFullscreen, focused: document.hasFocus() });
  const stateListeners = new Set();
  const pushState = () => stateListeners.forEach((fn) => fn(fsState()));
  for (const ev of ['fullscreenchange', 'focus', 'blur', 'visibilitychange']) (ev === 'fullscreenchange' || ev === 'visibilitychange' ? document : window).addEventListener(ev, pushState);

  function setFullscreen(on) {
    const root = document.documentElement;
    if (on && root.requestFullscreen) {
      root.requestFullscreen({ navigationUI: 'hide' }).catch(() => { emulatedFullscreen = true; pushState(); });
    } else if (!on && document.fullscreenElement) {
      document.exitFullscreen().catch(() => {});
    } else {
      // iPhone et certains navigateurs n'ont pas de plein écran pour une page : on l'émule (interface seule).
      emulatedFullscreen = !!on;
      queueMicrotask(pushState);
    }
    if (!on) {
      emulatedFullscreen = false;
      queueMicrotask(pushState);
    }
  }

  // ─── Téléchargements côté navigateur ──────────────────────────────────────
  function download(name, blob) {
    const url = URL.createObjectURL(blob);
    const a = Object.assign(document.createElement('a'), { href: url, download: name });
    document.body.append(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 10000);
  }

  function pickFile(accept) {
    return new Promise((resolve) => {
      const input = Object.assign(document.createElement('input'), { type: 'file', accept });
      input.style.display = 'none';
      input.addEventListener('change', () => { resolve(input.files[0] || null); input.remove(); });
      input.addEventListener('cancel', () => { resolve(null); input.remove(); });
      document.body.append(input);
      input.click();
    });
  }

  const readDataUrl = (file) => new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result);
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });

  // ─── window.control ────────────────────────────────────────────────────────
  const web = {
    user: boot.user,
    caps: boot.caps,
    server: boot.server,
    clientId,
    rpc: (channel, ...args) => call('rpc', channel, args),
    async logout() {
      leaving = true;
      await fetch('/api/auth/logout', { method: 'POST', headers: { 'x-control': '1' }, credentials: 'same-origin' }).catch(() => {});
      location.replace('/login');
    },
  };

  const idle = { phase: 'idle', supported: false };

  window.control = {
    platform: 'web',
    web,
    pathForFile: () => '',

    settings: {
      initial: boot.settings || {},
      set: send('settings:set'),
      reset: send('settings:reset'),
      onChange: listen('settings:changed'),
      onReset: listen('settings:reset'),
    },

    win: {
      close() {},
      minimize() {},
      toggleMaximize() {},
      setFullscreen,
      setZoom: (factor) => { document.documentElement.style.zoom = String(Math.min(1.5, Math.max(0.6, Number(factor) || 1))); },
      state: async () => fsState(),
      onState: (cb) => { stateListeners.add(cb); return () => stateListeners.delete(cb); },
    },

    library: {
      tracks: invoke('lib:tracks'),
      playlists: invoke('lib:playlists'),
      scan: invoke('lib:scan'),
      addPaths: async () => [],
      addFolder: async () => {
        const dir = await window.control.web.pickFolder?.();
        return dir ? invoke('srv:add-folder')(dir) : boot.settings.libraryFolders || [];
      },
      addMusicFolder: async () => boot.settings.libraryFolders || [],
      removeFolder: invoke('lib:remove-folder'),
      removeTracks: invoke('lib:remove-tracks'),
      reveal() {},
      playStart: invoke('lib:play-start'),
      playEnd: send('lib:play-end'),
      onChange: listen('lib:changed'),
      onScan: listen('lib:scan-progress'),
    },

    lyrics: {
      get: invoke('lyrics:get'),
      set: invoke('lyrics:set'),
      saveFile: invoke('lyrics:save-file'),
    },

    update: {
      state: async () => idle,
      check: async () => idle,
      download: async () => idle,
      install: async () => idle,
      onState: () => () => {},
    },

    discord: {
      setActivity() {},
      status: async () => ({ state: 'unsupported' }),
      test: async () => ({ ok: false }),
      onStatus: () => () => {},
    },

    stats: {
      get: invoke('stats:get'),
      async saveImage(dataUrl, name) {
        const match = /^data:image\/png;base64,([A-Za-z0-9+/=]+)$/.exec(String(dataUrl || ''));
        if (!match) return false;
        const bytes = Uint8Array.from(atob(match[1]), (c) => c.charCodeAt(0));
        const file = `${String(name || 'control').replace(/[\\/:*?"<>|]/g, '_')}.png`;
        download(file, new Blob([bytes], { type: 'image/png' }));
        return file;
      },
    },

    tags: {
      read: invoke('tags:read'),
      write: invoke('tags:write'),
      undo: invoke('tags:undo'),
      stageCover: invoke('tags:stage-cover'),
      async pickCover() {
        const file = await pickFile('image/jpeg,image/png,image/webp');
        if (!file) return { ok: true, data: null };
        if (file.size > 15 * 1024 * 1024) return { ok: false, message: 'Image trop volumineuse.' };
        return call('rpc', 'tags:stage-cover', [{ data: await readDataUrl(file) }]);
      },
      searchCovers: invoke('tags:search-covers'),
    },

    playlists: {
      create: invoke('pl:create'),
      rename: invoke('pl:rename'),
      rules: invoke('pl:rules'),
      delete: invoke('pl:delete'),
      add: invoke('pl:add'),
      remove: invoke('pl:remove'),
      reorder: invoke('pl:reorder'),
      import: async () => null,
      export: async () => null,
    },

    tools: {
      status: invoke('tools:status'),
      install: invoke('tools:install'),
      update: invoke('tools:update'),
      onStatus: listen('tools:status'),
      onProgress: listen('tools:progress'),
      onUpdated: listen('tools:updated'),
    },

    downloads: {
      list: invoke('dl:list'),
      info: invoke('dl:info'),
      search: invoke('dl:search'),
      add: invoke('dl:add'),
      cancel: send('dl:cancel'),
      retry: send('dl:retry'),
      remove: send('dl:remove'),
      clear: send('dl:clear'),
      chooseDir: async () => null,
      chooseCookies: async () => null,
      openDir() {},
      onUpdate: listen('dl:update'),
      onReset: listen('dl:reset'),
    },

    env: { ...boot.env, web: true, caps: boot.caps },

    media: {
      home: invoke('media:home'),
      list: invoke('media:list'),
      details: invoke('media:details'),
      season: invoke('media:season'),
      search: invoke('media:search'),
      testKey: invoke('media:test-key'),
      clearCache: invoke('media:clear-cache'),
    },

    watch: {
      save: invoke('watch:save'),
      show: invoke('watch:show'),
      continue: invoke('watch:continue'),
      history: invoke('watch:history'),
      remove: invoke('watch:remove'),
      clear: invoke('watch:clear'),
      list: invoke('watchlist:list'),
      toggle: invoke('watchlist:toggle'),
      onChange: listen('watch:changed'),
    },

    app: {
      info: async () => boot.info,
      openDataDir() {},
      openExternal: (url) => { if (/^https:\/\//.test(String(url))) window.open(url, '_blank', 'noopener,noreferrer'); },
      quit() {},
    },
  };
})();

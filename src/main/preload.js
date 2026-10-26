'use strict';

const { contextBridge, ipcRenderer, webUtils, webFrame } = require('electron');

function listen(channel) {
  return (callback) => {
    const handler = (_event, payload) => callback(payload);
    ipcRenderer.on(channel, handler);
    return () => ipcRenderer.removeListener(channel, handler);
  };
}

const invoke = (channel) => (...args) => ipcRenderer.invoke(channel, ...args);
const send = (channel) => (...args) => ipcRenderer.send(channel, ...args);

contextBridge.exposeInMainWorld('control', {
  platform: process.platform,
  pathForFile: (file) => webUtils.getPathForFile(file),

  settings: {
    initial: ipcRenderer.sendSync('settings:get-all'),
    set: (patch) => ipcRenderer.send('settings:set', patch),
    reset: () => ipcRenderer.send('settings:reset'),
    onChange: listen('settings:changed'),
    onReset: listen('settings:reset'),
  },

  win: {
    close: send('win:close'),
    minimize: send('win:minimize'),
    toggleMaximize: send('win:toggle-maximize'),
    setFullscreen: send('win:set-fullscreen'),
    setZoom: (factor) => webFrame.setZoomFactor(Math.min(1.5, Math.max(0.6, Number(factor) || 1))),
    state: invoke('win:state'),
    onState: listen('window:state'),
  },

  library: {
    tracks: invoke('lib:tracks'),
    playlists: invoke('lib:playlists'),
    scan: invoke('lib:scan'),
    addPaths: invoke('lib:add-paths'),
    addFolder: invoke('lib:add-folder'),
    addMusicFolder: invoke('lib:add-music-folder'),
    removeFolder: invoke('lib:remove-folder'),
    removeTracks: invoke('lib:remove-tracks'),
    reveal: send('lib:reveal'),
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
    state: invoke('update:state'),
    check: invoke('update:check'),
    download: invoke('update:download'),
    install: invoke('update:install'),
    onState: listen('update:state'),
  },

  discord: {
    setActivity: send('discord:activity'),
    status: invoke('discord:status'),
    test: invoke('discord:test'),
    onStatus: listen('discord:status'),
  },

  stats: {
    get: invoke('stats:get'),
    saveImage: invoke('stats:save-image'),
  },

  tags: {
    read: invoke('tags:read'),
    write: invoke('tags:write'),
    undo: invoke('tags:undo'),
    stageCover: invoke('tags:stage-cover'),
    pickCover: invoke('tags:pick-cover'),
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
    import: invoke('pl:import'),
    export: invoke('pl:export'),
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
    chooseDir: invoke('dl:choose-dir'),
    chooseCookies: invoke('dl:choose-cookies'),
    openDir: send('dl:open-dir'),
    onUpdate: listen('dl:update'),
    onReset: listen('dl:reset'),
  },

  env: ipcRenderer.sendSync('app:env'),

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

  // Serveurs control affichés dans la fenêtre (absent de la version web).
  sources: {
    state: invoke('sources:state'),
    add: invoke('sources:add'),
    rename: invoke('sources:rename'),
    remove: invoke('sources:remove'),
    switch: invoke('sources:switch'),
    reload: invoke('sources:reload'),
    peek: invoke('sources:peek'),
    unpeek: invoke('sources:unpeek'),
    onChange: listen('sources:changed'),
    onPauseRequest: listen('sources:pause'),
  },

  app: {
    info: invoke('app:info'),
    openDataDir: send('app:open-data-dir'),
    openExternal: send('app:open-external'),
    quit: send('app:quit'),
  },
});

'use strict';

const { contextBridge, ipcRenderer, webUtils } = require('electron');

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
    add: invoke('dl:add'),
    cancel: send('dl:cancel'),
    retry: send('dl:retry'),
    remove: send('dl:remove'),
    clear: send('dl:clear'),
    chooseDir: invoke('dl:choose-dir'),
    openDir: send('dl:open-dir'),
    onUpdate: listen('dl:update'),
    onReset: listen('dl:reset'),
  },

  app: {
    info: invoke('app:info'),
    openDataDir: send('app:open-data-dir'),
    openExternal: send('app:open-external'),
    quit: send('app:quit'),
  },
});

'use strict';

// Ce que les services (bibliothèque, tags, paroles, outils, films) demandent à l'environnement : réseau, images,
// corbeille. Dans l'application de bureau c'est Electron ; sur le serveur web, Node seul.

const fs = require('node:fs');
const fsp = fs.promises;
const os = require('node:os');
const path = require('node:path');

const electron = process.versions.electron ? require('electron') : null;
const settings = { trashDir: path.join(os.tmpdir(), 'control-trash') };

/** Le serveur choisit où vont les fichiers « supprimés » (pas de corbeille système sur un serveur). */
function configure(options) {
  Object.assign(settings, options);
}

const net = {
  fetch: (...args) => (electron ? electron.net.fetch(...args) : fetch(...args)),
};

const nativeImage = electron ? electron.nativeImage : require('./image-lite');

const shell = {
  async trashItem(file) {
    if (electron) return electron.shell.trashItem(file);
    await fsp.mkdir(settings.trashDir, { recursive: true });
    const target = path.join(settings.trashDir, `${Date.now()}-${path.basename(file)}`);
    try {
      await fsp.rename(file, target);
    } catch (err) {
      if (err.code !== 'EXDEV') throw err;
      await fsp.copyFile(file, target);
      await fsp.unlink(file);
    }
  },
};

module.exports = { net, nativeImage, shell, configure, isElectron: !!electron };

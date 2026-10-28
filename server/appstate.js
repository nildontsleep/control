'use strict';

// Versions du serveur installées par la mise à jour automatique : où elles sont rangées et laquelle lancer.
// Partagé par le lanceur (qui choisit la version et revient en arrière) et par le serveur (qui en télécharge de
// nouvelles). Le format de state.json doit rester compatible d'une version à l'autre : champs inconnus conservés.

const fs = require('node:fs');
const path = require('node:path');

/** Compare deux versions « 1.2.3 » (suffixe de préversion ignoré). Positif si a > b. */
function compareVersions(a, b) {
  const parts = (v) => String(v).replace(/^v/i, '').split('-')[0].split('.').map((n) => Number(n) || 0);
  const [x, y] = [parts(a), parts(b)];
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const d = (x[i] || 0) - (y[i] || 0);
    if (d) return d;
  }
  return 0;
}

const VERSION = /^\d+\.\d+\.\d+$/;

function paths(dataDir) {
  const root = path.join(dataDir, 'app');
  return { root, versions: path.join(root, 'versions'), tmp: path.join(root, 'tmp'), state: path.join(root, 'state.json') };
}

function readState(dataDir) {
  try {
    const state = JSON.parse(fs.readFileSync(paths(dataDir).state, 'utf8'));
    if (!state || typeof state !== 'object' || Array.isArray(state)) return { bad: [] };
    return { ...state, bad: Array.isArray(state.bad) ? state.bad.filter((v) => VERSION.test(v)) : [] };
  } catch {
    return { bad: [] };
  }
}

/** Écriture atomique (un arrêt en plein milieu ne laisse jamais un fichier à moitié écrit). */
function writeState(dataDir, patch) {
  const file = paths(dataDir).state;
  const next = { ...readState(dataDir), ...patch };
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(next, null, 2));
  fs.renameSync(tmp, file);
  return next;
}

/** Dossier d'une version installée si elle est complète (extraite et vérifiée), sinon null. */
function versionDir(dataDir, version) {
  if (!VERSION.test(String(version))) return null;
  const dir = path.join(paths(dataDir).versions, version);
  return fs.existsSync(path.join(dir, '.ok')) && fs.existsSync(path.join(dir, 'server', 'index.js')) ? dir : null;
}

module.exports = { compareVersions, VERSION, paths, readState, writeState, versionDir };

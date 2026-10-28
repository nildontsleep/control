#!/usr/bin/env node
'use strict';

// Lanceur du serveur web (`npm run web`, et l'image Docker) : il fait tourner le serveur comme un processus fils.
// C'est ce qui permet la mise à jour automatique : quand le serveur a téléchargé une nouvelle version, il demande au
// lanceur de le redémarrer, et le lanceur relance le serveur sur cette version. Si elle ne démarre pas, le lanceur
// revient tout seul à la précédente et se souvient de ne plus la réessayer.
//
// Ce fichier ne change pour ainsi dire jamais : il fait partie de la version installée à l'origine, et c'est lui qui
// choisit la version à lancer. Le serveur peut aussi se lancer sans lui (`node server/index.js`), mais sans mise à
// jour automatique.

const [major, minor] = process.versions.node.split('.').map(Number);
if (major < 22 || (major === 22 && minor < 13)) {
  console.error(`control (web) demande Node.js 22.13 ou plus (version installée : ${process.versions.node}).`);
  process.exit(1);
}

const fs = require('node:fs');
const path = require('node:path');
const { fork } = require('node:child_process');
const { loadConfig } = require('./config');
const appstate = require('./appstate');

const ROOT = path.resolve(__dirname, '..');
const BUNDLED = require('../package.json').version;
const { dataDir } = loadConfig();

const READY_TIMEOUT_MS = 90 * 1000; // une version qui n'écoute pas au bout de ce délai est abandonnée
const STOP_TIMEOUT_MS = 10 * 1000;
const CRASH_WINDOW_MS = 5 * 60 * 1000;

let child = null;
let stopping = false;
let crashes = [];

/** La version à lancer : celle que la mise à jour a installée, si elle est complète, plus récente et pas déjà en échec. */
function choose() {
  const st = appstate.readState(dataDir);
  const target = st.target;
  if (target && !st.bad.includes(target) && appstate.compareVersions(target, BUNDLED) > 0) {
    const dir = appstate.versionDir(dataDir, target);
    if (dir) return { version: target, dir, staged: true };
  }
  return { version: BUNDLED, dir: ROOT, staged: false };
}

/** Une version qui ne démarre pas : on ne la réessaie plus et on revient à la précédente (ou à celle d'origine). */
function abandon(version, reason) {
  const st = appstate.readState(dataDir);
  const previous = st.previous && st.previous !== version && !st.bad.includes(st.previous) && appstate.versionDir(dataDir, st.previous) ? st.previous : null;
  appstate.writeState(dataDir, { bad: [...new Set([...st.bad, version])], target: previous, previous: null, failure: { version, reason, at: Date.now() } });
  console.error(`control : la version ${version} ne démarre pas (${reason}) ; retour à la version ${previous || BUNDLED}.`);
}

/** Arrête le serveur proprement (message), puis par un signal s'il ne répond pas. */
function terminate(proc) {
  try { proc.send({ type: 'stop' }); } catch { /* canal déjà fermé */ }
  setTimeout(() => proc.exitCode === null && proc.kill('SIGTERM'), 3000).unref();
  setTimeout(() => proc.exitCode === null && proc.kill('SIGKILL'), STOP_TIMEOUT_MS).unref();
}

function start() {
  const pick = choose();
  const env = { ...process.env, CONTROL_SUPERVISED: '1', CONTROL_BASE_ROOT: ROOT };
  delete env.CONTROL_STAGED;
  if (pick.staged) {
    env.CONTROL_STAGED = pick.version;
    // Une version téléchargée sans dépendances propres utilise celles de l'installation d'origine.
    if (!fs.existsSync(path.join(pick.dir, 'node_modules'))) env.NODE_PATH = [path.join(ROOT, 'node_modules'), process.env.NODE_PATH].filter(Boolean).join(path.delimiter);
    console.log(`control : lancement de la version ${pick.version} (mise à jour installée).`);
  }

  let ready = false;
  let requested = false;
  const proc = fork(path.join(pick.dir, 'server', 'index.js'), process.argv.slice(2), { env, stdio: ['inherit', 'inherit', 'inherit', 'ipc'] });
  child = proc;
  const timer = setTimeout(() => {
    if (ready) return;
    console.error(`control : la version ${pick.version} n'a pas démarré en ${READY_TIMEOUT_MS / 1000} s.`);
    proc.kill('SIGKILL');
  }, READY_TIMEOUT_MS);

  proc.on('message', (message) => {
    if (message?.type === 'ready') {
      ready = true;
      clearTimeout(timer);
    } else if (message?.type === 'restart' && !stopping) {
      requested = true;
      terminate(proc);
    }
  });

  proc.on('error', (err) => console.error('control : lancement impossible :', err.message));
  proc.on('exit', (code, signal) => {
    clearTimeout(timer);
    child = null;
    if (stopping) process.exit(0);
    if (requested) {
      start();
      return;
    }
    if (ready && code === 0) process.exit(0); // arrêt voulu (Ctrl+C, commande d'arrêt)
    const reason = signal ? `signal ${signal}` : `code ${code}`;
    if (pick.staged && !ready) {
      abandon(pick.version, `a quitté avant d'être prêt, ${reason}`);
      start();
      return;
    }
    const now = Date.now();
    crashes = crashes.filter((t) => now - t < CRASH_WINDOW_MS).concat(now);
    if (pick.staged && crashes.length >= 3) {
      abandon(pick.version, `plantages répétés, ${reason}`);
      crashes = [];
      start();
      return;
    }
    if (crashes.length >= 10) {
      console.error('control : le serveur plante en boucle, abandon.');
      process.exit(1);
    }
    const wait = Math.min(30 * 1000, 1000 * 2 ** (crashes.length - 1));
    console.error(`control : le serveur s'est arrêté (${reason}) ; nouvel essai dans ${wait / 1000} s.`);
    setTimeout(start, wait);
  });
}

function shutdown() {
  if (stopping) return;
  stopping = true;
  if (child) terminate(child);
  else process.exit(0);
}
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);

start();

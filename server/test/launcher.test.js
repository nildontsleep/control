'use strict';

// Le lanceur de bout en bout : un vrai processus qui lance le serveur, le met à jour sur une version téléchargée,
// et revient à la version précédente quand la nouvelle ne démarre pas.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawn } = require('node:child_process');
const pkg = require('../../package.json');
const appstate = require('../appstate');
const { client, until, fakeBundle, mockGithub, freePort } = require('./helpers');

const health = async (base) => {
  try {
    const res = await fetch(`${base}/api/health`);
    return res.ok ? (await res.json()).version : null;
  } catch {
    return null;
  }
};

test('lanceur : mise à jour du serveur sur une nouvelle version, retour arrière si elle ne démarre pas', { timeout: 120000 }, async () => {
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'control-launch-'));
  const github = await mockGithub();
  const port = await freePort();
  const base = `http://127.0.0.1:${port}`;
  let output = '';
  const launcher = spawn(process.execPath, [path.join(__dirname, '..', 'launcher.js')], {
    env: { ...process.env, CONTROL_DATA: dataDir, PORT: String(port), CONTROL_HOST: '127.0.0.1', CONTROL_RESCAN_MINUTES: '1000', CONTROL_UPDATE_URL: github.url, CONTROL_UPDATE: '', CONTROL_LIBRARY: '' },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  launcher.stdout.on('data', (d) => { output += d; });
  launcher.stderr.on('data', (d) => { output += d; });
  const exited = new Promise((resolve) => launcher.on('exit', (code) => resolve(code)));

  try {
    assert.equal(await until(() => health(base), 30000), pkg.version, 'le serveur d’origine démarre');
    const admin = client(base);
    assert.equal((await admin.json('POST', '/api/setup/init', { username: 'admin', password: 'long enough password' })).status, 200);

    // ─── Mise à jour vers 99.0.0 ────────────────────────────────────────────
    github.publish(fakeBundle('99.0.0'));
    const found = await admin.rpc('srv:update:check');
    assert.deepEqual([found.phase, found.version, found.supervised], ['available', '99.0.0', true]);
    assert.equal((await admin.rpc('srv:update:download')).phase, 'ready');
    assert.equal(await health(base), pkg.version, 'rien ne change avant l’installation');
    assert.equal((await admin.rpc('srv:update:install')).phase, 'installing');
    assert.equal(await until(async () => { const v = await health(base); return v === '99.0.0' ? v : null; }, 40000), '99.0.0', 'le serveur tourne sur la nouvelle version');
    assert.equal(launcher.exitCode, null, 'le lanceur, lui, n’a pas bougé');
    assert.equal(appstate.readState(dataDir).target, '99.0.0');
    // La session et les données survivent au redémarrage.
    assert.equal((await admin.rpc('srv:update:status')).current, '99.0.0');
    assert.ok(fs.readdirSync(path.join(dataDir, 'backups')).some((n) => n.startsWith(`server-${pkg.version}-`)), 'base du serveur sauvegardée avant le changement');

    // ─── Une version cassée : retour à 99.0.0, et on ne la réessaie plus ──────
    github.publish(fakeBundle('99.1.0', { broken: true }));
    assert.equal((await admin.rpc('srv:update:check')).version, '99.1.0');
    assert.equal((await admin.rpc('srv:update:download')).phase, 'ready');
    assert.equal((await admin.rpc('srv:update:install')).phase, 'installing');
    await until(() => appstate.readState(dataDir).bad.includes('99.1.0'), 40000);
    assert.equal(await until(async () => { const v = await health(base); return v === '99.0.0' ? v : null; }, 40000), '99.0.0', 'retour à la version précédente');
    const st = appstate.readState(dataDir);
    assert.deepEqual([st.target, st.failure.version], ['99.0.0', '99.1.0']);
    const status = await admin.rpc('srv:update:status');
    assert.equal(status.failure.version, '99.1.0', 'l’administrateur voit pourquoi');
    assert.equal((await admin.rpc('srv:update:check')).phase, 'available', 'la version peut être redemandée à la main');

    // ─── Arrêt propre ───────────────────────────────────────────────────────
    launcher.kill('SIGTERM');
    assert.equal(await Promise.race([exited, new Promise((r) => setTimeout(() => r('délai'), 15000).unref())]), 0);
  } catch (err) {
    err.message += `\n--- sortie du lanceur ---\n${output.slice(-3000)}`;
    throw err;
  } finally {
    if (launcher.exitCode === null) launcher.kill('SIGKILL');
    await github.close();
    fs.rmSync(dataDir, { recursive: true, force: true });
  }
});

'use strict';

// Mise à jour automatique du serveur : archives (lecture stricte), vérification, téléchargement, installation.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const zlib = require('node:zlib');
const { pack, unpack } = require('../tar');
const appstate = require('../appstate');
const { WebUpdater, nodeSatisfies } = require('../updater');
const { openServerDb } = require('../serverdb');
const { fakeBundle, mockGithub } = require('./helpers');

/** Modifie l'en-tête de la première entrée d'une archive (puis recalcule la somme de contrôle). */
function tamper(gz, mutate) {
  const raw = zlib.gunzipSync(gz);
  const head = raw.subarray(0, 512);
  mutate(head);
  head.fill(32, 148, 156);
  let sum = 0;
  for (const b of head) sum += b;
  head.write(`${sum.toString(8).padStart(6, '0')}\0 `, 148, 8, 'latin1');
  return zlib.gzipSync(raw);
}

test('archive : aller-retour, droits et chemins longs conservés', () => {
  const long = `server/${'a'.repeat(60)}/${'b'.repeat(80)}.js`;
  const files = unpack(pack([{ name: 'package.json', data: '{}' }, { name: 'server/run.sh', data: 'x'.repeat(1500), mode: 0o755 }, { name: long, data: 'y' }]));
  assert.deepEqual(files.map((f) => f.name), ['package.json', 'server/run.sh', long]);
  assert.equal(files[1].data.length, 1500);
  assert.equal(files[1].mode, 0o755);
});

test('archive : liens, chemins qui sortent du dossier, doublons et archives abîmées refusés', () => {
  const good = pack([{ name: 'a.txt', data: 'x' }]);
  assert.throws(() => unpack(tamper(good, (h) => { h[156] = 50; })), /non prise en charge/); // lien symbolique
  assert.throws(() => unpack(tamper(good, (h) => { h.write('../evil\0', 0, 'latin1'); })), /chemin refusé/);
  assert.throws(() => unpack(tamper(good, (h) => { h.write('/etc/passwd\0', 0, 'latin1'); })), /chemin refusé/);
  assert.throws(() => unpack(tamper(good, (h) => { h.write('a\\b\0', 0, 'latin1'); })), /chemin refusé/);
  assert.throws(() => unpack(pack([{ name: 'a', data: '1' }, { name: 'a/b', data: '2' }])), /incohérent/);
  const dup = zlib.gzipSync(Buffer.concat([zlib.gunzipSync(pack([{ name: 'a', data: '1' }])).subarray(0, 1024), zlib.gunzipSync(pack([{ name: 'a', data: '2' }]))]));
  assert.throws(() => unpack(dup), /double/);
  const raw = zlib.gunzipSync(good);
  raw[3] ^= 0xff; // octet du nom modifié sans recalculer la somme
  assert.throws(() => unpack(zlib.gzipSync(raw)), /corrompu/);
  assert.throws(() => unpack(good, { maxFileBytes: 0 }), /volumineuse/);
  assert.throws(() => unpack(Buffer.from('pas une archive')));
  assert.throws(() => pack([{ name: '../x', data: '' }]), /chemin refusé/);
});

test('versions : comparaison et exigence de Node', () => {
  assert.ok(appstate.compareVersions('0.10.0', '0.9.9') > 0);
  assert.ok(appstate.compareVersions('v1.0.0', '1.0.0') === 0);
  assert.ok(appstate.compareVersions('0.9.1', '0.9.2') < 0);
  assert.equal(nodeSatisfies('>=22.13', '22.13.0'), true);
  assert.equal(nodeSatisfies('>=22.13', '22.12.9'), false);
  assert.equal(nodeSatisfies('>=24', '22.22.0'), false);
  assert.equal(nodeSatisfies(undefined), true);
});

async function setup(options = {}) {
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'control-upd-'));
  const github = await mockGithub();
  process.env.CONTROL_UPDATE_URL = github.url;
  const calls = { restart: 0, backup: 0 };
  const config = { mode: 'notify', hour: 4, token: '', locked: false, ...options.config };
  const updater = new WebUpdater({
    version: '0.9.0',
    dataDir,
    settings: () => config,
    supervised: options.supervised ?? true,
    restart: () => { calls.restart++; },
    backup: () => { calls.backup++; },
    idle: () => !!options.idle,
  });
  const states = [];
  updater.on('state', (s) => states.push(s.phase));
  return {
    dataDir, github, updater, calls, config, states,
    close: async () => {
      updater.stop();
      delete process.env.CONTROL_UPDATE_URL;
      await github.close();
      fs.rmSync(dataDir, { recursive: true, force: true });
    },
  };
}

test('mise à jour : vérification, téléchargement vérifié, installation, redémarrage demandé', async () => {
  const t = await setup();
  try {
    t.github.publish(fakeBundle('0.9.0'));
    assert.equal((await t.updater.check()).phase, 'none', 'même version : rien à faire');
    t.github.publish(fakeBundle('0.9.1'));
    const found = await t.updater.check({ manual: true });
    assert.equal(found.phase, 'available');
    assert.equal(found.version, '0.9.1');
    assert.equal(found.manual, undefined);
    assert.match(found.notes, /Nouveautés/);

    const ready = await t.updater.download();
    assert.equal(ready.phase, 'ready');
    assert.equal(ready.verified, true);
    const dir = appstate.versionDir(t.dataDir, '0.9.1');
    assert.ok(dir, 'version installée à côté de celle en cours');
    assert.equal(JSON.parse(fs.readFileSync(path.join(dir, 'package.json'), 'utf8')).version, '0.9.1');
    assert.ok(fs.existsSync(path.join(dir, 'server', 'index.js')));
    assert.ok(!fs.existsSync(path.join(dir, 'server', 'test')), 'les tests ne sont pas dans l’archive');
    assert.equal(appstate.readState(t.dataDir).target, undefined, 'rien n’est activé avant l’installation');

    const installing = await t.updater.install();
    assert.equal(installing.phase, 'installing');
    assert.equal(t.calls.backup, 1);
    assert.deepEqual([appstate.readState(t.dataDir).target, appstate.readState(t.dataDir).previous], ['0.9.1', null]);
    await new Promise((r) => setTimeout(r, 600));
    assert.equal(t.calls.restart, 1, 'le lanceur est prié de redémarrer le serveur');
  } finally {
    await t.close();
  }
});

test('mise à jour : empreinte différente, archive hors format, version annoncée fausse, Node trop ancien', async () => {
  const t = await setup();
  try {
    t.github.publish(fakeBundle('0.9.1'), { digestOverride: '0'.repeat(64) });
    await t.updater.check({ manual: true });
    let st = await t.updater.download();
    assert.deepEqual([st.phase, st.code], ['error', 'corrupt']);
    assert.equal(appstate.versionDir(t.dataDir, '0.9.1'), null);

    // L'archive est annoncée 0.9.2 mais contient 0.9.3.
    const lying = fakeBundle('0.9.3');
    t.github.publish({ ...lying, version: '0.9.2' });
    await t.updater.check({ manual: true });
    st = await t.updater.download();
    assert.deepEqual([st.phase, st.code], ['error', 'stage']);

    // Chemin qui sortirait du dossier d'extraction.
    const evil = tamper(pack([{ name: 'package.json', data: '{}' }]), (h) => { h.write('../../evil\0', 0, 'latin1'); });
    const evilBundle = { version: '0.9.4', data: evil, sha256: require('node:crypto').createHash('sha256').update(evil).digest('hex') };
    t.github.publish(evilBundle);
    await t.updater.check({ manual: true });
    st = await t.updater.download();
    assert.deepEqual([st.phase, st.code], ['error', 'stage']);
    assert.ok(!fs.existsSync(path.join(t.dataDir, '..', 'evil')));

    const bare = pack([{ name: 'server/index.js', data: 'x' }]); // pas de package.json
    t.github.publish({ version: '0.9.6', data: bare, sha256: require('node:crypto').createHash('sha256').update(bare).digest('hex') });
    await t.updater.check({ manual: true });
    st = await t.updater.download();
    assert.deepEqual([st.phase, st.code], ['error', 'stage']);

    t.github.publish(fakeBundle('0.9.5', { engines: '>=99' }));
    await t.updater.check({ manual: true });
    st = await t.updater.download();
    assert.deepEqual([st.phase, st.code], ['error', 'engine']);
    const versions = path.join(t.dataDir, 'app', 'versions');
    assert.deepEqual(fs.existsSync(versions) ? fs.readdirSync(versions) : [], [], 'aucune version à moitié installée');
  } finally {
    await t.close();
  }
});

test('mise à jour : sans lanceur, sans archive ou sans empreinte, on prévient sans installer', async () => {
  const bare = await setup({ supervised: false });
  try {
    bare.github.publish(fakeBundle('0.9.1'));
    const st = await bare.updater.check({ manual: true });
    assert.deepEqual([st.phase, st.manual, st.reason], ['available', true, 'unsupervised']);
    assert.equal((await bare.updater.download()).phase, 'available', 'rien n’est téléchargé');
  } finally {
    await bare.close();
  }
  const t = await setup();
  try {
    t.github.publish(fakeBundle('0.9.1'), { noAsset: true });
    assert.equal((await t.updater.check({ manual: true })).reason, 'asset');
    t.github.publish(fakeBundle('0.9.1'), { noDigest: true });
    assert.equal((await t.updater.check({ manual: true })).reason, 'nodigest');
  } finally {
    await t.close();
  }
});

test('mise à jour : modes off / notify / auto, versions déjà en échec ignorées', async () => {
  const t = await setup({ config: { mode: 'off' } });
  try {
    t.github.publish(fakeBundle('0.9.1'));
    await t.updater.check(); // périodique, mode off
    assert.deepEqual(t.github.state.hits, [], 'rien n’est demandé à GitHub');
    assert.equal((await t.updater.check({ manual: true })).phase, 'available', 'mais l’administrateur peut toujours le demander');

    t.config.mode = 'notify';
    await t.updater.run();
    assert.equal(t.updater.status.phase, 'available', 'on prévient, sans rien télécharger');

    t.config.mode = 'auto';
    await t.updater.run();
    await t.updater.check(); // une vérification périodique n'écarte pas une version déjà prête
    assert.equal(t.updater.status.phase, 'ready', 'téléchargée, mais personne ne l’a installée : des comptes sont connectés');
    t.config.hour = new Date().getHours();
    await t.updater.auto();
    assert.equal(t.updater.status.phase, 'installing', 'installée à l’heure choisie');

    // Une version abandonnée par le lanceur n'est plus proposée automatiquement.
    t.updater.stop();
    appstate.writeState(t.dataDir, { bad: ['0.9.2'] });
    t.github.publish(fakeBundle('0.9.2'));
    const fresh = new WebUpdater({ version: '0.9.0', dataDir: t.dataDir, settings: () => t.config, supervised: true, restart() {} });
    const st = await fresh.check();
    assert.deepEqual([st.phase, st.skipped], ['none', '0.9.2']);
    assert.equal((await fresh.check({ manual: true })).phase, 'available', 'sauf si l’administrateur la redemande');
  } finally {
    await t.close();
  }
});

test('mise à jour automatique : installée dès que plus personne n’est connecté', async () => {
  const t = await setup({ config: { mode: 'auto', hour: (new Date().getHours() + 12) % 24 }, idle: true });
  try {
    t.github.publish(fakeBundle('0.9.1'));
    await t.updater.run();
    assert.equal(t.updater.status.phase, 'installing');
    assert.equal(appstate.readState(t.dataDir).target, '0.9.1');
  } finally {
    await t.close();
  }
});

test('mise à jour : ménage au démarrage et copie de sauvegarde de la base', async () => {
  const t = await setup();
  try {
    const versions = path.join(t.dataDir, 'app', 'versions');
    for (const v of ['0.9.1', '0.9.2', '0.9.3']) { fs.mkdirSync(path.join(versions, v, 'server'), { recursive: true }); fs.writeFileSync(path.join(versions, v, 'server', 'index.js'), ''); fs.writeFileSync(path.join(versions, v, '.ok'), ''); }
    fs.mkdirSync(`${versions}/0.9.4.tmp-123`, { recursive: true });
    appstate.writeState(t.dataDir, { target: '0.9.3', previous: '0.9.2' });
    t.updater.start();
    await new Promise((r) => setTimeout(r, 300));
    assert.deepEqual(fs.readdirSync(versions).sort(), ['0.9.2', '0.9.3'], 'la version en cours de déploiement et la précédente sont gardées');

    const db = openServerDb(path.join(t.dataDir, 'server.db'));
    db.prepare("INSERT INTO kv (key, value) VALUES ('x', 'y')").run();
    const file = await WebUpdater.backupDb(db, t.dataDir, '0.9.0');
    db.close();
    assert.ok(fs.statSync(file).size > 0);
  } finally {
    await t.close();
  }
});

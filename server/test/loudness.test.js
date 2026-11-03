'use strict';

// Normalisation du volume : lecture du résumé de ffmpeg, mesure en arrière-plan, gain appliqué par le lecteur.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { parseSummary } = require('../../src/main/loudness');
const { boot, client, until } = require('./helpers');

const FFMPEG = ['/usr/bin/ffmpeg', '/usr/local/bin/ffmpeg'].find((f) => fs.existsSync(f));
const SUMMARY = `
[Parsed_ebur128_0 @ 0x55] Summary:

  Integrated loudness:
    I:         -14.3 LUFS
    Threshold: -24.4 LUFS

  Loudness range:
    LRA:         5.2 LU

  True peak:
    Peak:       -0.4 dBFS
`;

test('résumé de ffmpeg : niveau et crête lus, silence reconnu', () => {
  assert.deepEqual(parseSummary(SUMMARY), { loudness: -14.3, truePeak: -0.4 });
  assert.equal(parseSummary('I:         -inf LUFS'), null);
  assert.equal(parseSummary('rien d’utile'), null);
});

test('gain du lecteur : vers le niveau voulu, borné, sans écrêter, sans effet quand l’option est coupée', async () => {
  const { gainDb, toLinear, MAX_GAIN, MIN_GAIN } = await import('../../src/renderer/js/loudness.js');
  const on = { enabled: true, target: -16 };
  assert.equal(gainDb({ loudness: -8, truePeak: -0.2 }, on), -8, 'un morceau fort est atténué');
  assert.equal(gainDb({ loudness: -23, truePeak: -10 }, on), 7, 'un morceau faible est relevé');
  assert.equal(gainDb({ loudness: -23, truePeak: -3 }, on), 2, 'la relève s’arrête avant la crête (−1 dBTP)');
  assert.equal(gainDb({ loudness: -23, truePeak: null }, on), 0, 'crête inconnue : pas de relève');
  assert.equal(gainDb({ loudness: -40, truePeak: -30 }, on), MAX_GAIN, 'relève plafonnée');
  assert.equal(gainDb({ loudness: 5 }, on), MIN_GAIN, 'atténuation plafonnée');
  assert.equal(gainDb({ loudness: -23, truePeak: -10 }, { ...on, boost: false }), 0, 'sans graphe Web Audio : atténuer seulement');
  assert.equal(gainDb({ loudness: -8 }, { ...on, boost: false }), -8);
  assert.equal(gainDb({ loudness: -8 }, { enabled: false }), 0);
  assert.equal(gainDb({}, on), 0, 'pas encore mesuré : inchangé');
  assert.ok(Math.abs(toLinear(-6.0206) - 0.5) < 1e-4);
});

test('mesure en arrière-plan : les niveaux entrent dans la liste des morceaux, plus fort = plus haut', { skip: !FFMPEG }, async (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'control-loud-'));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const sine = (name, volume) => execFileSync(FFMPEG, ['-v', 'quiet', '-y', '-f', 'lavfi', '-i', 'sine=frequency=440:duration=4', '-af', `volume=${volume}`, path.join(dir, name)]);
  sine('fort.wav', 0.8);
  sine('doux.wav', 0.05);
  const s = await boot({ noCommon: true });
  t.after(s.close);
  const admin = client(s.base);
  await admin.json('POST', '/api/setup/init', { username: 'nil', password: 'long enough password', libraryFolders: [dir] });
  const tracks = await until(async () => { const l = await admin.rpc('lib:tracks'); return l.length === 2 ? l : null; });
  assert.ok(tracks.every((x) => x.loudness === null && 'truePeak' in x), 'rien n’est mesuré tant que l’option est coupée');

  const user = s.hub.accounts.list()[0];
  const loudness = s.hub.entry(user).core.loudness;
  loudness.tools = { path: () => FFMPEG };
  assert.equal((await admin.rpc('loudness:progress')).enabled, false);
  await admin.json('POST', '/api/send/settings%3Aset', { args: [{ normalize: true }] });
  await until(async () => { const p = await admin.rpc('loudness:progress'); return p.done === 2 && !p.running; }, 60000);

  const after = await admin.rpc('lib:tracks');
  const level = (name) => after.find((x) => x.path.endsWith(name));
  assert.ok(Number.isFinite(level('fort.wav').loudness) && Number.isFinite(level('doux.wav').loudness));
  assert.ok(level('fort.wav').loudness > level('doux.wav').loudness + 10, `${level('fort.wav').loudness} vs ${level('doux.wav').loudness}`);
  assert.ok(Math.abs(level('fort.wav').truePeak - -20) < 1, `crête du sinus (0,125 × 0,8 = −20 dBFS) : ${level('fort.wav').truePeak}`);
  const levels = await admin.rpc('loudness:levels', [level('fort.wav').id, level('doux.wav').id, 99999]);
  assert.equal(levels[level('fort.wav').id].loudness, level('fort.wav').loudness);
  assert.equal(levels[99999], null);
});

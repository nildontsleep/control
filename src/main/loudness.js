'use strict';

// Mesure du niveau sonore des morceaux (EBU R128, par ffmpeg) en arrière-plan, pour la normalisation du volume : un
// morceau à la fois, sans gêner la lecture ni le serveur, seulement tant que l'option est active. Le lecteur applique
// ensuite un gain par morceau pour que tout sonne au même niveau.

const { execFile } = require('node:child_process');
const fs = require('node:fs');

const MEASURE_SECONDS = 600; // on ne mesure que les 10 premières minutes d'un long fichier
const TIMEOUT_MS = 180 * 1000;

/** Lit le résumé de ffmpeg (stderr) : { loudness, truePeak } en LUFS et dBTP, ou null si le morceau est muet. */
function parseSummary(text) {
  const last = (re) => [...String(text).matchAll(re)].pop()?.[1];
  const loudness = Number(last(/I:\s+(-?\d+(?:\.\d+)?)\s+LUFS/g));
  const peak = Number(last(/Peak:\s+(-?\d+(?:\.\d+)?)\s+dBFS/g));
  if (!Number.isFinite(loudness)) return null;
  return { loudness, truePeak: Number.isFinite(peak) ? peak : null };
}

function measure(ffmpeg, file) {
  return new Promise((resolve, reject) => {
    execFile(ffmpeg, ['-nostdin', '-hide_banner', '-nostats', '-t', String(MEASURE_SECONDS), '-i', file, '-vn', '-af', 'ebur128=peak=true:framelog=quiet', '-f', 'null', '-'],
      { timeout: TIMEOUT_MS, maxBuffer: 4 * 1024 * 1024, windowsHide: true }, (err, stdout, stderr) => {
        const summary = parseSummary(stderr);
        if (summary) resolve(summary);
        else if (err && !/-inf/.test(stderr)) reject(err);
        else resolve(null); // silence : rien à mesurer
      });
  });
}

class Loudness {
  /**
   * @param {object} o
   * @param {object} o.library  sa base (db)
   * @param {object} o.tools  { path('ffmpeg') }
   * @param {object} o.store  réglages (normalize)
   * @param {Function} o.emit  (channel, payload)
   */
  constructor({ library, tools, store, emit }) {
    this.library = library;
    this.tools = tools;
    this.store = store;
    this.emit = emit;
    this.running = null;
    this.closed = false;
    this.lastEmit = 0;
    this.timer = setInterval(() => this.kick(), 5 * 60 * 1000);
    this.timer.unref?.();
    setTimeout(() => this.kick(), 20 * 1000).unref?.();
  }

  get enabled() {
    return !!this.store.get('normalize', false);
  }

  progress() {
    const { db } = this.library;
    const total = db.prepare("SELECT COUNT(*) AS n FROM tracks WHERE kind = 'audio'").get().n;
    const done = db.prepare("SELECT COUNT(*) AS n FROM tracks WHERE kind = 'audio' AND loudness_checked = 1").get().n;
    return { enabled: this.enabled, total, done, running: !!this.running, ffmpeg: !!this.tools.path('ffmpeg') };
  }

  /** Les niveaux de ces morceaux : { id: { loudness, truePeak } } (le lecteur les demande pour ceux qu'il ne connaît pas encore). */
  levels(ids) {
    const out = {};
    const stmt = this.library.db.prepare('SELECT loudness, true_peak AS truePeak FROM tracks WHERE id = ?');
    for (const id of (Array.isArray(ids) ? ids : []).filter(Number.isInteger).slice(0, 50)) out[id] = stmt.get(id) || null;
    return out;
  }

  /** (Re)lance la mesure si elle est activée et qu'il reste des morceaux. */
  kick() {
    if (this.closed || this.running || !this.enabled || !this.tools.path('ffmpeg')) return this.running;
    this.running = this.#loop().catch(() => {}).finally(() => {
      this.running = null;
      this.emit('loudness:progress', this.progress());
    });
    return this.running;
  }

  async #loop() {
    const { db } = this.library;
    const next = db.prepare("SELECT id, path FROM tracks WHERE kind = 'audio' AND loudness_checked = 0 ORDER BY id LIMIT 1");
    const save = db.prepare('UPDATE tracks SET loudness = ?, true_peak = ?, loudness_checked = 1 WHERE id = ?');
    while (!this.closed && this.enabled) {
      const row = next.get();
      if (!row) break;
      let result = null;
      if (fs.existsSync(row.path)) {
        try {
          // eslint-disable-next-line no-await-in-loop
          result = await measure(this.tools.path('ffmpeg'), row.path);
        } catch { /* fichier illisible : marqué comme tenté, on passe au suivant */ }
      }
      if (this.closed) return;
      save.run(result?.loudness ?? null, result?.truePeak ?? null, row.id);
      if (Date.now() - this.lastEmit > 2000) {
        this.lastEmit = Date.now();
        this.emit('loudness:progress', this.progress());
      }
      // eslint-disable-next-line no-await-in-loop
      await new Promise((resolve) => setTimeout(resolve, 40)); // laisse de la place au reste
    }
  }

  /** Les morceaux déjà mesurés sont gardés ; ceci ne sert qu'à tout refaire (ffmpeg mis à jour, par exemple). */
  reset() {
    this.library.db.prepare('UPDATE tracks SET loudness = NULL, true_peak = NULL, loudness_checked = 0').run();
    this.kick();
  }

  close() {
    this.closed = true;
    clearInterval(this.timer);
  }
}

module.exports = { Loudness, parseSummary, measure };

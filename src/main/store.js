'use strict';

const fs = require('node:fs');
const path = require('node:path');

const isObj = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);

function deepMerge(target, patch) {
  for (const [k, v] of Object.entries(patch)) {
    if (isObj(v) && isObj(target[k])) deepMerge(target[k], v);
    else target[k] = isObj(v) ? deepMerge({}, v) : v;
  }
  return target;
}

/** Réglages et état des fenêtres, écrits de façon atomique dans un fichier JSON. */
class Store {
  constructor(file) {
    this.file = file;
    this.data = { settings: {}, window: {}, servers: [], source: 'local' };
    this.timer = null;
    try {
      const raw = JSON.parse(fs.readFileSync(file, 'utf8'));
      this.data = {
        settings: raw.settings || {},
        window: raw.window || {},
        servers: Array.isArray(raw.servers) ? raw.servers : [],
        source: typeof raw.source === 'string' ? raw.source : 'local',
      };
    } catch { /* premier lancement ou fichier illisible */ }
  }

  get settings() {
    return this.data.settings;
  }

  get(key, fallback) {
    return this.data.settings[key] ?? fallback;
  }

  patch(patch) {
    deepMerge(this.data.settings, patch);
    this.save();
  }

  reset() {
    this.data.settings = {};
    this.flush();
  }

  save() {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.flush(), 300);
  }

  flush() {
    clearTimeout(this.timer);
    try {
      fs.mkdirSync(path.dirname(this.file), { recursive: true });
      const tmp = `${this.file}.tmp`;
      fs.writeFileSync(tmp, JSON.stringify(this.data, null, 2));
      fs.renameSync(tmp, this.file);
    } catch (err) {
      console.error('Impossible d’enregistrer les réglages :', err);
    }
  }
}

module.exports = { Store, deepMerge, isObj };

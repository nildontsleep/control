'use strict';

// La réponse « lib:tracks » (toute la liste des morceaux d'un compte), construite par un fil d'exécution à part et
// gardée prête tant que la bibliothèque ne change pas : l'ouverture d'une page ne refait plus la requête à chaque fois.

const path = require('node:path');
const { Worker } = require('node:worker_threads');

class TracksBuilder {
  constructor() {
    this.worker = null;
    this.pending = new Map();
    this.seq = 0;
  }

  #start() {
    const worker = new Worker(path.join(__dirname, 'tracks-worker.js'));
    worker.on('message', ({ id, gz, error }) => {
      const wait = this.pending.get(id);
      if (!wait) return;
      this.pending.delete(id);
      if (error) wait.reject(new Error(error));
      else wait.resolve(Buffer.from(gz.buffer, gz.byteOffset, gz.byteLength));
    });
    const fail = (err) => {
      for (const wait of this.pending.values()) wait.reject(err);
      this.pending.clear();
      if (this.worker === worker) this.worker = null;
    };
    worker.on('error', fail);
    worker.on('exit', () => fail(new Error('Analyse interrompue.')));
    worker.unref();
    return worker;
  }

  /** La réponse JSON compressée { ok: true, result: [...] } pour la base de ce compte. */
  build(file) {
    if (!this.worker) this.worker = this.#start();
    const id = ++this.seq;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.worker.postMessage({ id, file });
    });
  }

  close() {
    this.worker?.terminate();
    this.worker = null;
  }
}

module.exports = { TracksBuilder };

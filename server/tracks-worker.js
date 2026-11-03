'use strict';

// Construit la liste des morceaux d'un compte (requête, JSON, gzip) hors du fil principal : sur une grosse bibliothèque
// (30 000 morceaux : environ une seconde et demie), ces calculs bloqueraient tous les comptes du serveur.

const { parentPort } = require('node:worker_threads');
const zlib = require('node:zlib');
const { DatabaseSync } = require('node:sqlite');
const { TRACK_COLUMNS } = require('../src/main/db');

parentPort.on('message', ({ id, file }) => {
  try {
    const db = new DatabaseSync(file, { readOnly: true });
    try {
      const rows = db.prepare(`SELECT ${TRACK_COLUMNS} FROM tracks`).all();
      const gz = new Uint8Array(zlib.gzipSync(JSON.stringify({ ok: true, result: rows })));
      parentPort.postMessage({ id, gz }, [gz.buffer]);
    } finally {
      db.close();
    }
  } catch (err) {
    parentPort.postMessage({ id, error: String(err?.message || err) });
  }
});

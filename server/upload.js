'use strict';

// Envoi de musique depuis le navigateur : le fichier arrive en flux (jamais en mémoire), dans le dossier du compte
// (<téléchargements>/<nom>/Envois), sous un nom nettoyé, sans écraser l'existant ; il n'est gardé que si c'est bien un média.

const fs = require('node:fs');
const fsp = require('node:fs/promises');
const path = require('node:path');
const crypto = require('node:crypto');
const { Transform } = require('node:stream');
const { pipeline } = require('node:stream/promises');
const { isMediaFile } = require('../src/main/library');
const { httpError } = require('./auth');

const RESERVE_BYTES = 200 * 1024 * 1024; // de la place gardée libre sur le disque
const MAX_PER_USER = 3; // envois simultanés par compte
const MAX_DEPTH = 4;

/** Un nom de fichier sûr : ni chemin, ni caractère interdit, pas de point ou d'espace au bord, 180 caractères au plus. */
function cleanSegment(raw, { keepExt = false } = {}) {
  let name = String(raw ?? '').normalize('NFC').replace(/[\\/:*?"<>|\0-\x1f\x7f]/g, '_').replace(/^[.\s]+|[.\s]+$/g, '');
  if (!name || /^(con|prn|aux|nul|com\d|lpt\d)(\..*)?$/i.test(name)) return null;
  if (name.length > 180) {
    const ext = keepExt ? path.extname(name) : '';
    name = name.slice(0, 180 - ext.length) + ext;
  }
  return name;
}

/** Le sous-dossier demandé (« Album/Disc 1 ») : segments nettoyés, quatre niveaux au plus ; vide si rien de valable. */
function cleanDir(raw) {
  const parts = String(raw || '').split(/[\\/]+/).map((p) => cleanSegment(p)).filter(Boolean);
  return parts.slice(0, MAX_DEPTH);
}

/** Un nom libre dans ce dossier : « titre.mp3 », sinon « titre (2).mp3 »… */
async function freeName(dir, name) {
  const ext = path.extname(name);
  const base = name.slice(0, name.length - ext.length);
  for (let i = 1; i < 500; i++) {
    const candidate = i === 1 ? name : `${base} (${i})${ext}`;
    try {
      await fsp.access(path.join(dir, candidate));
    } catch {
      return candidate;
    }
  }
  return `${base}-${crypto.randomBytes(3).toString('hex')}${ext}`;
}

const active = new Map(); // compte → envois en cours

/**
 * Reçoit un fichier. `entry` : le compte ; `root` : son dossier d'envois ; `maxBytes` : taille maximale.
 * Renvoie { name, size, trackIds }.
 */
async function receive(req, { entry, root, name, dir: subdir, maxBytes }) {
  const base = cleanSegment(name, { keepExt: true });
  if (!base || !isMediaFile(base)) throw httpError(415, 'unsupported', 'Seuls les fichiers audio et vidéo sont acceptés (mp3, flac, m4a, ogg, opus, wav, mp4…).');
  if ((active.get(entry.id) || 0) >= MAX_PER_USER) throw httpError(429, 'too_many', 'Trop d’envois en même temps : patientez un instant.');
  const declared = Number(req.headers['content-length']);
  if (declared > maxBytes) throw httpError(413, 'too_large', `Fichier trop volumineux (${Math.round(maxBytes / 1048576)} Mo au plus).`);

  const dir = path.join(root, ...cleanDir(subdir));
  await fsp.mkdir(dir, { recursive: true });
  try {
    const stat = fs.statfsSync(dir);
    if ((declared || 0) + RESERVE_BYTES > stat.bavail * stat.bsize) throw httpError(507, 'no_space', 'Plus assez de place sur le disque du serveur.');
  } catch (err) {
    if (err.status) throw err; // statfs indisponible : on continue sans cette vérification
  }

  const finalName = await freeName(dir, base);
  const part = path.join(dir, `.${finalName}.${crypto.randomBytes(4).toString('hex')}.part`);
  active.set(entry.id, (active.get(entry.id) || 0) + 1);
  let size = 0;
  try {
    const counter = new Transform({
      transform(chunk, enc, cb) {
        size += chunk.length;
        if (size > maxBytes) cb(httpError(413, 'too_large', `Fichier trop volumineux (${Math.round(maxBytes / 1048576)} Mo au plus).`));
        else cb(null, chunk);
      },
    });
    await pipeline(req, counter, fs.createWriteStream(part, { mode: 0o644 }));
    if (!size) throw httpError(400, 'empty', 'Fichier vide.');
    const final = path.join(dir, finalName);
    await fsp.rename(part, final);
    // La bibliothèque garde les fichiers illisibles sous leur nom ; un envoi, lui, doit être un vrai média.
    try {
      const { parseFile } = await import('music-metadata');
      if (!(await parseFile(final, { skipPostHeaders: true })).format.container) throw new Error('format inconnu');
    } catch {
      await fsp.rm(final, { force: true });
      throw httpError(415, 'unreadable', 'Ce fichier n’a pas pu être lu comme un morceau ou une vidéo.');
    }
    const ids = await entry.core.library.addPaths([final]);
    if (!ids.length) {
      await fsp.rm(final, { force: true });
      throw httpError(415, 'unreadable', 'Ce fichier n’a pas pu être lu comme un morceau ou une vidéo.');
    }
    return { name: finalName, size, trackIds: ids };
  } catch (err) {
    await fsp.rm(part, { force: true }).catch(() => {});
    throw err;
  } finally {
    const left = (active.get(entry.id) || 1) - 1;
    if (left) active.set(entry.id, left); else active.delete(entry.id);
  }
}

module.exports = { receive, cleanSegment, cleanDir, freeName };

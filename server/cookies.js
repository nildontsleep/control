'use strict';

// Fichier cookies.txt d'un compte (version web) : le serveur n'a pas accès au navigateur de l'utilisateur, qui exporte
// lui-même ses cookies YouTube et les envoie ici. Le fichier reste sur le serveur, propre à ce compte, lisible par
// lui seul, et ne sert qu'à yt-dlp.

const fs = require('node:fs');
const path = require('node:path');
const { httpError } = require('./auth');

const MAX_BYTES = 2 * 1024 * 1024;
const HEADER = '# Netscape HTTP Cookie File';

/**
 * Vérifie et remet en forme un fichier au format Netscape : { text, count, domains }.
 * Chaque ligne est un commentaire, ou un cookie à 7 champs séparés par des tabulations (ce que yt-dlp exige).
 */
function normalize(input) {
  if (typeof input !== 'string' || !input.trim()) throw httpError(400, 'invalid_cookies', 'Fichier vide.');
  if (Buffer.byteLength(input) > MAX_BYTES) throw httpError(400, 'invalid_cookies', 'Fichier trop volumineux (2 Mo au plus).');
  if (input.includes('\0')) throw httpError(400, 'invalid_cookies', 'Ce n’est pas un fichier texte.');
  const lines = input.replace(/^﻿/, '').split(/\r\n|\r|\n/);
  const cookies = [];
  const domains = new Set();
  const kept = [];
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].replace(/\s+$/, '');
    if (!line) continue;
    if (line.startsWith('#') && !line.startsWith('#HttpOnly_')) {
      if (kept.length || !/^#\s*(Netscape )?HTTP Cookie File/i.test(line)) kept.push(line);
      continue;
    }
    const fields = line.split('\t');
    if (fields.length !== 7) throw httpError(400, 'invalid_cookies', `Ligne ${i + 1} invalide : ce n’est pas un fichier cookies.txt (format Netscape, champs séparés par des tabulations).`);
    cookies.push(line);
    kept.push(line);
    domains.add(fields[0].replace(/^#HttpOnly_/, '').replace(/^\./, '').toLowerCase());
  }
  if (!cookies.length) throw httpError(400, 'invalid_cookies', 'Aucun cookie dans ce fichier.');
  return { text: `${HEADER}\n${kept.join('\n')}\n`, count: cookies.length, domains: [...domains].sort() };
}

function save(file, input) {
  const { text, count, domains } = normalize(input);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, text, { mode: 0o600 });
  fs.renameSync(tmp, file);
  return { count, domains };
}

function status(file) {
  try {
    const stat = fs.statSync(file);
    const { count, domains } = normalize(fs.readFileSync(file, 'utf8'));
    return { present: true, savedAt: stat.mtimeMs, count, domains: domains.slice(0, 8) };
  } catch {
    return { present: false };
  }
}

function clear(file) {
  fs.rmSync(file, { force: true });
}

module.exports = { normalize, save, status, clear, MAX_BYTES };

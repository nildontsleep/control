'use strict';

// Archives .tgz du serveur : écriture (pour l'archive publiée avec chaque release) et lecture stricte (pour celle que
// le serveur télécharge). Seuls les fichiers et les dossiers sont acceptés : ni lien, ni chemin qui sortirait du
// dossier d'extraction, ni archive démesurée. Format ustar, sans dépendance.

const zlib = require('node:zlib');

const BLOCK = 512;
const LIMITS = { maxFiles: 5000, maxFileBytes: 64 * 1024 * 1024, maxTotalBytes: 200 * 1024 * 1024 };

const field = (buf, start, length) => {
  const end = buf.indexOf(0, start);
  return buf.toString('utf8', start, end === -1 || end > start + length ? start + length : end);
};

const octal = (buf, start, length) => {
  if (buf[start] & 0x80) throw new Error('archive : taille non prise en charge');
  const text = field(buf, start, length).trim();
  if (text && !/^[0-7]+$/.test(text)) throw new Error('archive : en-tête illisible');
  return text ? parseInt(text, 8) : 0;
};

/** Le chemin d'une entrée : relatif, sans « .. », sans séparateur Windows ni caractère de contrôle. */
function safeName(raw) {
  const name = String(raw).replace(/^(\.\/)+/, '').replace(/\/+$/, '');
  if (!name || name.startsWith('/') || /[\\\0-\x1f]/.test(name) || /^[a-zA-Z]:/.test(name)) throw new Error(`archive : chemin refusé (${JSON.stringify(raw)})`);
  if (name.split('/').some((part) => part === '..' || part === '' || part === '.')) throw new Error(`archive : chemin refusé (${JSON.stringify(raw)})`);
  return name;
}

/**
 * Lit une archive .tgz (en mémoire) : [{ name, data, mode }] pour les fichiers. Lève une erreur au moindre doute.
 */
function unpack(gz, limits = {}) {
  const max = { ...LIMITS, ...limits };
  const raw = zlib.gunzipSync(gz, { maxOutputLength: max.maxTotalBytes + 64 * BLOCK });
  const files = [];
  const seen = new Set();
  let total = 0;
  for (let at = 0; at + BLOCK <= raw.length;) {
    const head = raw.subarray(at, at + BLOCK);
    if (head.every((b) => b === 0)) break; // fin de l'archive
    let sum = 0;
    for (let i = 0; i < BLOCK; i++) sum += i >= 148 && i < 156 ? 32 : head[i];
    if (sum !== octal(head, 148, 8)) throw new Error('archive : en-tête corrompu');
    const prefix = field(head, 345, 155);
    const name = safeName(prefix ? `${prefix}/${field(head, 0, 100)}` : field(head, 0, 100));
    const size = octal(head, 124, 12);
    const type = String.fromCharCode(head[156] || 48);
    at += BLOCK;
    if (type === '5') continue; // dossier : créé avec les fichiers qu'il contient
    if (type !== '0') throw new Error(`archive : entrée non prise en charge (${JSON.stringify(name)}, type ${JSON.stringify(type)})`);
    if (size > max.maxFileBytes || (total += size) > max.maxTotalBytes || files.length >= max.maxFiles) throw new Error('archive : trop volumineuse');
    if (at + size > raw.length) throw new Error('archive : tronquée');
    if (seen.has(name)) throw new Error(`archive : fichier en double (${JSON.stringify(name)})`);
    seen.add(name);
    files.push({ name, data: Buffer.from(raw.subarray(at, at + size)), mode: octal(head, 100, 8) & 0o777 });
    at += Math.ceil(size / BLOCK) * BLOCK;
  }
  // Un fichier ne peut pas être aussi un dossier (« a » et « a/b »).
  for (const name of seen) {
    for (let i = name.indexOf('/'); i !== -1; i = name.indexOf('/', i + 1)) if (seen.has(name.slice(0, i))) throw new Error(`archive : chemin incohérent (${JSON.stringify(name)})`);
  }
  return files;
}

const put = (buf, text, start, length) => buf.write(text, start, length, 'utf8');
const putOctal = (buf, value, start, length) => put(buf, `${value.toString(8).padStart(length - 1, '0')}\0`, start, length);

/** Écrit une archive .tgz déterministe (dates à zéro) : [{ name, data, mode? }] → Buffer. */
function pack(files) {
  const parts = [];
  for (const { name, data, mode = 0o644 } of files) {
    const path = safeName(name);
    let base = path;
    let prefix = '';
    if (Buffer.byteLength(path) > 100) {
      const cut = path.lastIndexOf('/', 155 + 100);
      base = cut > 0 ? path.slice(cut + 1) : path;
      prefix = cut > 0 ? path.slice(0, cut) : '';
      if (!prefix || Buffer.byteLength(prefix) > 155 || Buffer.byteLength(base) > 100) throw new Error(`archive : chemin trop long (${path})`);
    }
    const body = Buffer.isBuffer(data) ? data : Buffer.from(data);
    const head = Buffer.alloc(BLOCK);
    put(head, base, 0, 100);
    putOctal(head, mode & 0o777, 100, 8);
    putOctal(head, 0, 108, 8);
    putOctal(head, 0, 116, 8);
    putOctal(head, body.length, 124, 12);
    putOctal(head, 0, 136, 12);
    head.fill(32, 148, 156);
    head[156] = 48;
    put(head, 'ustar\0', 257, 6);
    put(head, '00', 263, 2);
    put(head, prefix, 345, 155);
    let sum = 0;
    for (const b of head) sum += b;
    put(head, `${sum.toString(8).padStart(6, '0')}\0 `, 148, 8);
    parts.push(head, body, Buffer.alloc((BLOCK - (body.length % BLOCK)) % BLOCK));
  }
  parts.push(Buffer.alloc(BLOCK * 2));
  return zlib.gzipSync(Buffer.concat(parts), { level: 9 });
}

module.exports = { pack, unpack, safeName, LIMITS };

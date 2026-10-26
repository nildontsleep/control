'use strict';

// Petit serveur HTTP sans dépendance : routage, cookies, corps JSON, fichiers statiques et médias (requêtes Range), gzip.

const fs = require('node:fs');
const path = require('node:path');
const zlib = require('node:zlib');
const { promisify } = require('node:util');

const gzip = promisify(zlib.gzip);

const MIME = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8', '.json': 'application/json; charset=utf-8', '.webmanifest': 'application/manifest+json',
  '.svg': 'image/svg+xml', '.png': 'image/png', '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.webp': 'image/webp', '.ico': 'image/x-icon',
  '.woff2': 'font/woff2', '.woff': 'font/woff', '.ttf': 'font/ttf', '.txt': 'text/plain; charset=utf-8',
  // Médias
  '.mp3': 'audio/mpeg', '.flac': 'audio/flac', '.m4a': 'audio/mp4', '.m4b': 'audio/mp4', '.aac': 'audio/aac',
  '.ogg': 'audio/ogg', '.oga': 'audio/ogg', '.opus': 'audio/ogg', '.wav': 'audio/wav',
  '.mp4': 'video/mp4', '.m4v': 'video/mp4', '.mov': 'video/mp4', '.webm': 'video/webm', '.mkv': 'video/webm',
};
const COMPRESSIBLE = /^(text\/|application\/(json|manifest\+json|javascript)|image\/svg)/;

// Fichiers de l'application déjà compressés, gardés en mémoire (clé : chemin + empreinte) : on ne recompresse pas à chaque chargement.
const packed = new Map();
const PACKED_MAX = 400;

const mimeOf = (file) => MIME[path.extname(file).toLowerCase()] || 'application/octet-stream';

function parseCookies(header) {
  const out = {};
  for (const part of String(header || '').split(';')) {
    const i = part.indexOf('=');
    if (i < 0) continue;
    const name = part.slice(0, i).trim();
    if (name) out[name] = decodeURIComponent(part.slice(i + 1).trim());
  }
  return out;
}

function cookieHeader(name, value, { maxAge, secure, httpOnly = true, sameSite = 'Lax', path: p = '/' } = {}) {
  const parts = [`${name}=${encodeURIComponent(value)}`, `Path=${p}`, `SameSite=${sameSite}`];
  if (httpOnly) parts.push('HttpOnly');
  if (secure) parts.push('Secure');
  if (maxAge !== undefined) parts.push(`Max-Age=${maxAge}`);
  return parts.join('; ');
}

/** Corps JSON de la requête (limité en taille). */
function readJson(req, limit = 40 * 1024 * 1024) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    req.on('data', (chunk) => {
      size += chunk.length;
      if (size > limit) {
        reject(Object.assign(new Error('Requête trop volumineuse.'), { status: 413, code: 'too_large' }));
        req.destroy();
        return;
      }
      chunks.push(chunk);
    });
    req.on('end', () => {
      if (!size) return resolve({});
      try {
        resolve(JSON.parse(Buffer.concat(chunks).toString('utf8')));
      } catch {
        reject(Object.assign(new Error('JSON invalide.'), { status: 400, code: 'bad_json' }));
      }
    });
    req.on('error', reject);
  });
}

function send(res, status, body, headers = {}) {
  if (res.headersSent) return;
  res.writeHead(status, { 'x-content-type-options': 'nosniff', ...headers });
  res.end(body);
}

/** Répond en JSON, compressé si le navigateur le sait et que la réponse est assez grosse pour que ça serve. */
async function sendJson(req, res, status, data, headers = {}) {
  const raw = Buffer.from(JSON.stringify(data));
  const base = { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store', ...headers };
  if (raw.length > 1024 && /\bgzip\b/.test(req.headers['accept-encoding'] || '')) {
    const packed = await gzip(raw);
    return send(res, status, packed, { ...base, 'content-encoding': 'gzip', vary: 'Accept-Encoding', 'content-length': packed.length });
  }
  return send(res, status, raw, { ...base, 'content-length': raw.length });
}

async function sendBuffer(req, res, status, buffer, type, headers = {}) {
  const base = { 'content-type': type, ...headers };
  if (COMPRESSIBLE.test(type) && buffer.length > 1024 && /\bgzip\b/.test(req.headers['accept-encoding'] || '')) {
    const packed = await gzip(buffer);
    return send(res, status, packed, { ...base, 'content-encoding': 'gzip', vary: 'Accept-Encoding', 'content-length': packed.length });
  }
  return send(res, status, buffer, { ...base, 'content-length': buffer.length });
}

/**
 * Envoie un fichier : requêtes Range (lecture et déplacement dans un morceau), ETag, gzip pour le texte.
 * cache : valeur de Cache-Control.
 */
async function sendFile(req, res, file, { type = mimeOf(file), cache = 'no-cache', headers = {} } = {}) {
  let stat;
  try {
    stat = await fs.promises.stat(file);
    if (!stat.isFile()) throw new Error('not a file');
  } catch {
    return send(res, 404, 'Not found', { 'content-type': 'text/plain' });
  }
  const etag = `"${stat.size.toString(16)}-${Math.floor(stat.mtimeMs).toString(16)}"`;
  const base = { 'content-type': type, 'accept-ranges': 'bytes', 'cache-control': cache, etag, 'last-modified': stat.mtime.toUTCString(), ...headers };
  if (req.headers['if-none-match'] === etag) return send(res, 304, null, base);

  const range = /^bytes=(\d*)-(\d*)$/.exec(req.headers.range || '');
  if (range && stat.size > 0 && (!req.headers['if-range'] || req.headers['if-range'] === etag)) {
    let start = range[1] ? parseInt(range[1], 10) : 0;
    let end = range[2] ? parseInt(range[2], 10) : stat.size - 1;
    if (!range[1] && range[2]) {
      start = Math.max(0, stat.size - parseInt(range[2], 10));
      end = stat.size - 1;
    }
    end = Math.min(end, stat.size - 1);
    if (start > end || start >= stat.size) return send(res, 416, null, { 'content-range': `bytes */${stat.size}` });
    res.writeHead(206, { ...base, 'x-content-type-options': 'nosniff', 'content-range': `bytes ${start}-${end}/${stat.size}`, 'content-length': end - start + 1 });
    if (req.method === 'HEAD') return res.end();
    const stream = fs.createReadStream(file, { start, end });
    stream.on('error', () => res.destroy());
    res.on('close', () => stream.destroy());
    return stream.pipe(res);
  }

  if (COMPRESSIBLE.test(type) && stat.size < 4 * 1024 * 1024 && /\bgzip\b/.test(req.headers['accept-encoding'] || '') && req.method !== 'HEAD') {
    const key = `${file}|${etag}`;
    let body = packed.get(key);
    if (!body) {
      body = await gzip(await fs.promises.readFile(file));
      if (packed.size >= PACKED_MAX) packed.delete(packed.keys().next().value);
      packed.set(key, body);
    }
    return send(res, 200, body, { ...base, 'content-encoding': 'gzip', vary: 'Accept-Encoding', 'content-length': body.length });
  }
  res.writeHead(200, { ...base, 'x-content-type-options': 'nosniff', 'content-length': stat.size });
  if (req.method === 'HEAD') return res.end();
  const stream = fs.createReadStream(file);
  stream.on('error', () => res.destroy());
  res.on('close', () => stream.destroy());
  return stream.pipe(res);
}

/** Chemin demandé dans un dossier, sans sortir de celui-ci (null si c'est le cas). */
function safeJoin(root, urlPath) {
  let decoded;
  try {
    decoded = decodeURIComponent(urlPath);
  } catch {
    return null;
  }
  if (decoded.includes('\0')) return null;
  const file = path.normalize(path.join(root, decoded));
  return file === root || file.startsWith(root + path.sep) ? file : null;
}

module.exports = { MIME, mimeOf, parseCookies, cookieHeader, readJson, send, sendJson, sendBuffer, sendFile, safeJoin };

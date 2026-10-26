'use strict';

// Images envoyées par l'utilisateur (avatar, pochette de playlist) : décodage, recadrage, réduction, enregistrement
// par empreinte du contenu. Même code sur le bureau (nativeImage) et sur le serveur (image-lite).

const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const platform = require('./platform');

const MAX_BYTES = 12 * 1024 * 1024;

/** Contenu d'une data URL (ou d'un Buffer) → Buffer, ou null. */
function toBuffer(src) {
  if (Buffer.isBuffer(src)) return src.length <= MAX_BYTES ? src : null;
  const match = /^data:image\/(?:jpeg|jpg|png|webp);base64,([A-Za-z0-9+/=]+)$/.exec(String(src || ''));
  if (!match) return null;
  const buffer = Buffer.from(match[1], 'base64');
  return buffer.length && buffer.length <= MAX_BYTES ? buffer : null;
}

const isWebp = (b) => b.length > 12 && b.toString('ascii', 0, 4) === 'RIFF' && b.toString('ascii', 8, 12) === 'WEBP';

/** Une vraie image ? (Décodable, ou WebP : le navigateur sait l'afficher même si ce code ne sait pas la décoder.) */
function isImage(buffer) {
  return !platform.nativeImage.createFromBuffer(buffer).isEmpty() || isWebp(buffer);
}

/** Image carrée (recadrée au centre) de `size` px de côté, en JPEG. null si le format n'est pas décodable. */
function squareJpeg(buffer, size = 320) {
  const image = platform.nativeImage.createFromBuffer(buffer);
  if (image.isEmpty()) return null;
  const { width, height } = image.getSize();
  const side = Math.min(width, height);
  const cropped = width === height ? image : image.crop({ x: Math.round((width - side) / 2), y: Math.round((height - side) / 2), width: side, height: side });
  const scaled = side > size ? cropped.resize({ width: size, height: size, quality: 'best' }) : cropped;
  return scaled.toJPEG(88);
}

/** Enregistre un JPEG dans `dir` sous une empreinte de 20 caractères ; renvoie l'empreinte. */
function saveByHash(dir, jpeg) {
  const hash = crypto.createHash('sha1').update(jpeg).digest('hex').slice(0, 20);
  fs.mkdirSync(dir, { recursive: true });
  const file = path.join(dir, `${hash}.jpg`);
  if (!fs.existsSync(file)) fs.writeFileSync(file, jpeg);
  return hash;
}

module.exports = { toBuffer, isImage, squareJpeg, saveByHash, MAX_BYTES };

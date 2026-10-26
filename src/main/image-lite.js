'use strict';

// Sous-ensemble de nativeImage en JavaScript pur (JPEG et PNG), pour le serveur web où Electron n'est pas là.
// Même interface que celle dont la bibliothèque et l'éditeur de tags ont besoin : createFromBuffer, isEmpty, getSize,
// resize, toJPEG. Les autres formats (WebP, GIF…) donnent une image « vide » : l'appelant les garde tels quels.

const jpeg = require('jpeg-js');
const { PNG } = require('pngjs');

const MAX_PIXELS = 64e6;

class LiteImage {
  constructor(width = 0, height = 0, data = null) {
    this.width = width;
    this.height = height;
    this.data = data; // RGBA
  }

  isEmpty() {
    return !this.data || !this.width || !this.height;
  }

  getSize() {
    return { width: this.width, height: this.height };
  }

  /** Réduction par moyenne des pixels couverts (nette pour les miniatures), agrandissement bilinéaire sinon. */
  resize({ width, height }) {
    const w = Math.max(1, Math.round(width));
    const h = Math.max(1, Math.round(height));
    if (this.isEmpty() || (w === this.width && h === this.height)) return this;
    const out = Buffer.alloc(w * h * 4);
    const src = this.data;
    const sw = this.width;
    const sh = this.height;
    const kx = sw / w;
    const ky = sh / h;
    for (let y = 0; y < h; y++) {
      const y0 = y * ky;
      const y1 = Math.min(sh, y0 + ky);
      for (let x = 0; x < w; x++) {
        const x0 = x * kx;
        const x1 = Math.min(sw, x0 + kx);
        let r = 0;
        let g = 0;
        let b = 0;
        let a = 0;
        let total = 0;
        if (kx <= 1 || ky <= 1) {
          // Agrandissement (ou étirement dans un seul sens) : interpolation bilinéaire au centre du pixel.
          const cx = Math.min(sw - 1, Math.max(0, (x + 0.5) * kx - 0.5));
          const cy = Math.min(sh - 1, Math.max(0, (y + 0.5) * ky - 0.5));
          const ix = Math.floor(cx);
          const iy = Math.floor(cy);
          const fx = cx - ix;
          const fy = cy - iy;
          const jx = Math.min(sw - 1, ix + 1);
          const jy = Math.min(sh - 1, iy + 1);
          for (const [px, py, wgt] of [[ix, iy, (1 - fx) * (1 - fy)], [jx, iy, fx * (1 - fy)], [ix, jy, (1 - fx) * fy], [jx, jy, fx * fy]]) {
            const i = (py * sw + px) * 4;
            r += src[i] * wgt; g += src[i + 1] * wgt; b += src[i + 2] * wgt; a += src[i + 3] * wgt;
          }
          total = 1;
        } else {
          for (let sy = Math.floor(y0); sy < Math.ceil(y1); sy++) {
            const wy = Math.min(sy + 1, y1) - Math.max(sy, y0);
            for (let sx = Math.floor(x0); sx < Math.ceil(x1); sx++) {
              const wgt = wy * (Math.min(sx + 1, x1) - Math.max(sx, x0));
              const i = (sy * sw + sx) * 4;
              r += src[i] * wgt; g += src[i + 1] * wgt; b += src[i + 2] * wgt; a += src[i + 3] * wgt;
              total += wgt;
            }
          }
        }
        const o = (y * w + x) * 4;
        const t = total || 1;
        out[o] = r / t; out[o + 1] = g / t; out[o + 2] = b / t; out[o + 3] = a / t;
      }
    }
    return new LiteImage(w, h, out);
  }

  /** JPEG (les pixels transparents sont posés sur du blanc, comme le fait le navigateur pour une pochette). */
  toJPEG(quality = 90) {
    const flat = Buffer.from(this.data);
    for (let i = 0; i < flat.length; i += 4) {
      const a = flat[i + 3] / 255;
      if (a < 1) {
        flat[i] = flat[i] * a + 255 * (1 - a);
        flat[i + 1] = flat[i + 1] * a + 255 * (1 - a);
        flat[i + 2] = flat[i + 2] * a + 255 * (1 - a);
        flat[i + 3] = 255;
      }
    }
    return jpeg.encode({ data: flat, width: this.width, height: this.height }, quality).data;
  }

  toPNG() {
    const png = new PNG({ width: this.width, height: this.height });
    Buffer.from(this.data).copy(png.data);
    return PNG.sync.write(png);
  }
}

function createFromBuffer(buffer) {
  try {
    if (buffer[0] === 0xff && buffer[1] === 0xd8) {
      const img = jpeg.decode(buffer, { useTArray: true, formatAsRGBA: true, maxMemoryUsageInMB: 768, tolerantDecoding: true });
      if (img.width * img.height > MAX_PIXELS) return new LiteImage();
      return new LiteImage(img.width, img.height, img.data);
    }
    if (buffer[0] === 0x89 && buffer[1] === 0x50 && buffer[2] === 0x4e && buffer[3] === 0x47) {
      const img = PNG.sync.read(buffer);
      if (img.width * img.height > MAX_PIXELS) return new LiteImage();
      return new LiteImage(img.width, img.height, img.data);
    }
  } catch { /* image abîmée : vide */ }
  return new LiteImage();
}

/** Pas de miniature système d'un fichier : l'appelant retombe sur ffmpeg. */
async function createThumbnailFromPath() {
  throw new Error('Miniatures système indisponibles');
}

module.exports = { createFromBuffer, createThumbnailFromPath, LiteImage };

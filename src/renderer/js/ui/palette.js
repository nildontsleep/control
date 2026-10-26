// Couleurs d'une pochette : de quoi teinter le fond du mode immersif.

import { coverUrl, seedHues } from './cover.js';

const SIZE = 36;
const cache = new Map(); // hash de pochette → promesse de [ [h, s, l] × 4 ]

const clamp = (v, a, b) => Math.min(b, Math.max(a, v));

function toHsl(r, g, b) {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  const d = max - min;
  if (d === 0) return [0, 0, l];
  const s = d / (1 - Math.abs(2 * l - 1));
  let hue;
  if (max === r) hue = ((g - b) / d) % 6;
  else if (max === g) hue = (b - r) / d + 2;
  else hue = (r - g) / d + 4;
  return [(hue * 60 + 360) % 360, s, l];
}

/** Les teintes dominantes (les plus présentes et les plus vives), sinon des variantes de la teinte moyenne. */
function pick(pixels) {
  const buckets = Array.from({ length: 12 }, () => ({ w: 0, h: 0, hx: 0, hy: 0, s: 0, l: 0, n: 0 }));
  let tl = 0;
  let ts = 0;
  for (const [h, s, l] of pixels) {
    tl += l;
    ts += s;
    if (s < 0.18 || l < 0.1 || l > 0.92) continue;
    // Les teintes vives et mi-claires pèsent plus que les ternes.
    const w = s * (1 - Math.abs(l - 0.5) * 1.2);
    const b = buckets[Math.floor(h / 30) % 12];
    b.w += w;
    b.hx += Math.cos((h * Math.PI) / 180) * w;
    b.hy += Math.sin((h * Math.PI) / 180) * w;
    b.s += s * w;
    b.l += l * w;
    b.n += 1;
  }
  const top = buckets.filter((b) => b.w > 0).sort((a, b) => b.w - a.w).slice(0, 3).map((b) => [
    (Math.atan2(b.hy, b.hx) * 180 / Math.PI + 360) % 360, b.s / b.w, b.l / b.w, b.w,
  ]);
  const meanL = tl / pixels.length;
  const meanS = ts / pixels.length;
  const colors = top.map(([h, s, l]) => [h, clamp(s * 1.15, 0.5, 0.95), clamp(l, 0.4, 0.62)]);
  if (!colors.length) {
    // Pochette presque grise : une teinte neutre, très peu saturée.
    colors.push([220, clamp(meanS, 0.05, 0.2), clamp(meanL, 0.35, 0.6)]);
  }
  // Complète à quatre couleurs en décalant légèrement la teinte des premières.
  let i = 0;
  while (colors.length < 4) {
    const [h, s, l] = colors[i++ % colors.length];
    colors.push([(h + 28 + 17 * colors.length) % 360, s, clamp(l + (colors.length % 2 ? 0.08 : -0.06), 0.34, 0.66)]);
  }
  return colors.slice(0, 4);
}

function fromImage(url) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.decoding = 'async';
    img.onload = () => {
      try {
        const canvas = document.createElement('canvas');
        canvas.width = SIZE;
        canvas.height = SIZE;
        const ctx = canvas.getContext('2d', { willReadFrequently: true });
        ctx.drawImage(img, 0, 0, SIZE, SIZE);
        const data = ctx.getImageData(0, 0, SIZE, SIZE).data;
        const pixels = [];
        for (let i = 0; i < data.length; i += 4) pixels.push(toHsl(data[i], data[i + 1], data[i + 2]));
        resolve(pick(pixels));
      } catch (err) {
        reject(err);
      }
    };
    img.onerror = () => reject(new Error('pochette illisible'));
    img.src = url;
  });
}

/** Quatre couleurs [teinte°, saturation, luminosité] pour un morceau : d'après sa pochette, ou d'après son dégradé de repli. */
export function paletteOf(track) {
  if (!track) return Promise.resolve(null);
  const key = track.cover || `seed:${track.album || track.title || ''}`;
  if (!cache.has(key)) {
    const fallback = () => {
      const [a, b] = seedHues(track.album || track.title || '');
      return [[a, 0.64, 0.6], [b, 0.58, 0.45], [(a + 70) % 360, 0.55, 0.55], [(b + 40) % 360, 0.5, 0.4]];
    };
    cache.set(key, track.cover ? fromImage(coverUrl(track.cover, true)).catch(fallback) : Promise.resolve(fallback()));
    if (cache.size > 80) cache.delete(cache.keys().next().value);
  }
  return cache.get(key);
}

export const hsl = ([h, s, l]) => `hsl(${Math.round(h)} ${Math.round(s * 100)}% ${Math.round(l * 100)}%)`;

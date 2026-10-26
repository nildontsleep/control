// Pochettes : image réelle ou dégradé déterministe (comme Apple Music) quand il n’y en a pas.

import { h } from '../dom.js';
import { icon } from '../icons.js';

function hashString(str) {
  let hash = 2166136261;
  for (let i = 0; i < str.length; i++) {
    hash ^= str.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

/** Les deux teintes du dégradé de repli d'un morceau sans pochette. */
export function seedHues(seed) {
  const hue = hashString(seed || '?') % 360;
  return [hue, (hue + 38 + (hashString(`${seed}x`) % 40)) % 360];
}

export function gradientFor(seed) {
  const [hue, hue2] = seedHues(seed);
  return `linear-gradient(150deg, hsl(${hue} 64% 60%), hsl(${hue2} 58% 32%))`;
}

export const coverUrl = (hash, small = false) => (hash ? `app://nilcam/cover/${hash}${small ? '-s' : ''}.jpg` : null);

/**
 * item : morceau, album, artiste ou playlist ({ cover, title|name, kind, id }).
 * options : { small } utilise la miniature 160 px.
 */
export function coverEl(item, { small = false, className = '', round = false } = {}) {
  const seed = item.album || item.title || item.name || '';
  const el = h(`div.cover${className ? `.${className}` : ''}`, { style: { background: gradientFor(seed) } });
  if (round) el.classList.add('round');
  const isVideo = item.kind === 'video';
  el.append(h('span.cover-icon', { html: icon(isVideo ? 'film' : item.isPlaylist ? 'list-music' : item.isArtist ? 'user' : 'music-2', 22, 1.6) }));
  const src = item.cover ? coverUrl(item.cover, small) : isVideo && item.id ? `app://nilcam/vthumb/${item.id}` : null;
  if (src) {
    const img = h('img', { src, loading: 'lazy', decoding: 'async', alt: '', draggable: 'false' });
    img.addEventListener('load', () => el.classList.add('loaded'));
    img.addEventListener('error', () => img.remove());
    el.append(img);
  }
  return el;
}

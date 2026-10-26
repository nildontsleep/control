// Avatars des comptes (version web) : la photo, sinon l'initiale sur une couleur tirée du nom. Les changements de profil
// du compte connecté sont diffusés pour que la barre latérale et les pages ouvertes se mettent à jour sans recharger.

import { h } from '../dom.js';
import { avatarUrl } from '../urls.js';

function hue(text) {
  let hash = 2166136261;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0) % 360;
}

/** person : { username, name?, avatar?, color? } ; size en px. */
export function avatarEl(person, { size = 40, className = '' } = {}) {
  const label = person.name || person.username || '?';
  const el = h(`span.avatar${className ? `.${className}` : ''}`, { 'aria-hidden': 'true' });
  el.style.setProperty('--size', `${size}px`);
  el.style.setProperty('--c', person.color || `hsl(${hue(person.username || label)} 62% 46%)`);
  if (person.avatar) el.append(h('img', { src: avatarUrl(person.avatar), alt: '', draggable: 'false', loading: 'lazy' }));
  else el.append(h('span.avatar-initial', { text: [...label.trim()][0]?.toUpperCase() || '?' }));
  return el;
}

const listeners = new Set();

/** Appelé quand le profil du compte connecté change ; renvoie la fonction qui se désabonne. */
export function onMyProfile(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function announceProfile(profile) {
  const me = window.control.web.user;
  Object.assign(me, { name: profile.name, avatar: profile.avatar, color: profile.color });
  for (const fn of listeners) fn(me);
}

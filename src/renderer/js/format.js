// Formats d’affichage : durées, tailles, dates, pluriels.

import { currentLanguage } from './i18n.js';

export function fmtTime(seconds) {
  if (!Number.isFinite(seconds) || seconds < 0) return '0:00';
  const s = Math.floor(seconds);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`;
}

/** « 1 h 24 min », « 42 min ». */
export function fmtLong(seconds) {
  const total = Math.round((seconds || 0) / 60);
  const h = Math.floor(total / 60);
  const m = total % 60;
  if (h && m) return `${h} h ${m} min`;
  if (h) return `${h} h`;
  return `${Math.max(1, m)} min`;
}

export function fmtBytes(bytes) {
  if (!Number.isFinite(bytes)) return '';
  const units = ['o', 'Ko', 'Mo', 'Go'];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  const lang = currentLanguage();
  const unit = lang === 'fr' ? units[i] : units[i].replace('o', 'B');
  return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)} ${unit}`;
}

export function fmtDate(ms) {
  if (!ms) return '';
  return new Date(ms).toLocaleDateString(currentLanguage(), { day: 'numeric', month: 'short', year: 'numeric' });
}

export function fmtRelative(ms) {
  if (!ms) return '';
  const days = Math.floor((Date.now() - ms) / 86400000);
  const rtf = new Intl.RelativeTimeFormat(currentLanguage(), { numeric: 'auto' });
  if (days < 1) {
    const hours = Math.floor((Date.now() - ms) / 3600000);
    return hours < 1 ? rtf.format(-Math.max(1, Math.floor((Date.now() - ms) / 60000)), 'minute') : rtf.format(-hours, 'hour');
  }
  if (days < 30) return rtf.format(-days, 'day');
  if (days < 365) return rtf.format(-Math.floor(days / 30), 'month');
  return rtf.format(-Math.floor(days / 365), 'year');
}

export const count = (n) => new Intl.NumberFormat(currentLanguage()).format(n);

/** Normalise pour la recherche : minuscules, sans accents. */
export const fold = (s) => String(s || '').normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase();

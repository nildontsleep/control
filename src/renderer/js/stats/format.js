// Formats propres aux statistiques (durées lisibles, jours).

import { t, currentLanguage } from '../i18n.js';

/** 0 → « — », 40 s → « < 1 min », 7 500 s → « 2 h 05 min ». */
export function fmtDuration(seconds) {
  const s = Math.max(0, Math.round(seconds || 0));
  if (!s) return '0 min';
  if (s < 60) return t('stats.lessMinute');
  const total = Math.round(s / 60);
  if (total < 60) return `${total} min`;
  const hours = Math.floor(total / 60);
  const minutes = total % 60;
  const grouped = new Intl.NumberFormat(currentLanguage()).format(hours);
  if (hours >= 100 || !minutes) return `${grouped} h`;
  return `${grouped} h ${String(minutes).padStart(2, '0')} min`;
}

export const dayLabel = (date) => date.toLocaleDateString(currentLanguage(), { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' });

/** Étiquette courte sous une barre : jour de la semaine sur 7 jours, sinon numéro (avec le mois le 1er). */
export function shortDay(date, days) {
  const lang = currentLanguage();
  if (days <= 7) return date.toLocaleDateString(lang, { weekday: 'short' }).replace('.', '');
  return date.getDate() === 1 ? date.toLocaleDateString(lang, { day: 'numeric', month: 'short' }) : String(date.getDate());
}

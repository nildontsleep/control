// Propose le récap du mois écoulé en début de mois, et celui de la semaine écoulée en début de semaine (une fois chacun).

import { settings, update } from '../settings.js';
import { t, currentLanguage } from '../i18n.js';
import { toast } from '../ui/feedback.js';
import { openWrapped, periodRange, periodKey, monthKey, mondayOf, weeksOf } from '../ui/wrapped.js';

const api = window.control.stats;

const load = async (range) => {
  const res = await api.get(range);
  if (!res.ok) throw new Error(res.message);
  return res.data;
};

const openOptions = (data) => ({ years: data.years, months: data.months, weeks: weeksOf(data.days || []), days: data.days || [], load });
const enough = (data) => data.music.seconds >= 1800 || data.video.seconds >= 3600;

/** true si une proposition a été affichée. */
async function offerMonth(now) {
  if (now.getDate() > 12) return false;
  const before = new Date(now.getFullYear(), now.getMonth() - 1, 1);
  const period = { kind: 'month', year: before.getFullYear(), month: before.getMonth() + 1 };
  const key = monthKey(period);
  if (settings.recapSeen === key) return false;
  const res = await api.get(periodRange(period));
  if (!res.ok) return false;
  const data = res.data;
  update({ recapSeen: key }); // une seule proposition, même ignorée
  if (!enough(data)) return false; // pas assez d'écoute pour un récap
  const month = before.toLocaleDateString(currentLanguage(), { month: 'long' });
  toast(t('wrapped.monthReady', { month }), 'sparkles', {
    duration: 14000,
    action: { label: t('wrapped.see'), onClick: () => openWrapped(period, data, openOptions(data)) },
  });
  return true;
}

/** Du lundi au mercredi, la semaine précédente (du lundi au dimanche). */
async function offerWeek(now) {
  const weekday = (now.getDay() + 6) % 7; // lundi = 0
  if (weekday > 2) return false;
  const pad = (n) => String(n).padStart(2, '0');
  const today = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
  const start = mondayOf(today);
  const last = new Date(Number(start.slice(0, 4)), Number(start.slice(5, 7)) - 1, Number(start.slice(8, 10)) - 7);
  const period = { kind: 'week', start: `${last.getFullYear()}-${pad(last.getMonth() + 1)}-${pad(last.getDate())}` };
  const key = periodKey(period);
  if (settings.recapSeenWeek === key) return false;
  const res = await api.get(periodRange(period));
  if (!res.ok) return false;
  const data = res.data;
  update({ recapSeenWeek: key });
  if (!enough(data)) return false;
  toast(t('wrapped.weekReady'), 'sparkles', {
    duration: 14000,
    action: { label: t('wrapped.see'), onClick: () => openWrapped(period, data, openOptions(data)) },
  });
  return true;
}

/** Une seule proposition à la fois : le mois d'abord, la semaine au lancement suivant. */
export async function offerRecaps(now = new Date()) {
  if (await offerMonth(now)) return;
  await offerWeek(now);
}

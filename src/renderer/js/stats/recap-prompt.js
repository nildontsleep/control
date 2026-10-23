// En début de mois, propose le récap du mois écoulé (une seule fois par mois).

import { settings, update } from '../settings.js';
import { t, currentLanguage } from '../i18n.js';
import { toast } from '../ui/feedback.js';
import { openWrapped, periodRange, monthKey } from '../ui/wrapped.js';

const api = window.control.stats;

const load = async (range) => {
  const res = await api.get(range);
  if (!res.ok) throw new Error(res.message);
  return res.data;
};

export async function offerMonthlyRecap(now = new Date()) {
  if (now.getDate() > 12) return;
  const before = new Date(now.getFullYear(), now.getMonth() - 1, 1);
  const period = { kind: 'month', year: before.getFullYear(), month: before.getMonth() + 1 };
  const key = monthKey(period);
  if (settings.recapSeen === key) return;
  const res = await api.get(periodRange(period));
  if (!res.ok) return;
  const data = res.data;
  update({ recapSeen: key }); // une seule proposition, même ignorée
  if (data.music.seconds < 1800 && data.video.seconds < 3600) return; // pas assez d'écoute pour un récap
  const month = before.toLocaleDateString(currentLanguage(), { month: 'long' });
  toast(t('wrapped.monthReady', { month }), 'sparkles', {
    duration: 14000,
    action: { label: t('wrapped.see'), onClick: () => openWrapped(period, data, { years: data.years, months: data.months, load }) },
  });
}

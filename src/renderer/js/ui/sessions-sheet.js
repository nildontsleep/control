// Sessions ouvertes du compte (version web) : navigateurs et téléphones connectés, avec de quoi en fermer une ou toutes les autres.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t, currentLanguage } from '../i18n.js';
import { openSheet, confirmSheet } from './sheet.js';
import { toast } from './feedback.js';

const web = window.control?.web;

/** « Firefox sur Linux », « Safari sur iPad »… à partir de l'en-tête User-Agent (approximatif, pour s'y retrouver). */
export function describeAgent(ua = '') {
  const s = String(ua);
  const os = /iPhone/.test(s) ? 'iPhone' : /iPad/.test(s) ? 'iPad' : /Android/.test(s) ? 'Android' : /Windows/.test(s) ? 'Windows' : /Mac OS X|Macintosh/.test(s) ? 'macOS' : /CrOS/.test(s) ? 'ChromeOS' : /Linux|X11/.test(s) ? 'Linux' : '';
  const browser = /Electron\//.test(s) ? t('sessions.app') : /Edg\//.test(s) ? 'Edge' : /OPR\/|Opera/.test(s) ? 'Opera' : /Firefox\//.test(s) ? 'Firefox' : /Chrome\/|CriOS/.test(s) ? 'Chrome' : /Safari\//.test(s) ? 'Safari' : '';
  const kind = /iPhone|Android.*Mobile/.test(s) ? 'smartphone' : /iPad|Android/.test(s) ? 'tablet' : 'monitor';
  const label = browser && os ? t('sessions.on', { browser, os }) : browser || os || t('sessions.unknown');
  return { label, kind };
}

function ago(ms) {
  const diff = Date.now() - ms;
  if (diff < 90 * 1000) return t('sessions.now');
  const fmt = new Intl.RelativeTimeFormat(currentLanguage(), { numeric: 'auto' });
  const minutes = Math.round(diff / 60000);
  if (minutes < 60) return fmt.format(-minutes, 'minute');
  const hours = Math.round(minutes / 60);
  if (hours < 48) return fmt.format(-hours, 'hour');
  return fmt.format(-Math.round(hours / 24), 'day');
}

export function openSessionsSheet() {
  const list = h('div.ss-list');
  const others = h('button.btn.btn-secondary', { type: 'button', text: t('sessions.revokeOthers') });
  const close = h('button.btn.btn-primary', { type: 'button', text: t('common.close') });
  const sheet = openSheet({ title: t('sessions.title'), width: 520, body: [h('p.sheet-text', { text: t('sessions.intro') }), list], footer: [others, close] });
  close.addEventListener('click', () => sheet.close());

  async function load() {
    const sessions = await web.rpc('srv:me:sessions');
    others.hidden = sessions.length < 2;
    list.replaceChildren(...sessions.map((s) => {
      const { label, kind } = describeAgent(s.userAgent);
      const out = s.current ? null : h('button.btn.btn-secondary.btn-small', { type: 'button', text: t('sessions.revoke'), onclick: async () => {
        await web.rpc('srv:me:revoke-session', s.id);
        toast(t('sessions.revoked'), 'check');
        load();
      } });
      return h('div.ss-row', {},
        h('span.ss-icon', { html: icon(kind, 20, 1.8) }),
        h('div.ss-text', {},
          h('strong', {}, label, s.current ? h('span.ss-badge', { text: t('sessions.current') }) : null),
          h('span', { text: [s.ip, t('sessions.lastSeen', { when: ago(s.lastSeen) })].filter(Boolean).join(' · ') })),
        out);
    }));
  }
  others.addEventListener('click', async () => {
    const { ok } = await confirmSheet({ title: t('sessions.revokeOthers'), message: t('sessions.revokeOthersAsk'), confirmLabel: t('sessions.revokeOthers'), destructive: true });
    if (!ok) return;
    const n = await web.rpc('srv:me:revoke-others');
    toast(t('sessions.revokedN', { n }), 'check');
    load();
  });
  load().catch((err) => toast(err.message, 'triangle-alert'));
}

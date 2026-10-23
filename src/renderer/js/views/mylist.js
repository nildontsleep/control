// « Ma liste » : reprise, titres enregistrés et historique de visionnage.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { fmtRelative } from '../format.js';
import { navigate } from '../router.js';
import { watch, listItems, onList, loadList, watchParam } from '../media/api.js';
import { posterCard, resumeCard, shelf, kindLabel } from '../media/ui.js';
import { confirmSheet } from '../ui/sheet.js';
import { toast } from '../ui/feedback.js';
import { button } from '../ui/controls.js';
import { pageHead, emptyState } from './shared.js';

function historyRow(entry, onRemove) {
  const label = entry.season != null ? `S${entry.season} · E${entry.episode}` : entry.episode != null ? `${t('ep.short')} ${entry.episode}` : null;
  const art = h('div.hist-art', {}, entry.poster ? h('img', { src: entry.poster, alt: '', loading: 'lazy', referrerpolicy: 'no-referrer' }) : null);
  const pct = entry.duration ? Math.min(100, Math.round((entry.position / entry.duration) * 100)) : 0;
  const row = h('div.hist-row', { tabindex: '0' },
    art,
    h('div.hist-text', {},
      h('div.hist-title', { text: entry.title || '—' }),
      h('div.hist-sub', { text: [kindLabel(entry.type), label, fmtRelative(entry.updatedAt)].filter(Boolean).join(' · ') })),
    h('div.hist-status', {}, entry.finished ? h('span.ok', { html: `${icon('circle-check', 16, 2)}<span>${t('hist.watched')}</span>` }) : h('span', { text: `${pct}%` })),
    h('button.icon-btn', { type: 'button', 'data-tip': t('common.remove'), html: icon('x', 15, 2), onclick: (e) => { e.stopPropagation(); onRemove(); } }));
  row.addEventListener('click', () => navigate('title', `${entry.type}:${entry.id}`));
  row.addEventListener('keydown', (e) => { if (e.key === 'Enter') navigate('title', `${entry.type}:${entry.id}`); });
  return row;
}

export function mylistView(root) {
  let destroyed = false;
  const content = h('div.watch-content');
  const clear = button(t('hist.clear'), { icon: 'trash-2', kind: 'danger', small: true, onClick: async () => {
    const { ok } = await confirmSheet({ title: t('hist.clearTitle'), message: t('hist.clearText'), confirmLabel: t('hist.clear'), destructive: true });
    if (!ok) return;
    await watch.clear();
    toast(t('hist.cleared'), 'trash-2');
    render();
  } });
  root.append(pageHead({ title: t('nav.mylist'), subtitle: t('mylist.subtitle'), actions: clear }), content, h('div.page-foot'));

  async function render() {
    let continuing = [];
    let history = [];
    try {
      [continuing, history] = await Promise.all([watch.continue(), watch.history()]);
    } catch { /* base indisponible : on affiche ce qu'on a */ }
    if (destroyed) return;
    const saved = listItems().map((e) => ({ ...e, key: `${e.type}:${e.id}` }));
    clear.hidden = !history.length;
    if (!continuing.length && !saved.length && !history.length) {
      content.replaceChildren(emptyState({
        icon: 'bookmark',
        title: t('mylist.emptyTitle'),
        text: t('mylist.emptyText'),
        actions: [button(t('nav.movies'), { icon: 'clapperboard', kind: 'primary', onClick: () => navigate('watch') }), button(t('nav.anime'), { icon: 'tv-minimal', onClick: () => navigate('anime') })],
      }));
      return;
    }
    const parts = [];
    if (continuing.length) {
      parts.push(shelf({
        title: t('shelf.continue'),
        items: continuing,
        build: (entry) => resumeCard(entry, {
          onPlay: () => navigate('watching', watchParam(entry.type, entry.id, entry.season, entry.episode)),
          onRemove: async () => { await watch.remove(entry.type, entry.id); render(); },
        }),
      }));
    }
    if (saved.length) {
      parts.push(h('section.block', {}, h('h2.block-title', { text: t('shelf.myList') }), h('div.grid.posters', {}, ...saved.map((e) => posterCard(e, { showKind: true })))));
    }
    if (history.length) {
      parts.push(h('section.block', {}, h('h2.block-title', { text: t('hist.title') }),
        h('div.hist-list', {}, ...history.map((e) => historyRow(e, async () => { await watch.remove(e.type, e.id); render(); })))));
    }
    content.replaceChildren(...parts);
  }

  const off = onList(() => { if (!destroyed) render(); });
  loadList().catch(() => {}).finally(() => { if (!destroyed) render(); });
  return { destroy: () => { destroyed = true; off(); } };
}

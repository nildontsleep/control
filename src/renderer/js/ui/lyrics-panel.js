// Panneau de paroles de « À l'écoute » : défilement synchronisé, clic sur une ligne pour s'y rendre.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { player } from '../player.js';
import { parseLrc, lineAt } from '../lyrics/lrc.js';
import { openSheet } from './sheet.js';
import { toast } from './feedback.js';

const api = window.control.lyrics;

const SOURCE_KEYS = { lrclib: 'lyr.srcLrclib', file: 'lyr.srcFile', tags: 'lyr.srcTags', manual: 'lyr.srcManual' };

export function createLyricsPanel(scroller) {
  const cache = new Map(); // id → résultat du processus principal
  const root = h('div.ly');
  let trackId = null;
  let lines = [];
  let synced = false;
  let nodes = [];
  let active = -2;
  let manualUntil = 0;
  let token = 0;

  // Défiler à la main met la lecture automatique en pause quelques secondes.
  for (const ev of ['wheel', 'touchmove', 'pointerdown']) scroller.addEventListener(ev, () => { manualUntil = performance.now() + 3500; }, { passive: true });

  function actions(result) {
    const btn = (ic, tip, fn) => h('button.icon-btn', { type: 'button', 'data-tip': tip, html: icon(ic, 16, 2), onclick: fn });
    return h('div.ly-head', {},
      h('span.ly-source', { text: result?.source && SOURCE_KEYS[result.source] ? t(SOURCE_KEYS[result.source]) : '' }),
      h('span.spacer'),
      result && (result.synced || result.plain) ? btn('download', t('lyr.saveFile'), saveFile) : null,
      btn('square-pen', t('lyr.edit'), () => edit(result)),
      btn('refresh-cw', t('lyr.refresh'), () => load(player.track, true)));
  }

  async function saveFile() {
    const res = await api.saveFile(trackId);
    toast(res.ok ? t('lyr.saved') : res.message, res.ok ? 'file-audio' : 'triangle-alert');
  }

  function edit(result) {
    const area = h('textarea.text-input.ly-edit', { rows: '14', spellcheck: 'false', placeholder: t('lyr.placeholder') });
    area.value = result?.synced || result?.plain || '';
    const save = h('button.btn.btn-primary', { type: 'button', text: t('common.save') });
    const cancel = h('button.btn.btn-secondary', { type: 'button', text: t('common.cancel') });
    const id = trackId;
    const sheet = openSheet({ title: t('lyr.editTitle'), width: 560, body: [h('p.sheet-text', { text: t('lyr.editHint') }), area], footer: [cancel, save] });
    cancel.addEventListener('click', () => sheet.close());
    save.addEventListener('click', async () => {
      const res = await api.set(id, area.value);
      sheet.close();
      if (res.ok) {
        cache.set(id, res.data);
        if (trackId === id) paint(res.data);
      }
    });
    setTimeout(() => area.focus(), 60);
  }

  function status(icName, title, text, extra) {
    return h('div.ly-status', {}, h('span.ly-status-icon', { html: icon(icName, 28, 1.5) }), h('strong', { text: title }), text ? h('p', { text }) : null, extra || null);
  }

  function paint(result) {
    active = -2;
    nodes = [];
    lines = [];
    synced = false;
    if (result?.status === 'ok' && result.instrumental && !result.synced && !result.plain) {
      root.replaceChildren(actions(result), status('audio-lines', t('lyr.instrumental'), null));
      return;
    }
    const raw = result?.synced || result?.plain;
    if (!raw) {
      root.replaceChildren(
        actions(result),
        status('mic-vocal', t('lyr.noneTitle'), t('lyr.noneText'),
          h('div.ly-status-actions', {},
            h('button.btn.btn-secondary.btn-small', { type: 'button', onclick: () => load(player.track, true) }, h('span.btn-icon', { html: icon('refresh-cw', 13, 2.2) }), h('span', { text: t('lyr.refresh') })),
            h('button.btn.btn-secondary.btn-small', { type: 'button', onclick: () => edit(result) }, h('span.btn-icon', { html: icon('clipboard-paste', 13, 2.2) }), h('span', { text: t('lyr.paste') })))));
      return;
    }
    const parsed = parseLrc(raw);
    lines = parsed.lines;
    synced = parsed.synced;
    const body = h(`div.ly-body${synced ? '.synced' : ''}`);
    nodes = lines.map((line, i) => {
      let el;
      if (!synced) el = h(line.text ? 'div.ly-line.plain' : 'div.ly-line.plain.blank', { text: line.text });
      else if (line.text) el = h('button.ly-line', { type: 'button', text: line.text });
      else el = h('div.ly-line.gap', { html: '<i></i><i></i><i></i>' });
      if (synced && line.text) el.addEventListener('click', () => { player.seek(line.t); player.play(); });
      el.dataset.i = String(i);
      return el;
    });
    body.append(...(synced ? [h('div.ly-spacer'), ...nodes, h('div.ly-spacer')] : [h('div.ly-pad'), ...nodes, h('div.ly-pad')]));
    root.replaceChildren(actions(result), body);
    scroller.scrollTop = 0;
    if (synced) tick(player.time, true);
  }

  async function load(track, refresh = false) {
    const mine = ++token;
    if (!track) {
      trackId = null;
      root.replaceChildren(status('mic-vocal', t('lyr.noTrack'), null));
      return;
    }
    trackId = track.id;
    if (!refresh && cache.has(track.id)) {
      paint(cache.get(track.id));
      return;
    }
    root.replaceChildren(h('div.ly-loading', {}, h('span.spinner', {}, ...Array.from({ length: 12 }, (_, i) => h('i', { style: { '--i': i } }))), h('span', { text: t('lyr.searching') })));
    const res = await api.get(track.id, refresh);
    if (mine !== token) return;
    if (!res.ok) {
      root.replaceChildren(
        actions(null),
        status('triangle-alert', t('lyr.errorTitle'), res.code === 'busy' ? t('lyr.errorBusy') : res.code === 'network' ? t('lyr.errorNetwork') : res.message,
          h('div.ly-status-actions', {}, h('button.btn.btn-secondary.btn-small', { type: 'button', onclick: () => load(player.track, true) }, h('span', { text: t('action.retry') })))));
      return;
    }
    cache.set(track.id, res.data);
    paint(res.data);
  }

  /** Met la ligne courante en valeur et la ramène au centre. */
  function tick(time, force = false) {
    if (!synced || !nodes.length) return;
    const i = lineAt(lines, time + 0.25);
    if (i === active && !force) return;
    const previous = active;
    active = i;
    nodes.forEach((el, k) => {
      el.classList.toggle('active', k === i);
      el.classList.toggle('past', k < i);
      el.classList.toggle('near', Math.abs(k - i) === 1);
    });
    if (i < 0 || performance.now() < manualUntil) return;
    const el = nodes[i];
    const target = el.offsetTop - scroller.clientHeight * 0.34 + el.offsetHeight / 2;
    scroller.scrollTo({ top: Math.max(0, target), behavior: force || Math.abs(i - previous) > 4 ? 'auto' : 'smooth' });
  }

  return {
    el: root,
    show: (track) => load(track),
    tick,
    invalidate: (id) => cache.delete(id),
  };
}

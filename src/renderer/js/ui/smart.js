// Éditeur de playlists intelligentes : règles, tri, limite, aperçu en direct.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { SMART_FIELDS, SMART_OPS, SMART_PRESETS, evalSmart, loadPlaylists } from '../data.js';
import { navigate } from '../router.js';
import { openSheet } from './sheet.js';
import { popup, button, createScope, withScope, refreshScope } from './controls.js';
import { toast } from './feedback.js';

const api = window.control.playlists;
const clone = (v) => JSON.parse(JSON.stringify(v));

export function openSmartEditor(playlist = null, { preset = 'mostPlayed', name = '' } = {}) {
  const spec = clone(playlist?.rules || SMART_PRESETS[preset] || SMART_PRESETS.mostPlayed);
  let title = playlist?.name || name || t('pl.newSmart');
  const rulesBox = h('div.rules');
  const count = h('span.rules-count');
  const nameInput = h('input.text-input', { type: 'text', value: title, placeholder: t('pl.namePlaceholder'), spellcheck: 'false' });
  nameInput.addEventListener('input', () => { title = nameInput.value; });

  const fieldOf = (id) => SMART_FIELDS.find((f) => f.id === id) || SMART_FIELDS[0];
  const opLabel = (op) => t(`op.${op}`);

  function preview() {
    const n = evalSmart({ rules: spec }).length;
    count.textContent = t('smart.matches', { n });
  }

  function ruleRow(rule, index) {
    const field = fieldOf(rule.field);
    const fieldPopup = popup({
      get: () => rule.field,
      set: (v) => {
        rule.field = v;
        const next = fieldOf(v);
        rule.op = SMART_OPS[next.type][0];
        rule.value = next.type === 'text' ? '' : next.type === 'date' ? 30 : 0;
        render();
      },
    }, SMART_FIELDS.map((f) => ({ value: f.id, label: t(`field.${f.id}`) })));
    const opPopup = popup({ get: () => rule.op, set: (v) => { rule.op = v; render(); } }, SMART_OPS[field.type].map((op) => ({ value: op, label: opLabel(op) })));
    const input = h('input.text-input.small', { type: field.type === 'text' ? 'text' : 'number', value: String(rule.value ?? ''), min: '0', spellcheck: 'false' });
    input.addEventListener('input', () => {
      rule.value = field.type === 'text' ? input.value : Number(input.value);
      preview();
    });
    const suffix = field.type === 'date' ? h('span.rule-suffix', { text: t('smart.days') }) : field.id === 'duration' ? h('span.rule-suffix', { text: t('smart.minutes') }) : null;
    return h('div.rule', {}, fieldPopup, opPopup, input, suffix,
      h('button.icon-btn', { type: 'button', 'data-tip': t('common.delete'), html: icon('x', 14, 2), onclick: () => { spec.rules.splice(index, 1); render(); } }));
  }

  function render() {
    rulesBox.replaceChildren(...spec.rules.map(ruleRow),
      button(t('smart.addRule'), { icon: 'plus', small: true, onClick: () => { spec.rules.push({ field: 'genre', op: 'contains', value: '' }); render(); } }));
    preview();
  }

  const fixed = createScope();
  const matchPopup = withScope(fixed, () => popup({ get: () => spec.match, set: (v) => { spec.match = v; preview(); refreshScope(fixed); } }, [
    { value: 'all', label: t('smart.matchAll') },
    { value: 'any', label: t('smart.matchAny') },
  ]));
  const sortKeys = ['title', 'artist', 'playCount', 'addedAt', 'lastPlayed', 'year', 'random'];
  const sortPopup = withScope(fixed, () => popup({ get: () => spec.sort.key, set: (v) => { spec.sort.key = v; spec.sort.dir = v === 'title' || v === 'artist' ? 'asc' : 'desc'; refreshScope(fixed); } },
    sortKeys.map((k) => ({ value: k, label: t(`sort.${k}`) }))));
  const limitInput = h('input.text-input.small', { type: 'number', min: '0', value: String(spec.limit || 0), 'data-tip': t('smart.limitHint') });
  limitInput.addEventListener('input', () => { spec.limit = Math.max(0, Number(limitInput.value) || 0); preview(); });

  const presets = h('div.preset-chips', {}, ...Object.keys(SMART_PRESETS).map((key) => h('button.chip', {
    type: 'button',
    text: t(`preset.${key}`),
    onclick: () => {
      Object.assign(spec, clone(SMART_PRESETS[key]));
      limitInput.value = String(spec.limit || 0);
      if (!playlist && (!title || title === t('pl.newSmart') || Object.keys(SMART_PRESETS).some((k) => title === t(`preset.${k}`)))) {
        title = t(`preset.${key}`);
        nameInput.value = title;
      }
      refreshScope(fixed);
      render();
    },
  })));

  const save = async () => {
    const finalName = (title || '').trim() || t('pl.newSmart');
    spec.rules = spec.rules.filter((r) => r.field);
    if (playlist) {
      await api.rules(playlist.id, spec);
      if (finalName !== playlist.name) await api.rename(playlist.id, finalName);
      toast(t('toast.saved'), 'check');
    } else {
      const id = await api.create(finalName, 'smart', spec);
      await loadPlaylists();
      navigate('playlist', id);
    }
    handle.close();
  };

  const handle = openSheet({
    title: playlist ? t('pl.editRules') : t('pl.newSmart'),
    width: 600,
    className: 'smart-sheet',
    body: [
      h('label.field', {}, h('span', { text: t('pl.name') }), nameInput),
      h('div.field', {}, h('span', { text: t('smart.presets') }), presets),
      h('div.field', {}, h('span', { text: t('smart.rules') }), h('div.rules-head', {}, matchPopup), rulesBox),
      h('div.field.inline', {}, h('span', { text: t('smart.sortBy') }), sortPopup, h('span', { text: t('smart.limit') }), limitInput),
    ],
    footer: [count, h('span.spacer'), button(t('common.cancel'), { onClick: () => handle.close() }), button(playlist ? t('common.save') : t('common.create'), { kind: 'primary', onClick: save })],
  });
  render();
  setTimeout(() => nameInput.focus(), 80);
}

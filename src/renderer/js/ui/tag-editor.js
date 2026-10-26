// Éditeur de tags façon mp3tag : un ou plusieurs morceaux, pochette, outils par lot, renommage des fichiers.
// Rien n'est écrit tant qu'on n'a pas cliqué sur « Enregistrer » ; le dernier lot peut ensuite être annulé.

import { can } from '../web.js';
import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { fmtTime, fmtBytes } from '../format.js';
import { lib } from '../data.js';
import { player } from '../player.js';
import { openSheet } from './sheet.js';
import { openMenu } from './menu.js';
import { toast } from './feedback.js';
import { coverEl, coverUrl } from './cover.js';
import { createScope, withScope, refreshScope, popup } from './controls.js';
import {
  PATTERN_PRESETS, renderPattern, compilePattern, parsePattern, CASES, tidy, replaceText,
} from '../tags/pattern.js';

const api = window.control.tags;

async function call(promise) {
  const res = await promise;
  if (!res.ok) throw new Error(res.message);
  return res.data;
}

const TEXT_KEYS = ['title', 'artist', 'albumArtist', 'album', 'genre', 'composer', 'comment'];
const NUMBER_KEYS = ['year', 'trackNo', 'trackTotal', 'discNo', 'discTotal'];

const display = (value) => (value === 0 || value === null || value === undefined ? '' : String(value));
const baseName = (file) => file.replace(/^.*[\\/]/, '').replace(/\.[^.]+$/, '');
const extOf = (file) => (/\.[^.\\/]+$/.exec(file) || [''])[0];

// Les motifs choisis restent d'une ouverture à l'autre.
const remembered = { filename: '%track% - %title%', rename: '%track% - %title%' };
let lastIds = [];

export async function openTagEditor(tracks) {
  if (!can('editFiles')) return;
  const unique = [...new Map(tracks.map((x) => [x.id, x])).values()];
  if (!unique.length) return;

  const body = h('div.tg-wrap');
  const status = h('div.tg-status');
  const cancelBtn = h('button.btn.btn-secondary', { type: 'button', text: t('common.cancel') });
  const saveBtn = h('button.btn.btn-primary', { type: 'button', disabled: true, text: t('tag.save') });
  let confirmingDiscard = false;
  let dirtyNow = () => false;
  let paintFooter = () => {};

  const sheet = openSheet({
    title: t('tag.sheetTitle'),
    width: 980,
    className: 'tag-sheet',
    body,
    footer: [status, cancelBtn, saveBtn],
    canClose: () => {
      if (!dirtyNow()) return true;
      confirmingDiscard = true;
      paintFooter();
      return false;
    },
  });
  body.append(h('div.tg-loading', {}, h('span.spinner', {}, ...Array.from({ length: 12 }, (_, i) => h('i', { style: { '--i': i } }))), h('span', { text: t('tag.reading') })));

  let loaded;
  try {
    loaded = await call(api.read(unique.map((x) => x.id)));
  } catch (err) {
    body.replaceChildren(h('p.tg-error', { text: `${t('tag.readError')} ${err.message}` }));
    return;
  }

  const rows = loaded.filter((r) => !r.error).map((r) => ({
    id: r.id,
    track: lib.byId.get(r.id) || unique.find((x) => x.id === r.id),
    path: r.path,
    base: { ...r.fields },
    edits: {},
    cover: null, // { token, preview } | 'remove'
    rename: null,
    error: null,
  }));
  const skipped = loaded.filter((r) => r.error);

  if (!rows.length) {
    body.replaceChildren(h('div.tg-none', {}, h('span.tg-none-icon', { html: icon('tags', 28, 1.6) }), h('p', { text: t('tag.nothingEditable') }), h('p.dim', { text: skipped[0]?.error?.message || '' })));
    saveBtn.hidden = true;
    return;
  }

  // ─── Modèle ────────────────────────────────────────────────────────────────
  const state = { scope: rows.length > 1 ? 'all' : 0, tool: null, saving: false };
  const scopeRows = () => (state.scope === 'all' ? rows : [rows[state.scope]]);
  const valueOf = (row, key) => (key in row.edits ? row.edits[key] : display(row.base[key]));
  const isDirty = (row) => Object.keys(row.edits).length > 0 || !!row.cover || !!row.rename;
  const dirtyRows = () => rows.filter(isDirty);
  dirtyNow = () => !state.saving && dirtyRows().length > 0;

  function setField(row, key, value) {
    const same = value === display(row.base[key]);
    if (same) delete row.edits[key];
    else row.edits[key] = value;
  }

  const fieldsOf = (row) => {
    const out = {};
    for (const k of TEXT_KEYS) out[k] = valueOf(row, k);
    for (const k of NUMBER_KEYS) out[k] = Number(valueOf(row, k)) || 0;
    return out;
  };

  const titleOf = (row) => valueOf(row, 'title') || baseName(row.path);

  // ─── Pochette ──────────────────────────────────────────────────────────────
  const coverBox = h('div.tg-cover');
  const coverState = () => {
    const keys = new Set();
    let first = null;
    for (const row of scopeRows()) {
      let key;
      let src = null;
      if (row.cover === 'remove') key = 'none';
      else if (row.cover) { key = `t:${row.cover.token}`; src = row.cover.preview; } else {
        key = row.track?.cover || 'none';
        src = row.track?.cover ? coverUrl(row.track.cover) : null;
      }
      keys.add(key);
      first ??= { src, row };
    }
    return { mixed: keys.size > 1, src: first?.src || null, row: first?.row };
  };

  function paintCover() {
    const { mixed, src, row } = coverState();
    coverBox.replaceChildren();
    coverBox.classList.toggle('mixed', mixed);
    coverBox.classList.toggle('empty', !src);
    if (src) {
      coverBox.append(h('img', { src, alt: '', draggable: 'false' }));
    } else {
      const seed = valueOf(row, 'album') || valueOf(row, 'title');
      coverBox.append(coverEl({ album: seed, title: seed }, { className: 'tg-cover-ph' }));
    }
    if (mixed) coverBox.append(h('span.tg-cover-badge', { text: t('tag.coverMixed') }));
    else if (!src) coverBox.append(h('span.tg-cover-badge', { text: t('tag.coverNone') }));
    coverBox.append(h('span.tg-cover-drop', {}, h('span', { html: icon('image-plus', 22, 1.7) }), h('span', { text: t('tag.coverDrop') })));
  }

  const applyCover = (value) => { for (const row of scopeRows()) row.cover = value; paintAll(); };

  async function stage(src) {
    coverBox.classList.add('busy');
    try {
      const staged = await call(api.stageCover(src));
      if (staged) applyCover({ token: staged.token, preview: staged.preview });
    } catch (err) {
      toast(err.message, 'triangle-alert');
    } finally {
      coverBox.classList.remove('busy');
    }
  }

  async function pickCover() {
    coverBox.classList.add('busy');
    try {
      const staged = await call(api.pickCover());
      if (staged) applyCover({ token: staged.token, preview: staged.preview });
    } catch (err) {
      toast(err.message, 'triangle-alert');
    } finally {
      coverBox.classList.remove('busy');
    }
  }

  for (const ev of ['dragenter', 'dragover']) {
    coverBox.addEventListener(ev, (e) => {
      if (![...(e.dataTransfer?.types || [])].includes('Files')) return;
      e.preventDefault();
      coverBox.classList.add('over');
    });
  }
  coverBox.addEventListener('dragleave', () => coverBox.classList.remove('over'));
  coverBox.addEventListener('drop', (e) => {
    e.preventDefault();
    coverBox.classList.remove('over');
    const file = e.dataTransfer?.files?.[0];
    if (!file) return;
    const p = window.control.pathForFile(file);
    if (p) stage({ path: p });
  });
  coverBox.addEventListener('click', pickCover);

  const coverActions = h('div.tg-cover-actions', {},
    h('button.btn.btn-secondary.btn-small', { type: 'button', onclick: pickCover }, h('span', { text: t('tag.coverChoose') })),
    h('button.btn.btn-secondary.btn-small', { type: 'button', onclick: () => openTool('search') }, h('span.btn-icon', { html: icon('search', 13, 2.1) }), h('span', { text: t('tag.coverSearchShort') })),
    h('button.btn.btn-secondary.btn-small', { type: 'button', 'data-tip': t('tag.coverRemove'), onclick: () => applyCover('remove') }, h('span.btn-icon', { html: icon('trash-2', 13, 2.1) })));

  // ─── Informations sur le fichier ───────────────────────────────────────────
  const info = h('dl.tg-info');
  function paintInfo() {
    const list = scopeRows();
    const lines = [];
    if (list.length === 1) {
      const row = list[0];
      const tr = row.track || {};
      lines.push([t('tag.info.file'), row.rename ? `${baseName(row.path)}${extOf(row.path)}  →  ${row.rename}${extOf(row.path)}` : `${baseName(row.path)}${extOf(row.path)}`, row.path]);
      lines.push([t('tag.info.format'), [tr.codec, tr.bitrate ? `${tr.bitrate} kb/s` : null].filter(Boolean).join(' · ') || '—']);
      lines.push([t('tag.info.duration'), fmtTime(tr.duration || 0)]);
      lines.push([t('tag.info.size'), fmtBytes(tr.size || 0)]);
    } else {
      const total = list.reduce((s, r) => s + (r.track?.duration || 0), 0);
      const size = list.reduce((s, r) => s + (r.track?.size || 0), 0);
      lines.push([t('tag.info.selection'), t('count.tracks', { n: list.length })]);
      lines.push([t('tag.info.duration'), fmtTime(total)]);
      lines.push([t('tag.info.size'), fmtBytes(size)]);
    }
    info.replaceChildren(...lines.flatMap(([k, v, tip]) => [h('dt', { text: k }), h('dd', { text: v, 'data-tip': tip })]));
  }

  // ─── Formulaire ────────────────────────────────────────────────────────────
  const inputs = new Map(); // clé → { el, input }

  function field(key, label, { textarea = false, rows: lines = 2, numeric = false, className = '' } = {}) {
    const input = textarea
      ? h('textarea.text-input', { rows: String(lines), spellcheck: 'false', 'data-key': key })
      : h('input.text-input', { type: 'text', spellcheck: 'false', inputmode: numeric ? 'numeric' : null, autocomplete: 'off', 'data-key': key });
    const el = h(`label.tg-field${className ? `.${className}` : ''}`, {}, label ? h('span.tg-label', { text: label }) : null, input);
    input.addEventListener('input', () => {
      let v = input.value;
      if (numeric) {
        const cleaned = v.replace(/\D/g, '').slice(0, 4);
        if (cleaned !== v) input.value = v = cleaned;
      }
      for (const row of scopeRows()) setField(row, key, v);
      markDirty(key);
      paintFooter();
      paintInfo();
      if (key === 'title') paintScope();
    });
    inputs.set(key, { el, input });
    return el;
  }

  function markDirty(key) {
    const slot = inputs.get(key);
    slot?.el.classList.toggle('dirty', scopeRows().some((row) => key in row.edits));
  }

  function paintFields() {
    for (const [key, { input }] of inputs) {
      const values = new Set(scopeRows().map((row) => valueOf(row, key)));
      const mixed = values.size > 1;
      input.value = mixed ? '' : [...values][0] ?? '';
      input.placeholder = mixed ? (NUMBER_KEYS.includes(key) ? '…' : t('tag.mixed')) : '';
      input.classList.toggle('mixed', mixed);
      markDirty(key);
    }
  }

  const form = h('div.tg-form', {},
    field('title', t('tag.f.title')),
    h('div.tg-row', {}, field('artist', t('tag.f.artist')), field('albumArtist', t('tag.f.albumArtist'))),
    field('album', t('tag.f.album')),
    h('div.tg-row.tg-row-nums', {},
      field('genre', t('tag.f.genre')),
      field('year', t('tag.f.year'), { numeric: true, className: 'tg-num' }),
      h('div.tg-pair', {},
        h('span.tg-label', { text: t('tag.f.track') }),
        h('div.tg-pair-inputs', {}, field('trackNo', null, { numeric: true, className: 'tg-num' }), h('span.tg-of', { text: t('tag.of') }), field('trackTotal', null, { numeric: true, className: 'tg-num' }))),
      h('div.tg-pair', {},
        h('span.tg-label', { text: t('tag.f.disc') }),
        h('div.tg-pair-inputs', {}, field('discNo', null, { numeric: true, className: 'tg-num' }), h('span.tg-of', { text: t('tag.of') }), field('discTotal', null, { numeric: true, className: 'tg-num' })))),
    field('composer', t('tag.f.composer')),
    field('comment', t('tag.f.comment'), { textarea: true, rows: 2 }),
    field('lyrics', t('tag.f.lyrics'), { textarea: true, rows: 4 }));

  // ─── Sélection (tous / un morceau) ─────────────────────────────────────────
  const scopeBtn = h('button.popup.tg-scope-btn', { type: 'button' }, h('span.popup-label'), h('span.popup-chev', { html: icon('chevrons-up-down', 12, 2.4) }));
  const prevBtn = h('button.icon-btn', { type: 'button', html: icon('chevron-left', 16, 2.2), 'data-tip': t('tag.prev') });
  const nextBtn = h('button.icon-btn', { type: 'button', html: icon('chevron-right', 16, 2.2), 'data-tip': t('tag.next') });
  const scopeBar = h('div.tg-scope', {}, prevBtn, scopeBtn, nextBtn);

  const scopeLabel = (s) => (s === 'all' ? t('tag.all', { n: rows.length }) : `${s + 1}. ${titleOf(rows[s])}`);
  function paintScope() {
    if (rows.length < 2) return;
    scopeBtn.firstChild.textContent = scopeLabel(state.scope);
    prevBtn.disabled = state.scope === 'all' || state.scope === 0;
    nextBtn.disabled = state.scope === 'all' || state.scope === rows.length - 1;
    scopeBtn.classList.toggle('single', state.scope !== 'all');
  }
  function setScope(s) {
    state.scope = s;
    paintAll();
  }
  scopeBtn.addEventListener('click', () => {
    openMenu(scopeBtn, [
      { label: scopeLabel('all'), checked: state.scope === 'all', icon: 'layers', action: () => setScope('all') },
      { separator: true },
      ...rows.map((row, i) => ({ label: scopeLabel(i), checked: state.scope === i, action: () => setScope(i) })),
    ], { align: 'start', minWidth: 320 });
  });
  prevBtn.addEventListener('click', () => state.scope !== 'all' && state.scope > 0 && setScope(state.scope - 1));
  nextBtn.addEventListener('click', () => state.scope !== 'all' && state.scope < rows.length - 1 && setScope(state.scope + 1));

  // ─── Outils ────────────────────────────────────────────────────────────────
  const TOOLS = [
    ['filename', 'scan-text'], ['rename', 'file-pen'], ['number', 'list-ordered'], ['replace', 'replace'], ['case', 'case-sensitive'],
  ];
  const toolsBar = h('div.tg-tools', {},
    h('span.tg-tools-title', { text: t('tag.tools') }),
    ...TOOLS.map(([id, ic]) => h('button.btn.btn-secondary.btn-small', { type: 'button', onclick: () => openTool(id) }, h('span.btn-icon', { html: icon(ic, 14, 2) }), h('span', { text: t(`tag.tool.${id}`) }))),
    h('button.btn.btn-secondary.btn-small', { type: 'button', onclick: tidyAll }, h('span.btn-icon', { html: icon('wand-sparkles', 14, 2) }), h('span', { text: t('tag.tool.tidy') })));

  function tidyAll() {
    let n = 0;
    for (const row of scopeRows()) {
      for (const key of TEXT_KEYS) {
        const before = valueOf(row, key);
        const after = tidy(before);
        if (after !== before) { setField(row, key, after); n++; }
      }
    }
    toast(n ? t('tag.tidyDone', { n }) : t('tag.tidyNone'), 'wand-sparkles');
    paintAll();
  }

  const panel = h('div.tg-panel');
  const main = h('div.tg-main', {}, form, toolsBar);

  function openTool(id) {
    state.tool = id;
    panel.replaceChildren(buildTool(id));
    main.replaceChildren(panel);
    panel.classList.remove('in');
    void panel.offsetWidth;
    panel.classList.add('in');
  }
  function closeTool() {
    state.tool = null;
    main.replaceChildren(form, toolsBar);
    paintAll();
  }

  const toolHead = (title, hint) => h('header.tg-tool-head', {},
    h('button.btn.btn-secondary.btn-small', { type: 'button', onclick: closeTool }, h('span.btn-icon', { html: icon('arrow-left', 14, 2.2) }), h('span', { text: t('tag.back') })),
    h('div', {}, h('h3', { text: title }), hint ? h('p', { text: hint }) : null));

  function table(head, data, { limit = 300 } = {}) {
    if (!data.length) return h('p.tg-none-text', { text: t('tag.nothing') });
    const el = h('div.tg-table', {}, h('div.tg-tr.head', {}, ...head.map((x) => h('span', { text: x }))));
    for (const cells of data.slice(0, limit)) {
      el.append(h(cells.dim ? 'div.tg-tr.dim' : 'div.tg-tr', {}, ...cells.values.map((x) => h('span', { text: x, title: x }))));
    }
    if (data.length > limit) el.append(h('div.tg-tr.more', {}, h('span', { text: `+ ${data.length - limit}` })));
    return el;
  }

  const scopeVars = createScope();
  const patternControls = (key, onChange) => {
    const input = h('input.text-input.mono', { type: 'text', value: remembered[key], spellcheck: 'false' });
    input.addEventListener('input', () => { remembered[key] = input.value; onChange(); });
    const presets = h('button.btn.btn-secondary.btn-small', { type: 'button', onclick: (e) => {
      openMenu(e.currentTarget, PATTERN_PRESETS.map((p) => ({
        label: p,
        checked: p === input.value,
        action: () => { input.value = p; remembered[key] = p; onChange(); },
      })), { align: 'end' });
    } }, h('span', { text: t('tag.presets') }), h('span', { html: icon('chevron-down', 13, 2.2) }));
    return { input, row: h('div.tg-pattern', {}, input, presets) };
  };

  function buildTool(id) {
    scopeVars.clear();
    const targets = scopeRows();
    const preview = h('div.tg-preview');
    const summary = h('p.tg-summary');
    const applyBtn = h('button.btn.btn-primary', { type: 'button', text: t('tag.apply') });
    const done = () => { closeTool(); };

    if (id === 'filename') {
      let plan = [];
      const run = () => {
        const compiled = compilePattern(remembered.filename);
        plan = targets.map((row) => ({ row, parsed: parsePattern(compiled, baseName(row.path)) }));
        const ok = plan.filter((p) => p.parsed);
        summary.textContent = t('tag.matches', { ok: ok.length, n: plan.length });
        applyBtn.disabled = !ok.length;
        preview.replaceChildren(table([t('tag.info.file'), t('tag.result')], plan.map((p) => ({
          dim: !p.parsed,
          values: [baseName(p.row.path), p.parsed ? Object.entries(p.parsed).map(([k, v]) => `${t(`tag.short.${k}`)}: ${v}`).join('  ·  ') : t('tag.noMatch')],
        }))));
      };
      const { row } = patternControls('filename', run);
      applyBtn.onclick = () => {
        for (const { row: r, parsed } of plan) if (parsed) for (const [k, v] of Object.entries(parsed)) setField(r, k, String(v));
        done();
      };
      run();
      return h('div.tg-tool', {}, toolHead(t('tag.tool.filename'), t('tag.patternHint')), row, summary, preview, h('footer.tg-tool-foot', {}, applyBtn));
    }

    if (id === 'rename') {
      let plan = [];
      const run = () => {
        plan = targets.map((row) => {
          const next = renderPattern(remembered.rename, fieldsOf(row));
          return { row, next, current: baseName(row.path) };
        });
        const names = new Map();
        for (const p of plan) names.set(p.next.toLowerCase(), (names.get(p.next.toLowerCase()) || 0) + 1);
        const changed = plan.filter((p) => p.next && p.next !== p.current);
        summary.textContent = t('tag.renameCount', { n: changed.length });
        applyBtn.disabled = !changed.length;
        preview.replaceChildren(table([t('tag.current'), t('tag.newName')], plan.map((p) => ({
          dim: !p.next || p.next === p.current,
          values: [p.current + extOf(p.row.path), p.next ? p.next + extOf(p.row.path) + (names.get(p.next.toLowerCase()) > 1 ? ' ⚠' : '') : t('tag.emptyName')],
        }))));
      };
      const { row } = patternControls('rename', run);
      applyBtn.onclick = () => {
        for (const p of plan) p.row.rename = p.next && p.next !== p.current ? p.next : null;
        done();
      };
      run();
      return h('div.tg-tool', {}, toolHead(t('tag.tool.rename'), t('tag.renameHint')), row, summary, preview, h('footer.tg-tool-foot', {}, applyBtn));
    }

    if (id === 'number') {
      const opts = { start: 1, total: true };
      const start = h('input.text-input.tg-num', { type: 'number', min: '0', max: '999', value: '1' });
      const total = h('label.check-row', {}, h('input', { type: 'checkbox', checked: true }), h('span', { text: t('tag.setTotal') }));
      const run = () => {
        opts.start = Math.max(0, Math.min(999, parseInt(start.value, 10) || 0));
        opts.total = total.querySelector('input').checked;
        preview.replaceChildren(table([t('tag.f.title'), t('tag.f.track')], targets.map((row, i) => ({
          values: [titleOf(row), `${opts.start + i}${opts.total ? ` / ${targets.length + opts.start - 1}` : ''}`],
        }))));
        summary.textContent = t('count.tracks', { n: targets.length });
      };
      start.addEventListener('input', run);
      total.addEventListener('change', run);
      applyBtn.onclick = () => {
        targets.forEach((row, i) => {
          setField(row, 'trackNo', String(opts.start + i));
          if (opts.total) setField(row, 'trackTotal', String(targets.length + opts.start - 1));
        });
        done();
      };
      run();
      return h('div.tg-tool', {}, toolHead(t('tag.tool.number'), t('tag.numberHint')),
        h('div.tg-opts', {}, h('label.tg-field.tg-num', {}, h('span.tg-label', { text: t('tag.start') }), start), total),
        summary, preview, h('footer.tg-tool-foot', {}, applyBtn));
    }

    if (id === 'replace' || id === 'case') {
      const fieldOptions = [{ value: '*', label: t('tag.allText') }, ...TEXT_KEYS.map((k) => ({ value: k, label: t(`tag.f.${k}`) }))];
      const o = { field: 'title', find: '', replace: '', matchCase: false, regex: false, mode: 'title' };
      if (id === 'case') o.field = '*';
      let plan = [];
      const compute = (value) => (id === 'case' ? CASES[o.mode](value) : replaceText(value, o));
      const run = () => {
        const keys = o.field === '*' ? TEXT_KEYS : [o.field];
        plan = [];
        for (const row of targets) {
          for (const key of keys) {
            const before = valueOf(row, key);
            const after = before ? compute(before) : before;
            if (after !== before) plan.push({ row, key, before, after });
          }
        }
        summary.textContent = t('tag.changes', { n: plan.length });
        applyBtn.disabled = !plan.length;
        preview.replaceChildren(table([t('tag.current'), t('tag.result')], plan.map((p) => ({ values: [p.before, p.after] }))));
      };
      applyBtn.onclick = () => {
        for (const p of plan) setField(p.row, p.key, p.after);
        done();
      };
      const fieldPopup = withScope(scopeVars, () => popup({ get: () => o.field, set: (v) => { o.field = v; run(); refreshScope(scopeVars); } }, fieldOptions));
      let controls;
      if (id === 'replace') {
        const find = h('input.text-input', { type: 'text', placeholder: t('tag.find'), spellcheck: 'false' });
        const rep = h('input.text-input', { type: 'text', placeholder: t('tag.replaceWith'), spellcheck: 'false' });
        find.addEventListener('input', () => { o.find = find.value; run(); });
        rep.addEventListener('input', () => { o.replace = rep.value; run(); });
        const mc = h('label.check-row', {}, h('input', { type: 'checkbox', onchange: (e) => { o.matchCase = e.target.checked; run(); } }), h('span', { text: t('tag.matchCase') }));
        const rx = h('label.check-row', {}, h('input', { type: 'checkbox', onchange: (e) => { o.regex = e.target.checked; run(); } }), h('span', { text: t('tag.regex') }));
        controls = h('div.tg-opts.col', {}, h('div.tg-opts-row', {}, h('span.tg-label', { text: t('tag.field') }), fieldPopup), find, rep, h('div.tg-opts-row', {}, mc, rx));
        setTimeout(() => find.focus(), 40);
      } else {
        const modes = ['title', 'sentence', 'lower', 'upper'].map((m) => ({ value: m, label: t(`tag.case.${m}`) }));
        const modePopup = withScope(scopeVars, () => popup({ get: () => o.mode, set: (v) => { o.mode = v; run(); refreshScope(scopeVars); } }, modes));
        controls = h('div.tg-opts', {}, h('div.tg-opts-row', {}, h('span.tg-label', { text: t('tag.field') }), fieldPopup), h('div.tg-opts-row', {}, h('span.tg-label', { text: t('tag.tool.case') }), modePopup));
      }
      run();
      return h('div.tg-tool', {}, toolHead(t(`tag.tool.${id}`), id === 'replace' ? t('tag.replaceHint') : t('tag.caseHint')), controls, summary, preview, h('footer.tg-tool-foot', {}, applyBtn));
    }

    if (id === 'search') {
      const first = targets[0];
      const artist = h('input.text-input', { type: 'text', value: valueOf(first, 'albumArtist') || valueOf(first, 'artist'), placeholder: t('tag.f.artist'), spellcheck: 'false' });
      const album = h('input.text-input', { type: 'text', value: valueOf(first, 'album'), placeholder: t('tag.f.album'), spellcheck: 'false' });
      const grid = h('div.tg-results');
      const go = async () => {
        grid.replaceChildren(h('div.tg-loading.small', {}, h('span.spinner.small', {}, ...Array.from({ length: 12 }, (_, i) => h('i', { style: { '--i': i } }))), h('span', { text: t('tag.searching') })));
        try {
          const list = await call(api.searchCovers({ artist: artist.value, album: album.value }));
          if (state.tool !== 'search') return;
          if (!list.length) {
            grid.replaceChildren(h('p.tg-none-text', { text: t('tag.searchEmpty') }));
            return;
          }
          grid.replaceChildren(...list.map((r) => {
            const card = h('button.tg-result', { type: 'button', title: `${r.title} — ${r.artist}` },
              h('span.tg-result-art', {}, h('img', { src: r.preview, alt: '', loading: 'lazy', draggable: 'false' })),
              h('span.tg-result-title', { text: r.title }),
              h('span.tg-result-sub', { text: [r.artist, r.year, r.source].filter(Boolean).join(' · ') }));
            card.addEventListener('click', async () => {
              card.classList.add('busy');
              await stage({ url: r.full });
              card.classList.remove('busy');
              if (state.tool === 'search') closeTool();
            });
            return card;
          }));
        } catch {
          grid.replaceChildren(h('p.tg-none-text', { text: t('tag.searchError') }));
        }
      };
      for (const input of [artist, album]) input.addEventListener('keydown', (e) => { if (e.key === 'Enter') go(); });
      const goBtn = h('button.btn.btn-primary', { type: 'button', onclick: go }, h('span.btn-icon', { html: icon('search', 14, 2.2) }), h('span', { text: t('tag.search') }));
      if (artist.value || album.value) setTimeout(go, 60);
      return h('div.tg-tool', {}, toolHead(t('tag.coverSearch'), t('tag.searchHint')), h('div.tg-pattern', {}, artist, album, goBtn), grid);
    }
    return h('div');
  }

  // ─── Enregistrement ────────────────────────────────────────────────────────
  paintFooter = () => {
    const n = dirtyRows().length;
    const failed = rows.filter((r) => r.error);
    if (confirmingDiscard) {
      status.replaceChildren(h('span.tg-warn', { text: t('tag.discardTitle') }));
      cancelBtn.textContent = t('tag.keepEditing');
      cancelBtn.onclick = () => { confirmingDiscard = false; paintFooter(); };
      saveBtn.textContent = t('tag.discard');
      saveBtn.classList.add('danger');
      saveBtn.disabled = false;
      saveBtn.onclick = () => { dirtyNow = () => false; sheet.close(); };
      return;
    }
    cancelBtn.textContent = t('common.cancel');
    cancelBtn.onclick = () => {
      if (dirtyNow()) {
        confirmingDiscard = true;
        paintFooter();
      } else sheet.close();
    };
    saveBtn.classList.remove('danger');
    saveBtn.onclick = save;
    const parts = [];
    parts.push(n ? t('tag.status', { n }) : t('tag.statusNone'));
    if (skipped.length) parts.push(t('tag.skipped', { n: skipped.length }));
    status.replaceChildren(h('span', { text: parts.join(' · ') }));
    if (failed.length) {
      status.append(h('span.tg-fail', { text: ` — ${t('tag.failed', { n: failed.length })} : ${failed[0].error}${failed.length > 1 ? '…' : ''}`, title: failed.map((r) => `${baseName(r.path)} : ${r.error}`).join('\n') }));
    }
    saveBtn.textContent = state.saving ? t('tag.saving') : n > 1 ? t('tag.saveN', { n }) : t('tag.save');
    saveBtn.disabled = state.saving || n === 0;
  };

  async function save() {
    const dirty = dirtyRows();
    if (!dirty.length || state.saving) return;
    state.saving = true;
    body.classList.add('saving');
    paintFooter();
    const ids = dirty.map((r) => r.id);
    const resume = player.suspend(ids);
    const edits = dirty.map((row) => ({
      id: row.id,
      set: { ...row.edits },
      cover: row.cover === 'remove' ? { remove: true } : row.cover ? { token: row.cover.token } : undefined,
      rename: row.rename || undefined,
    }));
    let outcome;
    try {
      outcome = await call(api.write(edits));
    } catch (err) {
      toast(err.message, 'triangle-alert');
    }
    resume();
    state.saving = false;
    body.classList.remove('saving');
    if (!outcome) { paintFooter(); return; }

    let ok = 0;
    for (const res of outcome.results) {
      const row = rows.find((r) => r.id === res.id);
      if (!row) continue;
      if (res.ok) {
        ok++;
        for (const [k, v] of Object.entries(row.edits)) row.base[k] = NUMBER_KEYS.includes(k) ? Number(v) || 0 : v;
        row.edits = {};
        row.cover = null;
        row.rename = null;
        row.path = res.path || row.path;
        row.error = null;
      } else {
        row.error = res.message;
      }
    }
    lastIds = outcome.results.filter((r) => r.ok).map((r) => r.id);
    const failedCount = outcome.results.length - ok;
    if (!failedCount) {
      dirtyNow = () => false;
      sheet.close();
    } else {
      paintAll();
    }
    if (ok) {
      toast(t('tag.saved', { n: ok }), 'tags', {
        action: outcome.undoable ? { label: t('tag.undo'), onClick: undo } : null,
      });
    }
  }

  // ─── Affichage ─────────────────────────────────────────────────────────────
  function paintAll() {
    paintScope();
    paintFields();
    paintCover();
    paintInfo();
    paintFooter();
  }

  saveBtn.onclick = save;
  body.replaceChildren(h('div.tg', {},
    h('aside.tg-left', {}, coverBox, coverActions, info),
    h('div.tg-right', {}, rows.length > 1 ? scopeBar : null, main)));
  paintAll();
  setTimeout(() => inputs.get('title')?.input.focus(), 80);

  // Ctrl+Entrée enregistre depuis n'importe quel champ.
  body.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      save();
    }
  });
}

/** Annule le dernier lot enregistré (fichiers remis dans l'état d'avant). */
async function undo() {
  const resume = player.suspend(lastIds);
  try {
    const { restored, failed } = await call(api.undo());
    toast(failed ? t('tag.undoPartial', { n: restored, f: failed }) : t('tag.undone', { n: restored }), 'undo-2');
  } catch (err) {
    toast(err.message, 'triangle-alert');
  } finally {
    resume();
  }
}

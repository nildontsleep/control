// Contrôles façon Réglages Système : interrupteurs, curseurs, segments, pop-ups, pastilles.

import { h, getPath, patchFor } from '../dom.js';
import { settings, update, DEFAULTS } from '../settings.js';
import { icon } from '../icons.js';
import { openMenu } from './menu.js';

// Chaque contrôle s’enregistre ici pour se resynchroniser quand les réglages changent.
let refreshers = new Set();

export function beginScope() {
  refreshers = new Set();
  return refreshers;
}

export const createScope = () => new Set();

/** Exécute fn en enregistrant les contrôles créés dans `scope` (pour les resynchroniser plus tard). */
export function withScope(scope, fn) {
  const previous = refreshers;
  refreshers = scope;
  try {
    return fn();
  } finally {
    refreshers = previous;
  }
}

export function refreshScope(scope = refreshers) {
  for (const fn of scope) fn();
}

function register(fn) {
  refreshers.add(fn);
  fn();
}

function binding(b) {
  if (typeof b === 'string') {
    return {
      get: () => getPath(settings, b),
      set: (v) => update(patchFor(b, v)),
      def: getPath(DEFAULTS, b),
    };
  }
  return b;
}

export function toggle(b, { onChange } = {}) {
  const bd = binding(b);
  const input = h('input', { type: 'checkbox', role: 'switch' });
  const el = h('label.switch', {}, input, h('span.switch-track', {}, h('span.switch-knob')));
  input.addEventListener('change', () => {
    bd.set(input.checked);
    onChange?.(input.checked);
  });
  register(() => {
    input.checked = !!bd.get();
  });
  return el;
}

export function slider(b, { min = 0, max = 100, step = 1, center = false, format, track } = {}) {
  const bd = binding(b);
  const input = h('input.slider', { type: 'range', min, max, step });
  if (center) input.classList.add('centered');
  if (track) {
    input.classList.add('custom-track');
    input.style.setProperty('--track', track);
  }
  const value = h('span.slider-value');
  const paint = () => {
    const v = Number(input.value);
    const pct = ((v - min) / (max - min)) * 100;
    const zero = center ? ((0 - min) / (max - min)) * 100 : 0;
    input.style.setProperty('--from', `${Math.min(pct, zero)}%`);
    input.style.setProperty('--to', `${Math.max(pct, zero)}%`);
    input.style.setProperty('--pct', `${pct}%`);
    value.textContent = format ? format(v) : String(Math.round(v));
    value.classList.toggle('dim', center ? v === 0 : false);
  };
  input.addEventListener('input', () => {
    paint();
    bd.set(Number(input.value));
  });
  input.addEventListener('dblclick', () => {
    if (bd.def === undefined) return;
    input.value = bd.def;
    paint();
    bd.set(Number(bd.def));
  });
  register(() => {
    const v = bd.get();
    if (v !== undefined && Number(input.value) !== Number(v)) input.value = v;
    paint();
  });
  return { input, value };
}

export function segmented(b, options, { compact = false, iconsOnly = false } = {}) {
  const bd = binding(b);
  const el = h('div.seg', { role: 'radiogroup', style: { '--n': options.length } });
  if (compact) el.classList.add('compact');
  if (iconsOnly) el.classList.add('icons-only');
  const thumb = h('span.seg-thumb');
  el.append(thumb);
  const buttons = options.map((opt) => {
    const btn = h('button.seg-btn', { type: 'button', role: 'radio', 'data-tip': iconsOnly ? opt.label : opt.tip },
      opt.icon ? h('span.seg-icon', { html: icon(opt.icon, 15) }) : null,
      !iconsOnly && opt.label ? h('span.seg-label', { text: opt.label }) : null);
    btn.addEventListener('click', () => bd.set(opt.value));
    el.append(btn);
    return btn;
  });
  register(() => {
    const v = bd.get();
    const i = options.findIndex((o) => String(o.value) === String(v));
    buttons.forEach((btn, j) => {
      btn.classList.toggle('on', j === i);
      btn.setAttribute('aria-checked', j === i ? 'true' : 'false');
    });
    el.style.setProperty('--i', Math.max(0, i));
    thumb.style.opacity = i < 0 ? '0' : '1';
  });
  return el;
}

export function popup(b, options, { minWidth } = {}) {
  const bd = binding(b);
  const list = () => (typeof options === 'function' ? options() : options);
  const label = h('span.popup-label');
  const btn = h('button.popup', { type: 'button' }, label, h('span.popup-chev', { html: icon('chevrons-up-down', 12, 2.4) }));
  btn.addEventListener('click', () => {
    const cur = String(bd.get());
    openMenu(btn, list().map((o) => ({
      label: o.label,
      checked: String(o.value) === cur,
      action: () => bd.set(o.value),
    })), { align: 'selected', minWidth });
  });
  register(() => {
    const items = list();
    const cur = items.find((o) => String(o.value) === String(bd.get())) || items[0];
    label.textContent = cur ? cur.label : '—';
  });
  return btn;
}

export function swatches(b, items, { custom } = {}) {
  const bd = binding(b);
  const el = h('div.swatches');
  const buttons = items.map((it) => {
    const btn = h('button.swatch', { type: 'button', 'data-tip': it.label, style: { '--c': it.color } });
    btn.addEventListener('click', () => bd.set(it.value));
    el.append(btn);
    return btn;
  });
  let customBtn = null;
  if (custom) {
    const cb = binding(custom.path);
    const input = h('input', { type: 'color' });
    customBtn = h('label.swatch.swatch-custom', { 'data-tip': custom.label }, input);
    input.addEventListener('input', () => {
      cb.set(input.value);
      bd.set('custom');
    });
    input.addEventListener('click', () => bd.set('custom'));
    el.append(customBtn);
    register(() => {
      input.value = cb.get() || '#7c5cff';
      customBtn.style.setProperty('--c', input.value);
    });
  }
  register(() => {
    const v = bd.get();
    buttons.forEach((btn, i) => btn.classList.toggle('on', items[i].value === v));
    customBtn?.classList.toggle('on', v === 'custom');
  });
  return el;
}

export function button(label, { icon: ic, onClick, kind = 'secondary', tip, small = false } = {}) {
  const btn = h(`button.btn.btn-${kind}`, { type: 'button', 'data-tip': tip },
    ic ? h('span.btn-icon', { html: icon(ic, 14, 2) }) : null,
    label ? h('span', { text: label }) : null);
  if (small) btn.classList.add('btn-small');
  if (onClick) btn.addEventListener('click', onClick);
  return btn;
}

export function badge(name, color) {
  return h('span.row-badge', { style: { '--c': color }, html: icon(name, 14, 2.1) });
}

export function row(label, control, { hint, badge: bdg, className } = {}) {
  const el = h('div.row', {},
    bdg || null,
    h('div.row-label', {}, h('span.row-title', { text: label }), hint ? h('span.row-hint', { text: hint }) : null),
    control ? h('div.row-control', {}, control) : null);
  if (className) el.classList.add(className);
  return el;
}

export function sliderRow(label, b, opts = {}) {
  const { input, value } = slider(b, opts);
  return h('div.row.row-slider', {},
    h('div.row-head', {}, h('span.row-title', { text: label }), value),
    input);
}

export function group(title, children, { footer, action } = {}) {
  return h('section.group', {},
    title || action ? h('header.group-head', {}, title ? h('h3', { text: title }) : null, action || null) : null,
    h('div.group-body', {}, ...children.filter(Boolean)),
    footer ? h('p.group-footer', { text: footer }) : null);
}

/** Un contrôle dont le contenu dépend d’une valeur dynamique (non liée aux réglages). */
export function live(fn) {
  register(fn);
}

// Menus contextuels façon macOS (vibrance, coche, navigation clavier).

import { h, clamp } from '../dom.js';
import { icon } from '../icons.js';

let current = null;

export function closeMenu() {
  current?.close();
}

export function isMenuOpen() {
  return !!current;
}

/**
 * items : [{ label, action, checked, icon, shortcut, disabled } | { separator: true } | { header }]
 * align : 'start' | 'end' | 'selected' (l’élément coché se place sur le bouton, comme un pop-up macOS)
 */
export function openMenu(anchor, items, { align = 'start', placement = 'below', minWidth } = {}) {
  closeMenu();
  const hasChecks = items.some((i) => 'checked' in i);
  const menu = h('div.menu', { role: 'menu' });
  if (hasChecks) menu.classList.add('has-checks');
  const buttons = [];

  for (const item of items) {
    if (item.separator) {
      menu.append(h('div.menu-sep'));
      continue;
    }
    if (item.header) {
      menu.append(h('div.menu-header', { text: item.header }));
      continue;
    }
    const el = h('button.menu-item', { type: 'button', role: 'menuitem', disabled: item.disabled },
      hasChecks ? h('span.menu-check', { html: item.checked ? icon('check', 13, 2.6) : '' }) : null,
      item.icon ? h('span.menu-icon', { html: icon(item.icon, 15) }) : null,
      item.swatch ? h('span.menu-swatch', { style: { background: item.swatch } }) : null,
      h('span.menu-label', { text: item.label }),
      item.shortcut ? h('span.menu-kbd', { text: item.shortcut }) : null);
    if (item.checked) el.classList.add('checked');
    el.addEventListener('click', () => {
      close();
      item.action?.();
    });
    el.addEventListener('pointermove', () => el.focus({ preventScroll: true }));
    buttons.push(el);
    menu.append(el);
  }

  document.body.append(menu);
  const r = anchor.getBoundingClientRect();
  if (minWidth || align === 'selected') menu.style.minWidth = `${Math.max(minWidth || 0, r.width + 28)}px`;
  const m = menu.getBoundingClientRect();
  const vw = window.innerWidth;
  const vh = window.innerHeight;

  let x = align === 'end' ? r.right - m.width : r.left;
  let y = placement === 'above' ? r.top - m.height - 6 : r.bottom + 6;
  if (align === 'selected') {
    const sel = menu.querySelector('.menu-item.checked') || buttons[0];
    if (sel) {
      y = r.top + r.height / 2 - (sel.offsetTop + sel.offsetHeight / 2);
      x = r.left - (hasChecks ? 16 : 6);
    }
  }
  if (y + m.height > vh - 8) y = placement === 'above' || align === 'selected' ? vh - 8 - m.height : r.top - m.height - 6;
  x = clamp(x, 8, vw - m.width - 8);
  y = clamp(y, 8, Math.max(8, vh - m.height - 8));
  menu.style.left = `${Math.round(x)}px`;
  menu.style.top = `${Math.round(y)}px`;
  menu.style.transformOrigin = `${clamp(r.left + r.width / 2 - x, 0, m.width)}px ${y > r.top ? 0 : m.height}px`;
  requestAnimationFrame(() => menu.classList.add('open'));
  anchor.classList.add('menu-anchor');

  const onDown = (e) => {
    if (!menu.contains(e.target) && !anchor.contains(e.target)) close();
  };
  const onKey = (e) => {
    const idx = buttons.indexOf(document.activeElement);
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      e.stopPropagation();
      const d = e.key === 'ArrowDown' ? 1 : -1;
      const next = buttons[(idx + d + buttons.length) % buttons.length];
      next?.focus({ preventScroll: true });
    } else if (e.key === 'Enter' || e.key === ' ') {
      if (idx >= 0) {
        e.preventDefault();
        e.stopPropagation();
        buttons[idx].click();
      }
    }
  };

  function close() {
    if (current?.menu !== menu) return;
    current = null;
    anchor.classList.remove('menu-anchor');
    document.removeEventListener('pointerdown', onDown, true);
    document.removeEventListener('keydown', onKey, true);
    window.removeEventListener('blur', close);
    menu.classList.remove('open');
    menu.classList.add('closing');
    setTimeout(() => menu.remove(), 160);
  }

  setTimeout(() => {
    document.addEventListener('pointerdown', onDown, true);
    window.addEventListener('blur', close);
  });
  document.addEventListener('keydown', onKey, true);
  current = { menu, close };
  (menu.querySelector('.menu-item.checked') || buttons[0])?.focus({ preventScroll: true });
  return close;
}

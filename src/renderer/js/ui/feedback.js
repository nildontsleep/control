// Notifications discrètes (toast) et info-bulles.

import { h, $ } from '../dom.js';
import { icon } from '../icons.js';

let toastEl = null;
let toastTimer = null;

/** opts : { action: { label, onClick }, duration } — avec une action, la notification reste cliquable plus longtemps. */
export function toast(text, iconName = null, { action = null, duration = null } = {}) {
  const host = $('#toasts');
  if (!host) return;
  if (!toastEl) {
    toastEl = h('div.toast');
    host.append(toastEl);
  }
  toastEl.classList.toggle('has-action', !!action);
  toastEl.replaceChildren(...[
    iconName ? h('span.toast-icon', { html: icon(iconName, 16, 2) }) : null,
    h('span.toast-text', { text }),
    action ? h('button.toast-action', {
      type: 'button',
      text: action.label,
      onclick: () => {
        clearTimeout(toastTimer);
        toastEl.classList.add('out');
        toastEl.classList.remove('show');
        action.onClick();
      },
    }) : null,
  ].filter(Boolean));
  toastEl.classList.remove('out', 'bump');
  void toastEl.offsetWidth;
  toastEl.classList.add('show', 'bump');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toastEl.classList.add('out');
    toastEl.classList.remove('show');
  }, duration ?? (action ? 7000 : 2200));
}

// ─── Info-bulles ─────────────────────────────────────────────────────────────
let tipEl = null;
let tipTimer = null;
let tipTarget = null;
let warm = false;
let warmTimer = null;

function hideTip() {
  clearTimeout(tipTimer);
  tipTarget = null;
  if (tipEl) tipEl.classList.remove('show');
  clearTimeout(warmTimer);
  warmTimer = setTimeout(() => { warm = false; }, 500);
}

function showTip(target) {
  const text = target.dataset.tip;
  if (!text) return;
  if (!tipEl) {
    tipEl = h('div.tooltip', { role: 'tooltip' });
    document.body.append(tipEl);
  }
  tipEl.replaceChildren(...[
    h('span', { text }),
    target.dataset.kbd ? h('kbd', { text: target.dataset.kbd }) : null,
  ].filter(Boolean));
  const r = target.getBoundingClientRect();
  tipEl.style.left = '0px';
  tipEl.style.top = '0px';
  tipEl.classList.add('measure');
  const t = tipEl.getBoundingClientRect();
  tipEl.classList.remove('measure');
  const below = r.top < window.innerHeight / 2;
  const x = Math.min(window.innerWidth - t.width - 8, Math.max(8, r.left + r.width / 2 - t.width / 2));
  const y = below ? r.bottom + 8 : r.top - t.height - 8;
  tipEl.style.left = `${Math.round(x)}px`;
  tipEl.style.top = `${Math.round(y)}px`;
  tipEl.classList.toggle('below', below);
  tipEl.classList.add('show');
  warm = true;
}

export function installTooltips() {
  document.addEventListener('pointerover', (e) => {
    const target = e.target.closest?.('[data-tip]');
    if (target === tipTarget) return;
    if (!target) return hideTip();
    clearTimeout(tipTimer);
    clearTimeout(warmTimer);
    tipTarget = target;
    tipEl?.classList.remove('show');
    tipTimer = setTimeout(() => tipTarget === target && showTip(target), warm ? 60 : 700);
  });
  document.addEventListener('pointerdown', hideTip, true);
  document.addEventListener('keydown', hideTip, true);
  window.addEventListener('blur', hideTip);
  labelIconButtons();
}

/** Un bouton qui n'a qu'une icône prend son infobulle comme nom accessible (lecteurs d'écran), et inversement. */
function labelIconButtons() {
  const label = (root) => {
    const buttons = root.matches?.('button') ? [root, ...root.querySelectorAll('button')] : root.querySelectorAll?.('button') || [];
    for (const b of buttons) {
      if (b.hasAttribute('aria-label') || b.hasAttribute('aria-labelledby')) continue;
      const tip = b.dataset?.tip || b.getAttribute('title');
      if (tip && !b.textContent.trim()) b.setAttribute('aria-label', tip);
    }
  };
  const pending = new Set();
  let scheduled = false;
  const flush = () => {
    scheduled = false;
    for (const n of pending) if (n.isConnected) label(n);
    pending.clear();
  };
  new MutationObserver((records) => {
    for (const r of records) {
      if (r.type === 'attributes') pending.add(r.target);
      else for (const n of r.addedNodes) if (n.nodeType === 1) pending.add(n);
    }
    if (!scheduled) { scheduled = true; requestAnimationFrame(flush); }
  }).observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['data-tip'] });
  label(document.body);
}

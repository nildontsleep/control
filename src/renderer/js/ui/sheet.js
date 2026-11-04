// Feuilles modales façon macOS : saisie, confirmation, contenu libre.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';

let openCount = 0;
const stack = []; // feuilles ouvertes, la plus récente en dernier (bouton retour d'Android)

export function openSheet({ title, width = 460, body, footer, onClose, canClose, className = '' }) {
  const backdrop = h('div.sheet-backdrop');
  const closeBtn = h('button.icon-btn.sheet-close', { type: 'button', 'data-tip': t('common.close'), html: icon('x', 16, 2) });
  const sheet = h(`div.sheet${className ? `.${className}` : ''}`, { role: 'dialog', 'aria-modal': 'true', style: { width: `${width}px` } },
    title ? h('header.sheet-head', {}, h('h2', { text: title }), closeBtn) : null,
    h('div.sheet-body', {}, body),
    footer ? h('footer.sheet-foot', {}, footer) : null);
  backdrop.append(sheet);
  document.getElementById('layer').append(backdrop);
  openCount++;
  const entry = { request: () => request() };
  stack.push(entry);
  const prevFocus = document.activeElement;

  // Échap, clic à côté et croix passent par canClose (une feuille avec des modifications peut demander confirmation).
  const request = () => { if (!canClose || canClose() !== false) close(); };

  function close(result) {
    if (!backdrop.isConnected || backdrop.classList.contains('closing')) return;
    const at = stack.indexOf(entry);
    if (at >= 0) stack.splice(at, 1);
    backdrop.classList.remove('open');
    backdrop.classList.add('closing');
    document.removeEventListener('keydown', onKey, true);
    openCount--;
    setTimeout(() => backdrop.remove(), 220);
    prevFocus?.focus?.({ preventScroll: true });
    onClose?.(result);
  }
  const onKey = (e) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      request();
    }
  };
  document.addEventListener('keydown', onKey, true);
  backdrop.addEventListener('pointerdown', (e) => { if (e.target === backdrop) request(); });
  closeBtn.addEventListener('click', () => request());
  dragToClose(sheet, backdrop, request);
  requestAnimationFrame(() => backdrop.classList.add('open'));
  return { close, el: sheet };
}

export const isSheetOpen = () => openCount > 0;

/** Ferme la feuille du dessus (comme sa croix) ; false s'il n'y en a pas. */
export function closeTopSheet() {
  const top = stack[stack.length - 1];
  if (!top) return false;
  top.request();
  return true;
}

// Téléphone : la feuille arrive par le bas ; on la referme en la tirant vers le bas depuis son en-tête.
function dragToClose(sheet, backdrop, request) {
  const head = sheet.querySelector('.sheet-head') || sheet;
  let start = null;
  head.addEventListener('pointerdown', (e) => {
    if (e.pointerType !== 'touch' || e.target.closest('button, input, select, textarea')) return;
    if (getComputedStyle(backdrop).alignItems !== 'end') return; // feuille centrée (grand écran) : rien à tirer
    start = { y: e.clientY, t: performance.now() };
    sheet.style.transition = 'none';
  });
  head.addEventListener('pointermove', (e) => {
    if (!start) return;
    const dy = Math.max(0, e.clientY - start.y);
    sheet.style.transform = `translateY(${dy}px)`;
  });
  const end = (e) => {
    if (!start) return;
    const dy = Math.max(0, e.clientY - start.y);
    const fast = dy > 40 && dy / (performance.now() - start.t) > 0.6;
    start = null;
    sheet.style.transition = '';
    sheet.style.transform = '';
    if (dy > 110 || fast) request();
  };
  head.addEventListener('pointerup', end);
  head.addEventListener('pointercancel', end);
}

export function promptSheet({ title, message, value = '', placeholder = '', confirmLabel = t('common.ok'), secret = false }) {
  return new Promise((resolve) => {
    let done = false;
    const input = h('input.text-input', { type: secret ? 'password' : 'text', value, placeholder, spellcheck: 'false', autocomplete: 'off' });
    const finish = (v) => {
      if (done) return;
      done = true;
      handle.close();
      resolve(v);
    };
    const ok = h('button.btn.btn-primary', { type: 'button', text: confirmLabel, onclick: () => finish(input.value.trim() || null) });
    const cancel = h('button.btn.btn-secondary', { type: 'button', text: t('common.cancel'), onclick: () => finish(null) });
    input.addEventListener('keydown', (e) => {
      if (e.key !== 'Enter') return;
      // Sans preventDefault, la fermeture rend le focus au bouton qui a ouvert la fenêtre et la même frappe le « clique » :
      // la fenêtre se rouvrirait aussitôt.
      e.preventDefault();
      finish(input.value.trim() || null);
    });
    const handle = openSheet({
      title,
      width: 400,
      body: [message ? h('p.sheet-text', { text: message }) : null, input],
      footer: [cancel, ok],
      onClose: () => finish(null),
    });
    setTimeout(() => { input.focus(); input.select(); }, 60);
  });
}

export function confirmSheet({ title, message, confirmLabel = t('common.ok'), destructive = false, checkbox = null }) {
  return new Promise((resolve) => {
    let done = false;
    let checked = false;
    const finish = (ok) => {
      if (done) return;
      done = true;
      handle.close();
      resolve({ ok, checked });
    };
    const box = checkbox ? h('label.check-row', {},
      h('input', { type: 'checkbox', onchange: (e) => { checked = e.target.checked; } }),
      h('span', { text: checkbox })) : null;
    const ok = h(`button.btn.${destructive ? 'btn-danger-solid' : 'btn-primary'}`, { type: 'button', text: confirmLabel, onclick: () => finish(true) });
    const cancel = h('button.btn.btn-secondary', { type: 'button', text: t('common.cancel'), onclick: () => finish(false) });
    const handle = openSheet({
      title,
      width: 420,
      body: [h('p.sheet-text', { text: message }), box],
      footer: [cancel, ok],
      onClose: () => finish(false),
    });
    setTimeout(() => (destructive ? cancel : ok).focus(), 60);
  });
}

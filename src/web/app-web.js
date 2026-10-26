// Ce que la version web ajoute à l'interface (chargé après l'application) : tiroir de navigation sur petit écran,
// menu contextuel à l'appui long, service worker.
import { h } from '/js/dom.js';
import { icon } from '/js/icons.js';
import { onRoute } from '/js/router.js';

const web = window.control?.web;

if (web) {
  document.documentElement.classList.add('platform-web');
  if ('serviceWorker' in navigator) navigator.serviceWorker.register('/sw.js').catch(() => {});

  const app = document.getElementById('app');
  const topbar = document.getElementById('topbar');

  // Bouton « menu » et voile : la barre latérale glisse par-dessus quand l'écran est étroit (voir app-web.css).
  const toggle = h('button.nav-toggle', { type: 'button', 'aria-label': 'Menu', html: icon('list', 20, 2), onclick: () => app.classList.toggle('nav-open') });
  topbar?.prepend(toggle);
  app.append(h('div.nav-scrim', { onclick: () => app.classList.remove('nav-open') }));
  onRoute(() => app.classList.remove('nav-open'));
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') app.classList.remove('nav-open'); });
  document.getElementById('sidebar')?.addEventListener('click', (e) => {
    // Choisir une entrée (ou une playlist) referme le tiroir.
    if (e.target.closest('.sb-item, .sb-playlist, .sb-pl')) setTimeout(() => app.classList.remove('nav-open'), 120);
  });

  // iPhone et iPad ne déclenchent pas « contextmenu » à l'appui long : on le simule (menus des morceaux, albums…).
  let press = null;
  let sawNative = false;
  document.addEventListener('contextmenu', () => { sawNative = true; }, true);
  document.addEventListener('touchstart', (e) => {
    if (e.touches.length !== 1 || e.target.closest('input, textarea, .menu, .sheet')) return;
    const { clientX, clientY, target } = e.touches[0];
    sawNative = false;
    clearTimeout(press?.timer);
    press = { x: clientX, y: clientY, timer: setTimeout(() => {
      if (sawNative) return;
      target.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX, clientY, button: 2 }));
    }, 550) };
  }, { passive: true });
  const cancel = () => clearTimeout(press?.timer);
  document.addEventListener('touchend', cancel, { passive: true });
  document.addEventListener('touchcancel', cancel, { passive: true });
  document.addEventListener('touchmove', (e) => {
    const t = e.touches[0];
    if (press && t && Math.hypot(t.clientX - press.x, t.clientY - press.y) > 10) cancel();
  }, { passive: true });
}

// ─── Choisir un dossier du serveur (administrateur) ────────────────────────
// Le bouton « Ajouter un dossier » des réglages appelle web.pickFolder() : on parcourt le disque du serveur.
if (web) {
  const { openSheet } = await import('/js/ui/sheet.js');
  const { t } = await import('/js/i18n.js');
  web.pickFolder = (start) => new Promise((resolve) => {
    let current = null;
    let done = false;
    const path = h('div.sheet-text', { style: { wordBreak: 'break-all', fontWeight: '600' } });
    const list = h('div.folder-list', { style: { maxHeight: '320px', overflowY: 'auto' } });
    const error = h('p.sheet-text', { style: { color: '#ff6a5f', minHeight: '1.2em' } });
    const choose = h('button.btn.btn-primary', { type: 'button', text: t('web.pickChoose') });
    const cancel = h('button.btn.btn-secondary', { type: 'button', text: t('common.cancel') });
    const finish = (value) => { if (done) return; done = true; sheet.close(); resolve(value); };
    const sheet = openSheet({ title: t('web.pickTitle'), width: 460, body: [path, list, error], footer: [cancel, choose], onClose: () => finish(null) });
    cancel.addEventListener('click', () => finish(null));
    choose.addEventListener('click', () => current && finish(current.path));

    async function go(dir) {
      error.textContent = '';
      try {
        current = await web.rpc('srv:browse', dir);
      } catch (err) {
        error.textContent = err.message;
        return;
      }
      path.textContent = current.path;
      list.replaceChildren(
        current.parent ? h('button.folder-row', { type: 'button', style: { width: '100%', textAlign: 'left' }, onclick: () => go(current.parent) }, h('span.folder-icon', { html: icon('chevron-left', 17, 1.8) }), h('span.folder-path', { text: '..' })) : null,
        ...current.dirs.map((d) => h('button.folder-row', { type: 'button', style: { width: '100%', textAlign: 'left' }, onclick: () => go(`${current.path.replace(/[\\/]$/, '')}/${d}`) }, h('span.folder-icon', { html: icon('folder', 17, 1.8) }), h('span.folder-path', { text: d }))));
    }
    go(start || window.control.settings.initial.libraryFolders?.[0] || '');
  });
}

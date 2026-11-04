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
  const root = document.documentElement;

  // iOS, appli ajoutée à l'écran d'accueil : la fenêtre est parfois annoncée plus courte que l'écran (de la hauteur de l'encoche),
  // ce qui laisse une bande vide en bas. On mesure l'écart et on rallonge l'application (--app-h, voir app-web.css).
  const fitScreen = () => {
    root.style.removeProperty('--app-h');
    if (navigator.standalone !== true) return;
    const full = innerHeight >= innerWidth ? Math.max(screen.width, screen.height) : Math.min(screen.width, screen.height);
    const gap = full - app.getBoundingClientRect().height;
    if (gap > 4 && gap < 100) root.style.setProperty('--app-h', `${full}px`);
  };
  fitScreen();
  for (const ev of ['resize', 'orientationchange', 'pageshow']) addEventListener(ev, () => setTimeout(fitScreen, 60));

  // L'application ne défile jamais d'un bloc (voir app-web.css) : si un focus l'a quand même décalée, on la remet en place.
  app.addEventListener('scroll', () => { if (app.scrollTop || app.scrollLeft) app.scrollTo(0, 0); });

  // Clavier virtuel : sa hauteur (--kb) garde feuilles et menus au-dessus de lui ; à sa fermeture, iOS laisse parfois la
  // page décalée vers le haut.
  if (window.visualViewport) {
    const vv = window.visualViewport;
    const onViewport = () => {
      const kb = Math.round(innerHeight - vv.height - vv.offsetTop);
      root.style.setProperty('--kb', kb > 80 ? `${kb}px` : '0px');
      if (kb <= 80 && document.scrollingElement.scrollTop) window.scrollTo(0, 0);
    };
    vv.addEventListener('resize', onViewport);
    vv.addEventListener('scroll', onViewport);
  }

  // Barre d'état (Android, iOS) et barre d'onglets : de la couleur du fond de l'application, thème clair ou sombre.
  const themeMeta = document.querySelector('meta[name="theme-color"]');
  if (themeMeta) {
    const paint = () => {
      const bg = getComputedStyle(document.body).backgroundColor;
      if (bg && !/rgba\(0, 0, 0, 0\)|transparent/.test(bg)) themeMeta.setAttribute('content', bg);
    };
    new MutationObserver(() => requestAnimationFrame(paint)).observe(root, { attributes: true, attributeFilter: ['data-theme', 'class', 'style'] });
    requestAnimationFrame(paint);
  }

  // iOS ignore audio.volume (le volume se règle avec les boutons de l'appareil) : le curseur ne servirait à rien.
  try {
    const probe = new Audio();
    probe.volume = 0.5;
    if (probe.volume !== 0.5) root.classList.add('no-volume');
  } catch { /* pas d'élément audio : rien à régler */ }

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

  // « À l'écoute » : glisser vers le bas (depuis l'en-tête, la pochette ou le titre) le referme, comme sur iOS et Android.
  const np = document.getElementById('nowPlaying');
  if (np) {
    let drag = null;
    np.addEventListener('pointerdown', (e) => {
      if (e.pointerType !== 'touch' || !np.classList.contains('open') || np.classList.contains('side-open') || app.classList.contains('immersive')) return;
      if (!e.target.closest('.np-top, .np-art, .np-info') || e.target.closest('button, input, a')) return;
      drag = { x: e.clientX, y: e.clientY, t: performance.now(), dy: 0, on: false };
    }, { passive: true });
    np.addEventListener('pointermove', (e) => {
      if (!drag) return;
      const dy = e.clientY - drag.y;
      if (!drag.on) {
        if (Math.abs(e.clientX - drag.x) > 14 && Math.abs(e.clientX - drag.x) > dy) { drag = null; return; } // geste horizontal
        if (dy < 10) return;
        drag.on = true;
        np.style.transition = 'none';
      }
      drag.dy = Math.max(0, dy);
      np.style.transform = `translateY(${drag.dy}px)`;
    }, { passive: true });
    const end = () => {
      if (!drag) return;
      const { on, dy, t } = drag;
      drag = null;
      if (!on) return;
      np.style.transition = '';
      np.style.transform = '';
      if (dy > 130 || (dy > 50 && dy / (performance.now() - t) > 0.7)) np.querySelector('.np-close')?.click();
    };
    np.addEventListener('pointerup', end);
    np.addEventListener('pointercancel', end);
  }

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

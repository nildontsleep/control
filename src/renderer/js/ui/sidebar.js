// Barre latérale translucide : recherche, bibliothèque, téléchargements, playlists.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { lib, onLibrary, playlistById, loadPlaylists } from '../data.js';
import { navigate, currentRoute, onRoute, back } from '../router.js';
import { trafficLights } from './traffic.js';
import { openMenu } from './menu.js';
import { DRAG_TRACKS, addToPlaylist, createPlaylistWith } from './actions.js';
import { openSmartEditor } from './smart.js';
import { renamePlaylist, deletePlaylist, exportPlaylist } from '../views/playlist.js';
import { toast } from './feedback.js';
import { isWeb, can } from '../web.js';
import { openAccountMenu } from './account.js';

const playlistsApi = window.control.playlists;
const NAV = [
  { id: 'songs', icon: 'music-2' },
  { id: 'albums', icon: 'disc-3' },
  { id: 'artists', icon: 'mic-vocal' },
  { id: 'videos', icon: 'film' },
  { id: 'stats', icon: 'chart-column' },
];

/** Entrée de la barre latérale à surligner pour une page donnée. */
function sectionOf(route) {
  const param = String(route.param ?? '');
  switch (route.name) {
    case 'playlist': return `playlist:${route.param}`;
    case 'album': return 'albums';
    case 'artist': return 'artists';
    case 'title': return param.startsWith('anime:') ? 'anime' : 'watch';
    case 'watching': return param.startsWith('anime|') ? 'anime' : 'watch';
    case 'shelf': return param.startsWith('anime:') ? 'anime' : 'watch';
    default: return route.name;
  }
}

export function createSidebar(root, { openSettings }) {
  let activeDownloads = 0;
  const search = h('input.sb-search-input', { type: 'search', placeholder: t('search.placeholder'), spellcheck: 'false', autocomplete: 'off' });
  const clearBtn = h('button.sb-search-clear', { type: 'button', html: icon('x', 12, 2.4), tabindex: '-1' });
  const status = h('div.sb-status');
  const playlistsBox = h('div.sb-list');
  const navBox = h('div.sb-section');
  const downloadsBadge = h('span.sb-badge');

  let searchTimer = null;
  search.addEventListener('input', () => {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      const q = search.value;
      clearBtn.classList.toggle('show', !!q);
      if (q.trim()) navigate('search', q, { replace: currentRoute()?.name === 'search' });
      else if (currentRoute()?.name === 'search') back();
    }, 110);
  });
  search.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      search.value = '';
      search.dispatchEvent(new Event('input'));
      search.blur();
    }
  });
  clearBtn.addEventListener('click', () => {
    search.value = '';
    search.dispatchEvent(new Event('input'));
    search.focus();
  });

  function navItem({ id, icon: ic, label, badge, onClick }) {
    const btn = h('button.sb-item', { type: 'button', dataset: { route: id } },
      h('span.sb-icon', { html: icon(ic, 17, 1.9) }),
      h('span.sb-label', { text: label }),
      badge || null);
    btn.addEventListener('click', onClick || (() => navigate(id)));
    return btn;
  }

  function renderNav() {
    navBox.replaceChildren(
      h('div.sb-heading', { text: t('nav.library') }),
      ...NAV.map((n) => navItem({ id: n.id, icon: n.icon, label: t(`nav.${n.id}`) })),
      h('div.sb-heading', { text: t('nav.watch') }),
      navItem({ id: 'watch', icon: 'clapperboard', label: t('nav.movies') }),
      navItem({ id: 'anime', icon: 'tv-minimal', label: t('nav.anime') }),
      navItem({ id: 'mylist', icon: 'bookmark', label: t('nav.mylist') }),
      h('div.sb-heading', { text: t('nav.get') }),
      navItem({ id: 'downloads', icon: 'cloud-download', label: t('nav.downloads'), badge: downloadsBadge }),
    );
    paintActive();
  }

  function playlistItem(p) {
    const smart = p.kind === 'smart';
    const btn = h('button.sb-item.sb-playlist', { type: 'button', dataset: { route: `playlist:${p.id}` } },
      h('span.sb-icon', { html: icon(smart ? 'sparkles' : 'list-music', 17, 1.9) }),
      h('span.sb-label', { text: p.name }));
    btn.addEventListener('click', () => navigate('playlist', p.id));
    btn.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      const pl = playlistById(p.id);
      openMenu({ getBoundingClientRect: () => ({ left: e.clientX, top: e.clientY, right: e.clientX, bottom: e.clientY, width: 0, height: 0 }), classList: { add() {}, remove() {} }, contains: () => false }, [
        { label: t('pl.rename'), icon: 'square-pen', action: () => renamePlaylist(pl) },
        smart ? { label: t('pl.editRules'), icon: 'sliders-horizontal', action: () => openSmartEditor(pl) } : null,
        isWeb ? null : { label: t('pl.export'), icon: 'upload', action: () => exportPlaylist(pl) },
        { separator: true },
        { label: t('common.delete'), icon: 'trash-2', action: async () => { if (await deletePlaylist(pl) && currentRoute()?.name === 'playlist' && Number(currentRoute().param) === pl.id) navigate('songs'); } },
      ].filter(Boolean));
    });
    if (!smart) {
      btn.addEventListener('dragover', (e) => {
        if (!e.dataTransfer.types.includes(DRAG_TRACKS)) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = 'copy';
        btn.classList.add('drop');
      });
      btn.addEventListener('dragleave', () => btn.classList.remove('drop'));
      btn.addEventListener('drop', (e) => {
        btn.classList.remove('drop');
        const ids = JSON.parse(e.dataTransfer.getData(DRAG_TRACKS) || '[]');
        const tracks = ids.map((id) => lib.byId.get(id)).filter(Boolean);
        if (!tracks.length) return;
        e.preventDefault();
        addToPlaylist(p, tracks);
      });
    }
    return btn;
  }

  function renderPlaylists() {
    playlistsBox.replaceChildren(...lib.playlists.map(playlistItem));
    if (!lib.playlists.length) playlistsBox.append(h('div.sb-hint', { text: t('pl.sidebarHint') }));
    paintActive();
  }

  function paintActive() {
    const route = currentRoute();
    const key = !route ? '' : sectionOf(route);
    for (const el of root.querySelectorAll('.sb-item')) el.classList.toggle('active', el.dataset.route === key);
  }

  const addMenu = h('button.icon-btn.sb-add', { type: 'button', 'data-tip': t('pl.add'), html: icon('plus', 16, 2.2) });
  addMenu.addEventListener('click', () => {
    openMenu(addMenu, [
      { label: t('pl.new'), icon: 'list-plus', action: () => createPlaylistWith().then((id) => id && navigate('playlist', id)) },
      { label: t('pl.newSmart'), icon: 'sparkles', action: () => openSmartEditor(null) },
      ...(isWeb ? [] : [{ separator: true }, { label: t('pl.import'), icon: 'import', action: async () => {
        const res = await playlistsApi.import();
        if (res) {
          await loadPlaylists();
          toast(t('toast.imported', { n: res.matched, total: res.total }), 'check');
          navigate('playlist', res.id);
        }
      } }]),
    ], { align: 'start' });
  });

  const settingsDot = h('span.sb-dot', { 'data-tip': t('upd.dotTip') });
  const settingsBtn = h('button.sb-item.sb-settings', { type: 'button', onclick: () => openSettings() },
    h('span.sb-icon', { html: icon('settings', 17, 1.9) }), h('span.sb-label', { text: t('nav.settings') }), settingsDot);

  // Sur le web : le nom du serveur à la place des boutons de fenêtre, et le compte au-dessus des réglages.
  const brand = isWeb ? h('div.sb-brand', {}, h('img', { src: '/assets/icon.png', alt: '' }), h('span', { text: window.control.web.server.name })) : null;
  const accountBtn = isWeb
    ? h('button.sb-item.sb-account', { type: 'button', onclick: (e) => openAccountMenu(e.currentTarget) },
      h('span.sb-avatar', { text: window.control.web.user.username.slice(0, 1).toUpperCase() }),
      h('span.sb-label', { text: window.control.web.user.username }))
    : null;

  root.replaceChildren(
    h('div.sb-top', {}, trafficLights(), brand),
    h('div.sb-search', {}, h('span.sb-search-icon', { html: icon('search', 14, 2.2) }), search, clearBtn),
    h('nav.sb-scroll', {},
      navBox,
      h('div.sb-section', {},
        h('div.sb-heading.with-action', {}, h('span', { text: t('nav.playlists') }), addMenu),
        playlistsBox)),
    status,
    h('div.sb-foot', {}, accountBtn, settingsBtn),
  );
  renderNav();
  renderPlaylists();

  onRoute(paintActive);
  onLibrary((what) => {
    if (what === 'playlists' || what === 'tracks') renderPlaylists();
  });

  return {
    focusSearch() {
      search.focus();
      search.select();
    },
    setSearch(value) {
      search.value = value;
      clearBtn.classList.toggle('show', !!value);
    },
    setUpdate(on) {
      settingsDot.classList.toggle('show', !!on);
    },
    setDownloads(n) {
      activeDownloads = n;
      downloadsBadge.textContent = n ? String(n) : '';
      downloadsBadge.classList.toggle('show', n > 0);
    },
    setScan(progress) {
      if (!progress || progress.phase === 'done') {
        status.classList.remove('show');
        return;
      }
      const text = progress.phase === 'discover'
        ? t('scan.discover', { n: progress.found })
        : t('scan.read', { done: progress.done, total: progress.total });
      status.replaceChildren(
        h('span.spinner.small', {}, ...Array.from({ length: 12 }, (_, i) => h('i', { style: { '--i': i } }))),
        h('span', { text }));
      status.classList.add('show');
    },
    rerender() {
      renderNav();
      renderPlaylists();
      search.placeholder = t('search.placeholder');
    },
    get activeDownloads() {
      return activeDownloads;
    },
  };
}

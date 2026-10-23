// control — point d’entrée : coque de l’application, navigation, raccourcis, import par glisser-déposer.

import { $, debounce } from './dom.js';
import { icon } from './icons.js';
import { t, setLanguage, currentLanguage, resolveLanguage } from './i18n.js';
import { settings, update, subscribe } from './settings.js';
import { ACCENTS, hexToRgb, luminance } from './color.js';
import { loadAll, loadPlaylists } from './data.js';
import { player, on as onPlayer } from './player.js';
import {
  registerView, mount, navigate, back, forward, canBack, canForward, onRoute, refresh, currentRoute,
} from './router.js';
import { coverUrl } from './ui/cover.js';
import { createSidebar } from './ui/sidebar.js';
import { createPlayerBar } from './ui/playerbar.js';
import { createNowPlaying } from './ui/nowplaying.js';
import { openSettings } from './ui/settings-sheet.js';
import { isSheetOpen } from './ui/sheet.js';
import { isMenuOpen } from './ui/menu.js';
import { toast, installTooltips } from './ui/feedback.js';
import { songsView } from './views/songs.js';
import { albumsView, albumView } from './views/albums.js';
import { artistsView, artistView } from './views/artists.js';
import { videosView } from './views/videos.js';
import { playlistView } from './views/playlist.js';
import { searchView } from './views/search.js';
import { downloadsView } from './views/downloads.js';
import { moviesView, animeView } from './views/watch.js';
import { titleView } from './views/title.js';
import { watchingView } from './views/watching.js';
import { shelfView } from './views/shelf.js';
import { mylistView } from './views/mylist.js';
import { statsView } from './views/stats.js';
import { offerMonthlyRecap } from './stats/recap-prompt.js';
import { startPresence } from './discord/presence.js';
import { startUpdates, onUpdate } from './update/state.js';
import { applyAmbience, setSceneActive } from './theme/ambience.js';
import { LOOK_KEYS } from './theme/presets.js';

const control = window.control;
const root = document.documentElement;
const app = $('#app');

// ─── Thème ───────────────────────────────────────────────────────────────────
const darkQuery = matchMedia('(prefers-color-scheme: dark)');

function applyTheme() {
  const dark = settings.theme === 'dark' || settings.theme === 'oled' || (settings.theme === 'auto' && darkQuery.matches);
  root.dataset.theme = dark ? 'dark' : 'light';
  const hex = settings.accent === 'custom' ? settings.customAccent : (ACCENTS[settings.accent] || ACCENTS.pink)[dark ? 'dark' : 'light'];
  const rgb = hexToRgb(hex);
  root.style.setProperty('--accent', `rgb(${rgb.join(' ')})`);
  root.style.setProperty('--accent-rgb', rgb.join(' '));
  root.style.setProperty('--on-accent', luminance(rgb) > 0.5 ? '#1d1d1f' : '#ffffff');
  app.classList.toggle('no-ambient', !settings.ambient);
  root.classList.toggle('no-anim', !settings.animations);
  root.classList.toggle('compact', !!settings.compact);
  applyAmbience({ dark, rgb, active: !app.classList.contains('inactive') });
}
darkQuery.addEventListener('change', applyTheme);

// ─── Fond coloré (pochette floutée) ──────────────────────────────────────────
const layers = [$('#ambA'), $('#ambB')];
let pendingVideo = false;
let layerIndex = 0;
let lastCover = null;

function setAmbient(cover) {
  if (cover === lastCover) return;
  lastCover = cover;
  layerIndex = 1 - layerIndex;
  const next = layers[layerIndex];
  const prev = layers[1 - layerIndex];
  next.style.backgroundImage = cover ? `url(${coverUrl(cover, true)})` : 'none';
  next.classList.add('on');
  prev.classList.remove('on');
}

// ─── Coque ───────────────────────────────────────────────────────────────────
let sidebar;
let playerBar;
let nowPlaying;

function paintNav() {
  const mkBtn = (el, ic, enabled, tip, fn) => {
    el.innerHTML = icon(ic, 18, 2.1);
    el.disabled = !enabled;
    el.dataset.tip = tip;
    el.onclick = fn;
  };
  mkBtn($('#navBack'), 'chevron-left', canBack(), t('kb.back'), back);
  mkBtn($('#navForward'), 'chevron-right', canForward(), '', forward);
}

function buildShell() {
  const scroller = $('#scroller');
  mount($('#view'), scroller);
  registerView('songs', songsView);
  registerView('albums', albumsView);
  registerView('album', albumView);
  registerView('artists', artistsView);
  registerView('artist', artistView);
  registerView('videos', videosView);
  registerView('playlist', playlistView);
  registerView('search', searchView);
  registerView('downloads', downloadsView);
  registerView('watch', moviesView);
  registerView('anime', animeView);
  registerView('title', titleView);
  registerView('watching', watchingView);
  registerView('shelf', shelfView);
  registerView('mylist', mylistView);
  registerView('stats', statsView);

  nowPlaying = createNowPlaying($('#nowPlaying'));
  sidebar = createSidebar($('#sidebar'), { openSettings: () => openSettings() });
  playerBar = createPlayerBar($('#playerbar'), {
    openNowPlaying: () => nowPlaying.open(),
    openQueue: () => nowPlaying.isOpen ? nowPlaying.close() : nowPlaying.open('queue'),
    openLyrics: () => nowPlaying.open('lyrics'),
  });

  onRoute((route) => {
    paintNav();
    app.dataset.view = route.name;
    if (!['search', 'downloads', 'title', 'watching', 'shelf'].includes(route.name)) update({ lastView: { name: route.name, param: route.param } }, { persist: true });
    if (route.name !== 'search') sidebar.setSearch('');
  });
}

// ─── Données ─────────────────────────────────────────────────────────────────
// Pages qui ne dépendent pas de la bibliothèque musicale : inutile (et nuisible pour le lecteur vidéo) de les redessiner.
const LIBRARY_FREE = new Set(['downloads', 'watching', 'title', 'watch', 'anime', 'shelf', 'mylist', 'stats']);
const refreshView = debounce(() => {
  const route = currentRoute();
  if (route && !LIBRARY_FREE.has(route.name)) refresh();
}, 120);

function wireLibrary() {
  const api = control.library;
  api.onChange(async ({ what }) => {
    if (what === 'playlists') await loadPlaylists();
    else await loadAll();
    player.refreshReferences();
    player.prune();
    refreshView();
  });
  api.onScan((progress) => {
    sidebar.setScan(progress);
    if (progress.phase === 'done' && progress.added) toast(t('toast.scanDone', { n: progress.added }), 'music-2');
  });
}

function wireDownloads() {
  const status = new Map();
  const count = () => [...status.values()].filter((s) => ['queued', 'downloading', 'processing'].includes(s)).length;
  control.downloads.list().then((jobs) => {
    jobs.forEach((j) => status.set(j.id, j.status));
    sidebar.setDownloads(count());
  });
  control.downloads.onUpdate((job) => {
    const prev = status.get(job.id);
    status.set(job.id, job.status);
    sidebar.setDownloads(count());
    if (job.status === 'done' && prev && prev !== 'done') toast(job.title || job.current ? t('dl.doneToast', { title: job.title || job.current }) : t('dl.doneGeneric'), 'circle-check');
  });
  control.downloads.onReset((jobs) => {
    status.clear();
    jobs.forEach((j) => status.set(j.id, j.status));
    sidebar.setDownloads(count());
  });
}

// ─── Glisser-déposer de fichiers ─────────────────────────────────────────────
function wireDrop() {
  const zone = $('#dropzone');
  $('#dropTitle').textContent = t('drop.title');
  $('#dropText').textContent = t('drop.text');
  let depth = 0;
  const hasFiles = (e) => e.dataTransfer?.types?.includes('Files');
  window.addEventListener('dragenter', (e) => {
    if (!hasFiles(e)) return;
    depth++;
    zone.classList.add('show');
  });
  window.addEventListener('dragleave', (e) => {
    if (!hasFiles(e)) return;
    depth = Math.max(0, depth - 1);
    if (!depth) zone.classList.remove('show');
  });
  window.addEventListener('dragover', (e) => { if (hasFiles(e)) e.preventDefault(); });
  window.addEventListener('drop', async (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    depth = 0;
    zone.classList.remove('show');
    const paths = [...e.dataTransfer.files].map((f) => control.pathForFile(f)).filter(Boolean);
    if (!paths.length) return;
    const ids = await control.library.addPaths(paths);
    if (ids.length) toast(t('toast.scanDone', { n: ids.length }), 'music-2');
    else toast(t('toast.droppedNone'), 'info');
  });
}

// ─── Clavier ─────────────────────────────────────────────────────────────────
function wireKeys() {
  // Un bouton cliqué garde le focus : Espace réactiverait alors ce bouton au lieu de lancer la lecture.
  document.addEventListener('click', (e) => {
    const btn = e.target.closest?.('button');
    if (btn && !btn.closest('.sheet, .menu')) setTimeout(() => btn.blur());
  });
  document.addEventListener('keydown', (e) => {
    if (isSheetOpen() || isMenuOpen()) return;
    const tag = e.target?.tagName;
    const typing = tag === 'INPUT' && !['range', 'checkbox'].includes(e.target.type) || tag === 'TEXTAREA';
    const ctrl = e.ctrlKey || e.metaKey;
    const key = e.key;

    if (ctrl && key === ',') { e.preventDefault(); openSettings(); return; }
    if (ctrl && (key === 'f' || key === 'F')) { e.preventDefault(); sidebar.focusSearch(); return; }
    if (e.altKey && key === 'ArrowLeft') { e.preventDefault(); back(); return; }
    if (e.altKey && key === 'ArrowRight') { e.preventDefault(); forward(); return; }
    if (key === 'Escape') {
      if (nowPlaying.isOpen) {
        if (nowPlaying.isFullscreen) control.win.setFullscreen(false);
        else nowPlaying.close();
        e.preventDefault();
      }
      return;
    }
    if (typing) return;
    // Pendant un film ou un épisode, le clavier appartient au lecteur vidéo : pas de raccourcis musique.
    if (currentRoute()?.name === 'watching') return;

    if (ctrl && key === 'ArrowRight') { e.preventDefault(); player.next(); return; }
    if (ctrl && key === 'ArrowLeft') { e.preventDefault(); player.prev(); return; }
    if (ctrl && key === 'ArrowUp') { e.preventDefault(); player.setVolume(settings.volume + 0.05); return; }
    if (ctrl && key === 'ArrowDown') { e.preventDefault(); player.setVolume(settings.volume - 0.05); return; }
    if (ctrl || e.altKey) return;

    if (key === ' ' && tag !== 'BUTTON') { e.preventDefault(); player.toggle(); return; }
    if (key === 'ArrowRight' && tag !== 'INPUT') { e.preventDefault(); player.seekBy(10); return; }
    if (key === 'ArrowLeft' && tag !== 'INPUT') { e.preventDefault(); player.seekBy(-10); return; }
    switch (key.toLowerCase()) {
      case 's': player.toggleShuffle(); break;
      case 'r': player.cycleRepeat(); break;
      case 'm': player.toggleMute(); break;
      case 'q': if (nowPlaying.isOpen) nowPlaying.close(); else nowPlaying.open('queue'); break;
      case 'e': nowPlaying.open('eq'); break;
      case 'l': nowPlaying.open('lyrics'); break;
      case 'f': if (nowPlaying.isOpen) control.win.state().then((st) => control.win.setFullscreen(!st.fullscreen)); break;
      case 'enter': if (tag !== 'BUTTON' && !e.target.closest?.('.tl')) nowPlaying.toggle(); else return; break;
      case '/': e.preventDefault(); sidebar.focusSearch(); break;
      default: return;
    }
    e.preventDefault();
  });
}

// ─── Initialisation ──────────────────────────────────────────────────────────
async function init() {
  setLanguage(settings.language);
  applyTheme();
  installTooltips();
  if (control.platform === 'win32') root.classList.add('platform-win');

  buildShell();
  wireKeys();
  wireDrop();

  control.win.onState((s) => {
    app.classList.toggle('inactive', !s.focused);
    setSceneActive(!!s.focused);
    app.classList.toggle('fullscreen', !!s.fullscreen);
    app.classList.toggle('maximized', !!s.maximized);
    nowPlaying.setFullscreen(!!s.fullscreen);
  });
  control.win.state().then((s) => {
    app.classList.toggle('inactive', s.focused === false);
    app.classList.toggle('fullscreen', !!s.fullscreen);
  });

  await loadAll();
  wireLibrary();
  wireDownloads();
  control.library.scan();

  let lastTrackId = null;
  onPlayer('track', (track) => {
    setAmbient(track?.cover || null);
    document.title = track ? `${track.title}${track.artist ? ` — ${track.artist}` : ''}` : 'control';
    // Une vidéo qui démarre ouvre « À l'écoute » ; un simple rechargement de la bibliothèque, non.
    if (track?.id !== lastTrackId) pendingVideo = track?.kind === 'video';
    lastTrackId = track?.id ?? null;
  });
  onPlayer('error', (track) => toast(t('toast.cannotPlay', { title: track.title }), 'triangle-alert'));
  onPlayer('state', () => {
    if (pendingVideo && player.playing) {
      pendingVideo = false;
      nowPlaying.open();
    }
  });

  player.restoreSession();
  setAmbient(player.track?.cover || null);

  const last = settings.lastView;
  const valid = new Set(['songs', 'albums', 'artists', 'videos', 'album', 'artist', 'playlist', 'watch', 'anime', 'mylist', 'stats']);
  if (last && valid.has(last.name)) navigate(last.name, last.param);
  else navigate('songs');

  subscribe((patch, source) => {
    if ([...LOOK_KEYS, 'animations', 'compact'].some((k) => k in patch)) applyTheme();
    if ('compact' in patch && !LIBRARY_FREE.has(currentRoute()?.name)) refresh();
    if ('language' in patch && source !== 'remote' && resolveLanguage(settings.language) !== currentLanguage()) setTimeout(() => location.reload(), 120);
  });
  requestAnimationFrame(() => root.classList.add('ready'));
  setTimeout(() => offerMonthlyRecap().catch(() => {}), 6000);
  startPresence();
  onUpdate((s) => sidebar.setUpdate(['available', 'ready'].includes(s.phase)));
  startUpdates({
    onAvailable: (s) => toast(t('upd.available', { v: s.version }), 'cloud-download', { action: { label: t('upd.view'), onClick: () => openSettings('about') }, duration: 10000 }),
  });
}

init();

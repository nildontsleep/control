// Réglages : feuille modale à onglets (Général, Bibliothèque, Téléchargements, À propos).

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t, currentLanguage } from '../i18n.js';
import { settings, update, subscribe, resetAll } from '../settings.js';
import { lib } from '../data.js';
import { ACCENTS } from '../color.js';
import { openSheet } from './sheet.js';
import { toast } from './feedback.js';
import {
  createScope, withScope, refreshScope, toggle, segmented, popup, swatches, button, row, group, badge, live,
} from './controls.js';

const library = window.control.library;
const downloads = window.control.downloads;
const tools = window.control.tools;
const app = window.control.app;

const B = { blue: '#0a84ff', pink: '#ff375f', green: '#30d158', orange: '#ff9f0a', gray: '#8e8e93', purple: '#bf5af2', teal: '#40c8e0', red: '#ff453a' };

export const SHORTCUTS = [
  [['Espace'], 'kb.playPause'], [['Ctrl', '→'], 'kb.next'], [['Ctrl', '←'], 'kb.prev'], [['→'], 'kb.seekForward'], [['←'], 'kb.seekBack'],
  [['Ctrl', '↑'], 'kb.volUp'], [['Ctrl', '↓'], 'kb.volDown'], [['S'], 'kb.shuffle'], [['R'], 'kb.repeat'], [['M'], 'kb.mute'],
  [['Q'], 'kb.queue'], [['E'], 'kb.eq'], [['Entrée'], 'kb.nowPlaying'], [['Ctrl', 'F'], 'kb.search'], [['Alt', '←'], 'kb.back'],
  [['Ctrl', ','], 'kb.settings'], [['F'], 'kb.fullscreen'], [['Échap'], 'kb.escape'],
];

export function openSettings(initial = 'general') {
  const scope = createScope();
  let tabId = initial;
  const body = h('div.settings-body');
  const nav = h('nav.settings-tabs', { role: 'tablist' });
  let info = null;

  const TABS = [
    { id: 'general', icon: 'settings' },
    { id: 'library', icon: 'hard-drive' },
    { id: 'downloads', icon: 'cloud-download' },
    { id: 'about', icon: 'info' },
  ];

  const builders = {
    general() {
      const langs = ['auto', 'fr', 'en'].map((v) => ({ value: v, label: t(`lang.${v}`) }));
      const accents = Object.entries(ACCENTS).map(([value, c]) => ({ value, color: c.dark, label: t(`accent.${value}`) }));
      return [
        group(t('set.appearance'), [
          row(t('set.theme'), segmented('theme', [
            { value: 'auto', label: t('theme.auto'), icon: 'circle-dashed' },
            { value: 'light', label: t('theme.light'), icon: 'sun' },
            { value: 'dark', label: t('theme.dark'), icon: 'moon' },
          ], { compact: true, iconsOnly: true }), { badge: badge('palette', B.purple) }),
          row(t('set.accent'), swatches('accent', accents, { custom: { path: 'customAccent', label: t('accent.custom') } }), { className: 'row-stack' }),
          row(t('set.ambient'), toggle('ambient'), { hint: t('set.ambientHint'), badge: badge('sparkles', B.pink) }),
          row(t('set.compact'), toggle('compact'), { hint: t('set.compactHint'), badge: badge('list', B.teal) }),
          row(t('set.animations'), toggle('animations'), { badge: badge('wand-sparkles', B.orange) }),
        ]),
        group(t('set.language'), [
          row(t('set.language'), popup('language', langs), { badge: badge('languages', B.blue) }),
        ], { footer: t('set.languageHint') }),
      ];
    },

    library() {
      const folders = h('div.folder-list');
      const renderFolders = () => {
        const list = settings.libraryFolders || [];
        folders.replaceChildren(...(list.length
          ? list.map((f) => h('div.folder-row', {},
            h('span.folder-icon', { html: icon('folder', 17, 1.8) }),
            h('span.folder-path', { text: f, title: f }),
            h('button.icon-btn', { type: 'button', 'data-tip': t('common.remove'), html: icon('x', 15, 2), onclick: async () => { await library.removeFolder(f); } })))
          : [h('div.folder-empty', { text: t('set.noFolders') })]));
      };
      live(renderFolders);
      const stats = h('div.row.row-block.stats');
      live(() => {
        stats.replaceChildren(
          h('div.stat', {}, h('strong', { text: String(lib.audio.length) }), h('span', { text: t('nav.songs') })),
          h('div.stat', {}, h('strong', { text: String(lib.albums.length) }), h('span', { text: t('nav.albums') })),
          h('div.stat', {}, h('strong', { text: String(lib.artists.length) }), h('span', { text: t('nav.artists') })),
          h('div.stat', {}, h('strong', { text: String(lib.videos.length) }), h('span', { text: t('nav.videos') })));
      });
      return [
        group(t('set.folders'), [
          h('div.row.row-block', {}, folders),
          h('div.row.row-block', {}, h('div.row-buttons', {},
            button(t('set.addFolder'), { icon: 'folder-plus', kind: 'primary', small: true, onClick: () => library.addFolder() }),
            button(t('set.scanNow'), { icon: 'refresh-cw', small: true, onClick: async () => { toast(t('set.scanning'), 'refresh-cw'); await library.scan(); } }))),
        ], { footer: t('set.foldersHint') }),
        group(t('set.inLibrary'), [stats]),
      ];
    },

    downloads() {
      const toolRows = h('div.tool-rows');
      const renderTools = async () => {
        const s = await tools.status();
        const line = (name, item, note) => h('div.row', {},
          h('div.row-label', {}, h('span.row-title', { text: name }), h('span.row-hint', { text: item.installed ? (item.version ? `v${item.version}` : t('tools.installedShort')) : t('tools.missing') })),
          h('div.row-control', {}, note || null));
        toolRows.replaceChildren(
          line('yt-dlp', s.ytdlp),
          line('ffmpeg', s.ffmpeg),
          line('deno', s.deno, h('span.row-hint', { text: t('tools.denoNote') })));
      };
      renderTools();
      const dir = h('div.folder-row', {}, h('span.folder-icon', { html: icon('folder', 17, 1.8) }), h('span.folder-path'));
      live(() => {
        const p = settings.downloadDir || info?.downloadDir || '';
        dir.querySelector('.folder-path').textContent = p;
        dir.querySelector('.folder-path').title = p;
      });
      return [
        group(t('set.dlFolder'), [
          h('div.row.row-block', {}, dir),
          h('div.row.row-block', {}, h('div.row-buttons', {},
            button(t('common.change'), { icon: 'folder-open', small: true, onClick: () => downloads.chooseDir() }),
            button(t('dl.openFolder'), { icon: 'external-link', small: true, onClick: () => downloads.openDir() }))),
        ]),
        group(t('set.dlBehavior'), [
          row(t('set.concurrency'), segmented({
            get: () => settings.dl.concurrency,
            set: (v) => update({ dl: { concurrency: v } }),
          }, [1, 2, 3, 4].map((n) => ({ value: n, label: String(n) })), { compact: true })),
          row(t('set.cookies'), popup({
            get: () => settings.dl.cookies || '',
            set: (v) => update({ dl: { cookies: v } }),
          }, [{ value: '', label: t('cookies.none') }, ...['firefox', 'chrome', 'edge', 'brave', 'opera', 'vivaldi', 'chromium'].map((b) => ({ value: b, label: b[0].toUpperCase() + b.slice(1) }))]),
          { hint: t('set.cookiesHint'), badge: badge('globe', B.blue) }),
          row(t('set.autoUpdate'), toggle({
            get: () => settings.dl.autoUpdate !== false,
            set: (v) => update({ dl: { autoUpdate: v } }),
          }), { hint: t('set.autoUpdateHint'), badge: badge('refresh-cw', B.green) }),
        ]),
        group(t('set.tools'), [toolRows], {
          action: button(t('tools.update'), { small: true, kind: 'plain', onClick: async () => {
            toast(t('tools.updating'), 'refresh-cw');
            const r = await tools.update();
            toast(r.error ? t('tools.updateFailed') : r.updated ? t('tools.updated', { v: r.version }) : t('tools.upToDate'), r.error ? 'triangle-alert' : 'check');
            renderTools();
          } }),
        }),
      ];
    },

    about() {
      const space = currentLanguage() === 'fr' ? 'Espace' : 'Space';
      let armed = false;
      const reset = button(t('set.reset'), { kind: 'danger', small: true });
      reset.addEventListener('click', () => {
        if (!armed) {
          armed = true;
          reset.querySelector('span:last-child').textContent = t('set.resetConfirm');
          setTimeout(() => {
            armed = false;
            if (reset.isConnected) reset.querySelector('span:last-child').textContent = t('set.reset');
          }, 3000);
          return;
        }
        resetAll();
        setTimeout(() => location.reload(), 150);
      });
      const edition = !info ? '' : info.portable ? t('about.portable') : t('about.installed');
      return [
        h('div.about-card', {},
          h('img.about-icon', { src: 'assets/icon.png', alt: '' }),
          h('div.about-name', { text: 'control' }),
          h('div.about-tagline', { text: t('about.tagline') }),
          h('div.about-meta', { text: `${t('about.version', { v: info?.version || '0.1.0' })}${edition ? ` · ${edition}` : ''}` })),
        group(t('set.shortcuts'), SHORTCUTS.map(([keys, label]) => row(t(label), h('span.kbds', {}, ...keys.map((k) => h('kbd', { text: k === 'Espace' ? space : k })))))),
        group(t('set.data'), [
          h('div.row.row-block', {}, h('div.row-buttons', {}, reset, button(t('about.dataDir'), { icon: 'folder-open', small: true, onClick: () => app.openDataDir() }))),
        ], { footer: t('about.credits') }),
      ];
    },
  };

  function render() {
    scope.clear();
    nav.replaceChildren(...TABS.map((tab) => {
      const btn = h('button.settings-tab', { type: 'button', role: 'tab', dataset: { tab: tab.id } },
        h('span.settings-tab-icon', { html: icon(tab.icon, 22, 1.7) }),
        h('span', { text: t(`set.tab.${tab.id}`) }));
      btn.classList.toggle('on', tab.id === tabId);
      btn.addEventListener('click', () => { tabId = tab.id; render(); });
      return btn;
    }));
    body.replaceChildren(...withScope(scope, () => builders[tabId]()).filter(Boolean));
    body.scrollTop = 0;
  }

  const unsub = subscribe(() => refreshScope(scope));
  const offTools = tools.onStatus(() => { if (tabId === 'downloads') render(); });
  const sheet = openSheet({
    title: null,
    width: 640,
    className: 'settings-sheet',
    body: [nav, body],
    onClose: () => { unsub(); offTools(); },
  });
  app.info().then((i) => { info = i; if (tabId === 'about' || tabId === 'downloads') render(); else refreshScope(scope); });
  render();
  return sheet;
}

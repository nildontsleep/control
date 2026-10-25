// Réglages : feuille modale à onglets (Général, Apparence, Bibliothèque, Streaming, Téléchargements, À propos).

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t, currentLanguage } from '../i18n.js';
import { settings, update, subscribe, resetAll } from '../settings.js';
import { lib } from '../data.js';
import { ACCENTS } from '../color.js';
import { openSheet, confirmSheet, promptSheet } from './sheet.js';
import { media } from '../media/api.js';
import { toast } from './feedback.js';
import { fmtBytes } from '../format.js';
import { onUpdate, checkNow, updateNow } from '../update/state.js';
import { SCENES } from '../theme/scenes.js';
import { PRESETS, DEFAULT_PRESET, currentLook, matchingPreset, swatchFor } from '../theme/presets.js';
import {
  createScope, withScope, refreshScope, toggle, segmented, popup, swatches, button, row, group, badge, live, sliderRow,
} from './controls.js';

const library = window.control.library;
const downloads = window.control.downloads;
const tools = window.control.tools;
const app = window.control.app;

const B = { blue: '#0a84ff', pink: '#ff375f', green: '#30d158', orange: '#ff9f0a', gray: '#8e8e93', purple: '#bf5af2', teal: '#40c8e0', red: '#ff453a' };

export const SHORTCUTS = [
  [['Espace'], 'kb.playPause'], [['Ctrl', '→'], 'kb.next'], [['Ctrl', '←'], 'kb.prev'], [['→'], 'kb.seekForward'], [['←'], 'kb.seekBack'],
  [['Ctrl', '↑'], 'kb.volUp'], [['Ctrl', '↓'], 'kb.volDown'], [['S'], 'kb.shuffle'], [['R'], 'kb.repeat'], [['M'], 'kb.mute'],
  [['Q'], 'kb.queue'], [['L'], 'kb.lyrics'], [['E'], 'kb.eq'], [['Ctrl', 'I'], 'kb.tags'], [['Entrée'], 'kb.nowPlaying'], [['Ctrl', 'F'], 'kb.search'], [['Alt', '←'], 'kb.back'],
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
    { id: 'appearance', icon: 'palette' },
    { id: 'library', icon: 'hard-drive' },
    { id: 'streaming', icon: 'clapperboard' },
    { id: 'downloads', icon: 'cloud-download' },
    { id: 'about', icon: 'info' },
  ];

  let offDiscord = null;
  let offUpdate = null;


  /** Mises à jour : état en direct, un seul bouton pour télécharger, installer et relancer. */
  function updateGroup() {
    const line = h('div.upd-line');
    const paint = (st) => {
      const phase = st.phase;
      const text = (key, vars) => h('span.upd-text', { text: t(key, vars) });
      let nodes;
      if (phase === 'checking') nodes = [h('span.spinner.small', {}, ...Array.from({ length: 12 }, (_, i) => h('i', { style: { '--i': i } }))), text('upd.checking')];
      else if (phase === 'available') {
        nodes = [
          h('div.upd-info', {}, h('strong', { text: t('upd.available', { v: st.version }) }), h('span', { text: st.manual ? t('upd.manual') : st.size ? fmtBytes(st.size) : '' })),
          h('div.row-buttons', {},
            // Paquet .deb, macOS, lancement depuis les sources : pas d'installation automatique, on renvoie vers la page.
            st.manual
              ? button(t('upd.download'), { icon: 'external-link', kind: 'primary', small: true, onClick: () => app.openExternal(st.page) })
              : button(t('upd.install'), { icon: 'cloud-download', kind: 'primary', small: true, onClick: () => updateNow() }),
            !st.manual && st.page ? button(t('upd.notes'), { icon: 'external-link', small: true, onClick: () => app.openExternal(st.page) }) : null),
        ];
      } else if (phase === 'downloading') {
        const pct = st.size ? Math.min(100, Math.round(((st.received || 0) / st.size) * 100)) : 0;
        nodes = [
          h('div.upd-info', {}, h('strong', { text: t('upd.downloading', { v: st.version }) }), h('span', { text: st.size ? `${fmtBytes(st.received || 0)} / ${fmtBytes(st.size)}` : '' })),
          h('div.upd-bar', {}, h('i', { style: { width: `${pct}%` } })),
        ];
      } else if (phase === 'ready') {
        nodes = [
          h('div.upd-info', {}, h('strong', { text: t('upd.ready', { v: st.version }) }), h('span', { text: st.verified ? t('upd.verified') : '' })),
          button(t('upd.restart'), { icon: 'rotate-cw', kind: 'primary', small: true, onClick: () => updateNow() }),
        ];
      } else if (phase === 'installing') nodes = [h('span.spinner.small', {}, ...Array.from({ length: 12 }, (_, i) => h('i', { style: { '--i': i } }))), text('upd.installing')];
      else if (phase === 'error') {
        nodes = [
          h('div.upd-info', {}, h('strong', { text: t('upd.failed') }), h('span', { text: t(`upd.err.${st.code}`) })),
          button(t('action.retry'), { small: true, onClick: () => checkNow() }),
        ];
      } else {
        nodes = [
          h('div.upd-info', {}, h('strong', { text: phase === 'none' ? t('upd.upToDate') : t('upd.idle') }), h('span', { text: st.checkedAt ? t('upd.checkedAt', { when: new Date(st.checkedAt).toLocaleTimeString(currentLanguage(), { hour: '2-digit', minute: '2-digit' }) }) : '' })),
          button(t('upd.check'), { icon: 'refresh-cw', small: true, onClick: () => checkNow() }),
        ];
      }
      line.replaceChildren(...nodes.filter(Boolean));
    };
    const tokenInput = h('input.text-input.key-input', { type: 'password', value: settings.update?.token || '', placeholder: t('upd.tokenPlaceholder'), spellcheck: 'false', autocomplete: 'off' });
    const saveToken = () => { update({ update: { token: tokenInput.value.trim() } }); checkNow(); };
    tokenInput.addEventListener('change', saveToken);
    tokenInput.addEventListener('keydown', (e) => { if (e.key === 'Enter') saveToken(); });
    // Visible seulement quand GitHub refuse l'accès sans jeton (dépôt privé) ou qu'un jeton est déjà enregistré.
    const tokenRow = row(t('upd.token'), tokenInput, { hint: t('upd.tokenHint'), badge: badge('key-round', B.gray) });
    const base = paint;
    const paintAll = (st) => { base(st); tokenRow.hidden = !(['private', 'token'].includes(st.code) || settings.update?.token); };
    offUpdate?.();
    offUpdate = onUpdate(paintAll);
    return group(t('upd.title'), [
      h('div.row.row-block', {}, line),
      tokenRow,
      row(t('upd.auto'), toggle({ get: () => settings.update?.auto !== false, set: (v) => update({ update: { auto: v } }) }), { hint: t('upd.autoHint'), badge: badge('refresh-cw', B.green) }),
    ]);
  }

  /** Discord : présence enrichie (musique, vidéo, streaming) et état de la connexion. */
  function discordGroup() {
    const dc = window.control.discord;
    const sub = (key) => ({ get: () => settings.discord?.[key] !== false, set: (v) => update({ discord: { [key]: v } }) });
    const status = h('div.row-hint.dc-status');
    const coverLine = h('div.row-hint.dc-cover');
    const input = h('input.text-input.key-input', { type: 'text', value: settings.discord?.clientId || '', placeholder: t('dc.idPlaceholder'), spellcheck: 'false', autocomplete: 'off', inputmode: 'numeric' });
    const paint = (s) => {
      const key = s?.state || 'off';
      status.textContent = key === 'connected' && s.detail ? t('dc.state.connectedAs', { name: s.detail }) : t(`dc.state.${key}`);
      status.dataset.state = key;
      // Où en est la pochette de la lecture en cours : utile quand Discord n'affiche que le logo de control.
      const c = s?.cover;
      coverLine.textContent = c && key === 'connected' ? t(`dc.cover.${c.state}`, { detail: c.detail }) : '';
      coverLine.dataset.state = c?.state || '';
    };
    dc.status().then(paint);
    offDiscord?.();
    offDiscord = dc.onStatus(paint);
    const hook = h('input.text-input.key-input', { type: 'password', value: settings.discord?.coverWebhook || '', placeholder: 'https://discord.com/api/webhooks/…', spellcheck: 'false', autocomplete: 'off' });
    const saveHook = () => {
      const v = hook.value.trim();
      if (v && !/^https:\/\/(?:(?:ptb|canary)\.)?discord(?:app)?\.com\/api\/webhooks\/\d{15,25}\/[\w-]{20,120}$/.test(v)) { toast(t('dc.webhookInvalid'), 'triangle-alert'); return; }
      update({ discord: { coverWebhook: v } });
    };
    hook.addEventListener('change', saveHook);
    hook.addEventListener('keydown', (e) => { if (e.key === 'Enter') saveHook(); });
    const saveId = () => {
      const v = input.value.trim();
      if (v && !/^\d{15,25}$/.test(v)) { status.textContent = t('dc.state.bad-id'); status.dataset.state = 'bad-id'; return; }
      update({ discord: { clientId: v } });
    };
    input.addEventListener('change', saveId);
    input.addEventListener('keydown', (e) => { if (e.key === 'Enter') saveId(); });
    const rows = [
      row(t('dc.enable'), toggle('discord.enabled'), { hint: t('dc.enableHint'), badge: badge('radio', B.purple) }),
      row(t('dc.music'), toggle(sub('music')), { badge: badge('headphones', B.pink) }),
      row(t('dc.video'), toggle(sub('video')), { badge: badge('film', B.orange) }),
      row(t('dc.streaming'), toggle(sub('streaming')), { badge: badge('clapperboard', B.teal) }),
      row(t('dc.hide'), toggle('discord.hideTitles'), { hint: t('dc.hideHint'), badge: badge('eye-off', B.gray) }),
      row(t('dc.covers'), toggle(sub('covers')), { hint: t('dc.coversHint'), badge: badge('image', B.green) }),
      row(t('dc.webhook'), hook, { hint: t('dc.webhookHint'), badge: badge('link', B.blue), className: 'row-stack' }),
      h('div.row.row-block', {}, h('div.key-form', {}, input, button(t('dc.test'), { kind: 'primary', small: true, onClick: async () => { saveId(); status.textContent = t('dc.state.connecting'); paint(await dc.test()); } })), h('div.key-meta', {}, status, coverLine)),
    ];
    return group(t('dc.title'), rows, { footer: t('dc.footer') });
  }

  const builders = {
    general() {
      const langs = ['auto', 'fr', 'en'].map((v) => ({ value: v, label: t(`lang.${v}`) }));
      return [
        group(t('set.interface'), [
          row(t('set.compact'), toggle('compact'), { hint: t('set.compactHint'), badge: badge('list', B.teal) }),
          row(t('set.animations'), toggle('animations'), { badge: badge('wand-sparkles', B.orange) }),
        ]),
        group(t('set.playback'), [
          sliderRow(t('set.crossfade'), 'crossfade', { min: 0, max: 12, step: 1, format: (v) => (v ? t('set.seconds', { n: v }) : t('set.off')) }),
        ], { footer: t('set.crossfadeHint') }),
        discordGroup(),
        group(t('set.language'), [
          row(t('set.language'), popup('language', langs), { badge: badge('languages', B.blue) }),
        ], { footer: t('set.languageHint') }),
      ];
    },

    appearance() {
      const accents = Object.entries(ACCENTS).map(([value, c]) => ({ value, color: c.dark, label: t(`accent.${value}`) }));
      const pct = (v) => `${Math.round(v)} %`;
      const scenes = SCENES.map((value) => ({ value, label: t(`scene.${value}`) }));

      const grid = h('div.amb-grid');
      const tile = (key, name, desc, sw, onDelete) => {
        const el = h('div.amb-tile', { role: 'button', tabindex: '0', dataset: { amb: key } },
          h('span.amb-swatch', { style: { '--from': sw.from, '--to': sw.to, '--acc': sw.accent } }, h('i'), h('b')),
          h('span.amb-name', { text: name }),
          desc ? h('span.amb-desc', { text: desc }) : null,
          onDelete ? h('button.amb-del.icon-btn', { type: 'button', 'data-tip': t('common.remove'), html: icon('x', 13, 2.2), onclick: (e) => { e.stopPropagation(); onDelete(); } }) : null);
        const apply = () => update({ ...(key === DEFAULT_PRESET ? PRESETS[0].patch : (customs().find((c) => c.key === key) || PRESETS.find((p) => p.key === key)).patch) });
        el.addEventListener('click', apply);
        el.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); apply(); } });
        return el;
      };
      const customs = () => (Array.isArray(settings.customAmbiences) ? settings.customAmbiences : []);
      const paintGrid = () => {
        const on = matchingPreset(settings, customs());
        const items = [
          ...PRESETS.map((p) => tile(p.key, t(`amb.${p.key}`), t(`amb.${p.key}.desc`), p.swatch)),
          ...customs().map((c) => tile(c.key, c.name, t('amb.customDesc'), swatchFor(c.patch), () => {
            update({ customAmbiences: customs().filter((x) => x.key !== c.key) });
            paintGrid();
          })),
        ];
        grid.replaceChildren(...items);
        for (const el of grid.children) el.classList.toggle('on', el.dataset.amb === on);
      };
      live(paintGrid);

      const saveCurrent = async () => {
        const name = await promptSheet({ title: t('amb.saveTitle'), message: t('amb.saveText'), placeholder: t('amb.namePlaceholder'), confirmLabel: t('common.save') });
        if (!name) return;
        const key = `custom-${Date.now().toString(36)}`;
        update({ customAmbiences: [...customs(), { key, name: name.slice(0, 40), patch: currentLook(settings) }] });
        toast(t('amb.saved', { name }), 'palette');
      };
      const resetLook = () => update({ ...PRESETS.find((p) => p.key === DEFAULT_PRESET).patch });

      const oledRow = sliderRow(t('set.oledLevel'), 'oledLevel', { min: 0, max: 100, step: 5, format: (v) => (v <= 5 ? t('amb.oledDeep') : v >= 95 ? t('amb.oledSoft') : pct(v)) });
      live(() => { oledRow.hidden = settings.theme !== 'oled'; });

      return [
        group(t('set.ambiances'), [
          h('div.row.row-block', {}, grid),
          h('div.row.row-block', {}, h('div.row-buttons', {},
            button(t('amb.save'), { icon: 'bookmark-plus', small: true, onClick: saveCurrent }),
            button(t('amb.reset'), { icon: 'rotate-ccw', small: true, onClick: resetLook }))),
        ], { footer: t('amb.hint') }),
        group(t('set.colors'), [
          row(t('set.theme'), segmented('theme', [
            { value: 'auto', label: t('theme.auto'), icon: 'circle-dashed' },
            { value: 'light', label: t('theme.light'), icon: 'sun' },
            { value: 'dark', label: t('theme.dark'), icon: 'moon' },
            { value: 'oled', label: t('theme.oled'), icon: 'moon-star' },
          ], { compact: true, iconsOnly: true }), { badge: badge('palette', B.purple) }),
          oledRow,
          row(t('set.accent'), swatches('accent', accents, { custom: { path: 'customAccent', label: t('accent.custom') } }), { className: 'row-stack' }),
          row(t('set.ambient'), toggle('ambient'), { hint: t('set.ambientHint'), badge: badge('sparkles', B.pink) }),
        ]),
        group(t('set.scene'), [
          row(t('set.sceneKind'), popup('scene', scenes), { badge: badge('wand-sparkles', B.orange) }),
          sliderRow(t('set.sceneSpeed'), 'sceneSpeed', { min: 20, max: 250, step: 5, format: pct }),
          sliderRow(t('set.sceneBlur'), 'sceneBlur', { min: 0, max: 14, step: 1, format: (v) => (v ? `${v} px` : t('set.off')) }),
        ], { footer: t('set.sceneHint') }),
        group(t('set.shape'), [
          sliderRow(t('set.radius'), 'radius', { min: 20, max: 180, step: 1, format: pct }),
          sliderRow(t('set.translucency'), 'translucency', { min: 30, max: 170, step: 1, format: pct }),
          sliderRow(t('set.uiScale'), 'uiScale', { min: 80, max: 130, step: 5, format: pct }),
        ], { footer: t('set.shapeHint') }),
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

    streaming() {
      const input = h('input.text-input.key-input', { type: 'password', value: settings.media?.tmdbKey || '', placeholder: t('key.placeholder'), spellcheck: 'false', autocomplete: 'off' });
      const status = h('div.key-status');
      const reveal = h('button.link-btn', { type: 'button', text: t('key.show'), onclick: () => {
        const hidden = input.type === 'password';
        input.type = hidden ? 'text' : 'password';
        reveal.textContent = hidden ? t('key.hide') : t('key.show');
      } });
      const saveKey = async () => {
        const key = input.value.trim();
        if (!key) {
          update({ media: { tmdbKey: '' } });
          status.textContent = t('key.removed');
          status.className = 'key-status';
          return;
        }
        status.textContent = t('key.testing');
        status.className = 'key-status';
        try {
          await media.testKey(key);
          update({ media: { tmdbKey: key } });
          status.textContent = t('key.saved');
          status.className = 'key-status ok';
        } catch (err) {
          status.textContent = err.code === 'auth' ? t('key.invalid') : t('err.network');
          status.className = 'key-status bad';
        }
      };
      input.addEventListener('keydown', (e) => { if (e.key === 'Enter') saveKey(); });
      const servers = ['https://embed.vidrift.net', 'https://embed.vidrift.in'].map((v) => ({ value: v, label: v.replace('https://', '') }));
      return [
        group(t('set.tmdb'), [
          h('div.row.row-block', {}, h('div.key-form', {}, input, button(t('key.test'), { kind: 'primary', small: true, onClick: saveKey })), h('div.key-meta', {}, status, reveal)),
        ], { footer: t('set.tmdbHint'), action: button(t('key.get'), { small: true, kind: 'plain', icon: 'external-link', onClick: () => app.openExternal('https://www.themoviedb.org/settings/api') }) }),
        group(t('set.animeGroup'), [
          row(t('set.animeTitles'), segmented('media.animeTitles', [{ value: 'romaji', label: 'Romaji' }, { value: 'english', label: t('lang.en') }], { compact: true }), { badge: badge('languages', B.blue) }),
          row(t('set.dubDefault'), toggle('media.dub'), { hint: t('set.dubHint'), badge: badge('captions', B.purple) }),
        ]),
        group(t('set.playerGroup'), [
          row(t('set.server'), popup('media.streamBase', servers), { hint: t('set.serverHint'), badge: badge('globe', B.teal) }),
        ], { footer: t('set.streamNote') }),
        group(t('set.data'), [
          h('div.row.row-block', {}, h('div.row-buttons', {},
            button(t('set.clearCache'), { icon: 'refresh-cw', small: true, onClick: async () => { await media.clearCache(); toast(t('set.cacheCleared'), 'check'); } }),
            button(t('hist.clear'), { icon: 'trash-2', small: true, kind: 'danger', onClick: async () => {
              const { ok } = await confirmSheet({ title: t('hist.clearTitle'), message: t('hist.clearText'), confirmLabel: t('hist.clear'), destructive: true });
              if (!ok) return;
              await window.control.watch.clear();
              toast(t('hist.cleared'), 'trash-2');
            } }))),
        ]),
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
      const edition = !info ? '' : info.portable ? t('about.portable') : info.appimage ? t('about.appimage') : t('about.installed');
      return [
        h('div.about-card', {},
          h('img.about-icon', { src: 'assets/icon.png', alt: '' }),
          h('div.about-name', { text: 'control' }),
          h('div.about-tagline', { text: t('about.tagline') }),
          h('div.about-meta', { text: `${t('about.version', { v: info?.version || '0.1.0' })}${edition ? ` · ${edition}` : ''}` })),
        updateGroup(),
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
    width: 680,
    className: 'settings-sheet',
    body: [nav, body],
    onClose: () => { unsub(); offTools(); offDiscord?.(); offUpdate?.(); },
  });
  app.info().then((i) => { info = i; if (tabId === 'about' || tabId === 'downloads') render(); else refreshScope(scope); });
  render();
  return sheet;
}

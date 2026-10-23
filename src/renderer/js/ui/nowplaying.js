// Écran « À l’écoute » : pochette, vidéo, commandes, file d’attente et égaliseur, sur fond de pochette floutée.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { fmtTime } from '../format.js';
import { settings, update, subscribe } from '../settings.js';
import { player, on as onPlayer, EQ_BANDS, EQ_PRESETS } from '../player.js';
import { navigate } from '../router.js';
import { coverEl, coverUrl } from './cover.js';
import { createTrackList } from './tracklist.js';
import { createLyricsPanel } from './lyrics-panel.js';
import { openMenu } from './menu.js';
import { trafficLights } from './traffic.js';
import { createScope, withScope, refreshScope, popup, toggle, segmented } from './controls.js';

const win = window.control.win;

function paintRange(input, pct) {
  input.style.setProperty('--from', '0%');
  input.style.setProperty('--to', `${pct}%`);
}

export function createNowPlaying(root) {
  const scope = createScope();
  const bg = root.querySelector('.np-bg');
  const art = root.querySelector('#npArt');
  const top = root.querySelector('.np-top');
  const info = root.querySelector('#npInfo');
  const seekBox = root.querySelector('#npSeek');
  const controls = root.querySelector('#npControls');
  const side = root.querySelector('#npPanel');
  const tabs = root.querySelector('#npTabs');
  const video = player.media;
  const stage = root.querySelector('.np-stage');
  let tab = ['queue', 'lyrics', 'eq'].includes(settings.npTab) ? settings.npTab : 'queue';
  let list = null;
  const lyrics = createLyricsPanel(side);
  let open = false;
  let idleTimer = null;

  // ─── En-tête ───────────────────────────────────────────────────────────────
  const closeBtn = h('button.np-close', { type: 'button', 'data-tip': t('np.close'), 'data-kbd': 'Échap', html: icon('chevron-down', 20, 2.2), onclick: () => api.close() });
  const fsBtn = h('button.np-icon-btn', { type: 'button', 'data-tip': t('np.fullscreen'), html: icon('maximize', 17, 2), onclick: () => {
    win.setFullscreen(!root.classList.contains('fullscreen-mode'));
  } });
  top.replaceChildren(trafficLights(), closeBtn, h('div.np-top-title', { text: t('np.title') }), fsBtn);

  // ─── Informations ──────────────────────────────────────────────────────────
  const titleEl = h('h1.np-title');
  const artistEl = h('button.np-artist', { type: 'button' });
  const albumEl = h('div.np-album');
  info.replaceChildren(titleEl, artistEl, albumEl);
  artistEl.addEventListener('click', () => {
    const track = player.track;
    if (track?.albumKey) {
      api.close();
      navigate('album', track.albumKey);
    }
  });

  // ─── Progression ───────────────────────────────────────────────────────────
  const cur = h('span.np-time', { text: '0:00' });
  const remaining = h('span.np-time.right', { text: '-0:00' });
  const seek = h('input.slider.seek.np-seek-input', { type: 'range', min: '0', max: '1', step: '0.05', value: '0' });
  let seeking = false;
  seek.addEventListener('pointerdown', () => { seeking = true; });
  seek.addEventListener('input', () => {
    cur.textContent = fmtTime(Number(seek.value));
    remaining.textContent = `-${fmtTime(Number(seek.max) - Number(seek.value))}`;
    paintRange(seek, (Number(seek.value) / Math.max(0.001, Number(seek.max))) * 100);
  });
  seek.addEventListener('change', () => { player.seek(Number(seek.value)); seeking = false; });
  seek.addEventListener('pointerup', () => { seeking = false; });
  seekBox.replaceChildren(seek, h('div.np-times', {}, cur, remaining));

  // ─── Commandes ─────────────────────────────────────────────────────────────
  const ctl = (ic, tip, fn, cls = '') => h(`button.np-btn${cls}`, { type: 'button', 'data-tip': tip, html: icon(ic, 22, 1.9), onclick: (e) => { fn(); e.currentTarget.blur(); } });
  const shuffle = ctl('shuffle', t('player.shuffle'), () => player.toggleShuffle(), '.toggle');
  const prev = ctl('skip-back', t('player.prev'), () => player.prev());
  const play = h('button.np-play', { type: 'button', onclick: (e) => { player.toggle(); e.currentTarget.blur(); } });
  const next = ctl('skip-forward', t('player.next'), () => player.next());
  const repeat = ctl('repeat', t('player.repeat'), () => player.cycleRepeat(), '.toggle');
  const volIcon = h('button.np-icon-btn', { type: 'button', 'data-tip': t('player.mute'), onclick: (e) => { player.toggleMute(); e.currentTarget.blur(); } });
  const volume = h('input.slider.volume', { type: 'range', min: '0', max: '1', step: '0.01' });
  volume.addEventListener('input', () => player.setVolume(Number(volume.value)));
  const rate = withScope(scope, () => popup({ get: () => String(settings.rate), set: (v) => player.setRate(Number(v)) },
    ['0.75', '1', '1.25', '1.5', '2'].map((v) => ({ value: v, label: `${v}×` }))));
  controls.replaceChildren(
    h('div.np-transport', {}, shuffle, prev, play, next, repeat),
    h('div.np-extra', {}, volIcon, volume, h('span.spacer'), rate));

  // ─── Barre de lecture sur l'image (vidéos) ─────────────────────────────────
  // Une vraie barre de lecteur vidéo : progression, ±10 s, précédent / suivant, volume, vitesse, image dans l'image, plein écran.
  const vseek = h('input.slider.seek.vseek', { type: 'range', min: '0', max: '1', step: '0.05', value: '0', 'aria-label': t('player.seek') });
  const vcur = h('span.vtime', { text: '0:00' });
  const vrem = h('span.vtime.right', { text: '-0:00' });
  let vseeking = false;
  vseek.addEventListener('pointerdown', () => { vseeking = true; });
  vseek.addEventListener('input', () => {
    vcur.textContent = fmtTime(Number(vseek.value));
    vrem.textContent = `-${fmtTime(Number(vseek.max) - Number(vseek.value))}`;
    paintRange(vseek, (Number(vseek.value) / Math.max(0.001, Number(vseek.max))) * 100);
  });
  vseek.addEventListener('change', () => { player.seek(Number(vseek.value)); vseeking = false; });
  vseek.addEventListener('pointerup', () => { vseeking = false; });

  const vbtn = (ic, tip, fn, cls = '') => h(`button.vbtn${cls}`, { type: 'button', 'data-tip': tip, html: icon(ic, 20, 1.9), onclick: (e) => { fn(e); e.currentTarget.blur(); } });
  const vPrev = vbtn('skip-back', t('player.prev'), () => player.prev());
  const vBack = vbtn('rotate-ccw', t('video.back10'), () => player.seekBy(-10));
  const vPlay = h('button.vplay', { type: 'button', onclick: (e) => { player.toggle(); e.currentTarget.blur(); } });
  const vFwd = vbtn('rotate-cw', t('video.fwd10'), () => player.seekBy(10));
  const vNext = vbtn('skip-forward', t('player.next'), () => player.next());
  for (const el of [vBack, vFwd]) el.append(h('span.skip-n', { text: '10' }));
  const vVolIcon = vbtn('volume-2', t('player.mute'), () => player.toggleMute());
  const vVolume = h('input.slider.volume.vvolume', { type: 'range', min: '0', max: '1', step: '0.01', 'aria-label': t('player.volume') });
  vVolume.addEventListener('input', () => player.setVolume(Number(vVolume.value)));
  const vRate = h('button.vchip', { type: 'button', 'data-tip': t('player.rate'), onclick: (e) => {
    openMenu(e.currentTarget, [0.5, 0.75, 1, 1.25, 1.5, 2].map((v) => ({ label: `${v}×`, checked: settings.rate === v, action: () => player.setRate(v) })), { align: 'end', placement: 'above' });
  } });
  const vPip = document.pictureInPictureEnabled
    ? vbtn('picture-in-picture-2', t('video.pip'), () => {
      if (document.pictureInPictureElement) document.exitPictureInPicture().catch(() => {});
      else video.requestPictureInPicture().catch(() => {});
    })
    : null;
  const vFs = vbtn('maximize', t('np.fullscreen'), () => win.setFullscreen(!root.classList.contains('fullscreen-mode')));
  const vbar = h('div.vbar', {},
    h('div.vbar-seek', {}, vcur, vseek, vrem),
    h('div.vbar-row', {},
      h('div.vbar-side', {}, vVolIcon, vVolume),
      h('div.vbar-main', {}, vPrev, vBack, vPlay, vFwd, vNext),
      h('div.vbar-side.end', {}, vRate, vPip, vFs)));
  const vtopTitle = h('div.vtop-title');
  const vtop = h('div.vtop', {},
    h('button.vbtn', { type: 'button', 'data-tip': t('video.exitFullscreen'), html: icon('minimize', 20, 1.9), onclick: () => win.setFullscreen(false) }),
    vtopTitle);
  stage.append(vtop, vbar);
  // Un clic sur la barre ne doit pas mettre la vidéo en pause (le clic sur l'image le fait).
  for (const el of [vbar, vtop]) {
    el.addEventListener('click', (e) => e.stopPropagation());
    el.addEventListener('dblclick', (e) => e.stopPropagation());
  }

  // ─── Panneaux ──────────────────────────────────────────────────────────────
  function destroyList() {
    list?.destroy();
    list = null;
  }

  function renderQueue() {
    destroyList();
    const queue = player.state.queue;
    const head = h('div.panel-head.end', {}, queue.length > player.state.index + 1
      ? h('button.btn.btn-plain.btn-small', { type: 'button', text: t('queue.clear'), onclick: () => player.clearUpcoming() })
      : null);
    if (!queue.length) {
      side.replaceChildren(head, h('div.panel-empty', { text: t('queue.empty') }));
      return;
    }
    const items = queue.map((track) => ({ track }));
    list = createTrackList({
      items,
      layout: 'queue',
      scroller: side,
      context: { removeFromPlaylist: (rows) => rows.map((r) => queue.indexOf(r.track)).sort((a, b) => b - a).forEach((i) => player.removeAt(i)), removeLabel: t('queue.remove') },
      onPlay: (i) => player.jumpTo(i),
      onMove: (from, to) => {
        const i = from[0];
        player.move(i, to > i ? to - 1 : to);
      },
    });
    side.replaceChildren(head, list.el);
    requestAnimationFrame(() => {
      const row = list.el.querySelector('.tl-row.current');
      if (row) side.scrollTop = Math.max(0, row.offsetTop + list.el.offsetTop - 70);
    });
  }

  function renderEq() {
    destroyList();
    const presetNames = Object.keys(EQ_PRESETS);
    const presetPopup = popup({
      get: () => settings.eq.preset,
      set: (v) => update({ eq: { preset: v, gains: [...EQ_PRESETS[v]] } }),
    }, [...presetNames.map((p) => ({ value: p, label: t(`eq.${p}`) })), ...(settings.eq.preset === 'custom' ? [{ value: 'custom', label: t('eq.custom') }] : [])]);
    const sliders = EQ_BANDS.map((freq, i) => {
      const input = h('input.eq-slider', { type: 'range', min: '-12', max: '12', step: '0.5', value: String(settings.eq.gains[i] || 0), orient: 'vertical' });
      const value = h('span.eq-value', { text: '0' });
      const paint = () => { value.textContent = `${Number(input.value) > 0 ? '+' : ''}${Number(input.value)}`; };
      input.addEventListener('input', () => {
        const gains = [...settings.eq.gains];
        gains[i] = Number(input.value);
        paint();
        update({ eq: { gains, preset: 'custom' } });
      });
      input.addEventListener('dblclick', () => {
        input.value = '0';
        input.dispatchEvent(new Event('input'));
      });
      paint();
      return h('div.eq-band', {}, value, input, h('span.eq-freq', { text: freq >= 1000 ? `${freq / 1000}k` : String(freq) }));
    });
    side.replaceChildren(
      h('div.panel-head', {}, h('h2', { text: t('eq.title') }), toggle('eq.enabled')),
      h('div.eq-presets', {}, presetPopup),
      h('div.eq-bands', {}, ...sliders),
      h('p.eq-hint', { text: t('eq.hint') }));
  }

  function renderLyrics() {
    destroyList();
    side.replaceChildren(lyrics.el);
    lyrics.show(player.track);
  }

  function renderPanel() {
    if (tab === 'queue') renderQueue();
    else if (tab === 'lyrics') renderLyrics();
    else withScope(scope, renderEq);
  }

  const tabControl = withScope(scope, () => segmented({
    get: () => tab,
    set: (v) => { tab = v; update({ npTab: v }); renderPanel(); refreshScope(scope); },
  }, [
    { value: 'queue', label: t('queue.title'), icon: 'list-music' },
    { value: 'lyrics', label: t('lyr.title'), icon: 'mic-vocal' },
    { value: 'eq', label: t('eq.title'), icon: 'sliders-horizontal' },
  ], { compact: true }));
  tabs.replaceChildren(tabControl);

  // ─── Rendu ─────────────────────────────────────────────────────────────────
  function paintTrack() {
    const track = player.track;
    titleEl.textContent = track ? track.title : t('player.nothing');
    artistEl.textContent = track?.artist || '';
    albumEl.textContent = track ? [track.album, track.year].filter(Boolean).join(' · ') : '';
    art.replaceChildren(track ? coverEl(track, { className: 'np-cover' }) : h('div.cover.np-cover', {}, h('span.cover-icon', { html: icon('music-2', 60, 1.2) })));
    const url = track?.cover ? coverUrl(track.cover) : null;
    bg.style.backgroundImage = url ? `url(${url})` : 'none';
    root.classList.toggle('has-video', track?.kind === 'video');
    vtopTitle.textContent = track?.title || '';
    root.classList.toggle('no-cover', !track?.cover && track?.kind !== 'video');
    remaining.textContent = `-${fmtTime(player.duration)}`;
    seek.max = String(player.duration || 1);
    if (open && tab === 'queue') renderQueue();
  }

  function paintState() {
    const playing = player.playing;
    play.innerHTML = icon(playing ? 'pause' : 'play', 30, 0);
    play.dataset.tip = playing ? t('player.pause') : t('player.play');
    vPlay.innerHTML = icon(playing ? 'pause' : 'play', 26, 0);
    vPlay.dataset.tip = playing ? t('player.pause') : t('player.play');
    root.classList.toggle('is-playing', playing);
    seek.max = String(player.duration || 1);
  }

  function paintSettings() {
    shuffle.classList.toggle('on', !!settings.shuffle);
    repeat.classList.toggle('on', settings.repeat !== 'off');
    repeat.innerHTML = icon(settings.repeat === 'one' ? 'repeat-1' : 'repeat', 22, 1.9);
    volume.value = String(settings.muted ? 0 : settings.volume);
    paintRange(volume, (settings.muted ? 0 : settings.volume) * 100);
    volIcon.innerHTML = icon(settings.muted || settings.volume === 0 ? 'volume-x' : settings.volume < 0.4 ? 'volume-1' : 'volume-2', 18, 1.9);
    vVolume.value = String(settings.muted ? 0 : settings.volume);
    paintRange(vVolume, (settings.muted ? 0 : settings.volume) * 100);
    vVolIcon.innerHTML = icon(settings.muted || settings.volume === 0 ? 'volume-x' : settings.volume < 0.4 ? 'volume-1' : 'volume-2', 20, 1.9);
    vRate.textContent = `${settings.rate}×`;
  }

  function loop() {
    if (!open) return;
    requestAnimationFrame(loop);
    if (!player.track) return;
    const time = player.time;
    const duration = player.duration || 1;
    if (!seeking) {
      seek.value = String(time);
      cur.textContent = fmtTime(time);
      remaining.textContent = `-${fmtTime(Math.max(0, duration - time))}`;
      paintRange(seek, (time / duration) * 100);
    }
    if (!vseeking && root.classList.contains('has-video')) {
      vseek.max = String(duration);
      vseek.value = String(time);
      vcur.textContent = fmtTime(time);
      vrem.textContent = `-${fmtTime(Math.max(0, duration - time))}`;
      paintRange(vseek, (time / duration) * 100);
    }
    if (tab === 'lyrics') lyrics.tick(time);
  }

  // ─── Ouverture / fermeture ─────────────────────────────────────────────────
  const api = {
    open(which) {
      if (which) tab = which;
      open = true;
      root.classList.add('open');
      root.setAttribute('aria-hidden', 'false');
      document.body.classList.add('np-open');
      paintTrack();
      paintState();
      paintSettings();
      renderPanel();
      refreshScope(scope);
      requestAnimationFrame(loop);
    },
    close() {
      open = false;
      root.classList.remove('open', 'idle');
      root.setAttribute('aria-hidden', 'true');
      document.body.classList.remove('np-open');
      destroyList();
      if (root.classList.contains('fullscreen-mode')) win.setFullscreen(false);
    },
    toggle() {
      if (open) api.close();
      else api.open();
    },
    get isOpen() {
      return open;
    },
    setFullscreen(on) {
      root.classList.toggle('fullscreen-mode', !!on);
      fsBtn.innerHTML = icon(on ? 'minimize' : 'maximize', 17, 2);
      vFs.innerHTML = icon(on ? 'minimize' : 'maximize', 20, 1.9);
      vFs.dataset.tip = on ? t('video.exitFullscreen') : t('np.fullscreen');
    },
    get isFullscreen() {
      return root.classList.contains('fullscreen-mode');
    },
  };

  // Vidéo : clic = lecture/pause, double-clic = plein écran, interface qui se masque.
  video.addEventListener('click', () => player.toggle());
  video.addEventListener('dblclick', () => win.setFullscreen(!root.classList.contains('fullscreen-mode')));
  const wake = () => {
    root.classList.remove('idle');
    clearTimeout(idleTimer);
    if (root.classList.contains('has-video') && player.playing) {
      idleTimer = setTimeout(() => root.classList.add('idle'), 2800);
    }
  };
  root.addEventListener('pointermove', wake);
  root.addEventListener('pointerdown', wake);

  onPlayer('track', () => { paintTrack(); if (open && tab === 'lyrics') lyrics.show(player.track); });
  onPlayer('state', () => { paintState(); wake(); });
  onPlayer('queue', () => { if (open && tab === 'queue') renderQueue(); });
  onPlayer('settings', paintSettings);
  subscribe(() => refreshScope(scope));
  paintTrack();
  paintState();
  paintSettings();
  return api;
}

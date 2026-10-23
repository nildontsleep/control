// Écran « À l’écoute » : pochette, vidéo, commandes, file d’attente et égaliseur, sur fond de pochette floutée.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { fmtTime } from '../format.js';
import { settings, update, subscribe } from '../settings.js';
import { player, on as onPlayer, getAnalyser, EQ_BANDS, EQ_PRESETS } from '../player.js';
import { navigate } from '../router.js';
import { coverEl, coverUrl } from './cover.js';
import { createTrackList } from './tracklist.js';
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
  const viz = root.querySelector('#npViz');
  const top = root.querySelector('.np-top');
  const info = root.querySelector('#npInfo');
  const seekBox = root.querySelector('#npSeek');
  const controls = root.querySelector('#npControls');
  const side = root.querySelector('#npPanel');
  const tabs = root.querySelector('#npTabs');
  const video = player.media;
  let tab = 'queue';
  let list = null;
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

  function renderPanel() {
    if (tab === 'queue') renderQueue();
    else withScope(scope, renderEq);
  }

  const tabControl = withScope(scope, () => segmented({
    get: () => tab,
    set: (v) => { tab = v; renderPanel(); refreshScope(scope); },
  }, [
    { value: 'queue', label: t('queue.title'), icon: 'list-music' },
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
    root.classList.toggle('no-cover', !track?.cover && track?.kind !== 'video');
    remaining.textContent = `-${fmtTime(player.duration)}`;
    seek.max = String(player.duration || 1);
    if (open && tab === 'queue') renderQueue();
  }

  function paintState() {
    const playing = player.playing;
    play.innerHTML = icon(playing ? 'pause' : 'play', 30, 0);
    play.dataset.tip = playing ? t('player.pause') : t('player.play');
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
  }

  // Visualiseur discret au bas de l'écran.
  const vctx = viz.getContext('2d');
  const bins = new Uint8Array(128);
  function drawViz() {
    if (!open) return;
    requestAnimationFrame(drawViz);
    const analyser = getAnalyser();
    const w = viz.clientWidth;
    const hgt = viz.clientHeight;
    const dpr = window.devicePixelRatio || 1;
    if (viz.width !== Math.round(w * dpr)) {
      viz.width = Math.round(w * dpr);
      viz.height = Math.round(hgt * dpr);
    }
    vctx.clearRect(0, 0, viz.width, viz.height);
    if (!analyser || !player.playing || !settings.animations) return;
    analyser.getByteFrequencyData(bins);
    const n = 48;
    const gap = 6 * dpr;
    const bw = (viz.width - gap * (n - 1)) / n;
    const accent = getComputedStyle(document.documentElement).getPropertyValue('--accent-rgb').trim().replace(/ /g, ',') || '255,55,95';
    for (let i = 0; i < n; i++) {
      const v = bins[Math.floor((i / n) ** 1.6 * 100)] / 255;
      const bh = Math.max(4 * dpr, v * viz.height * 0.72);
      vctx.fillStyle = `rgba(${accent},${0.1 + v * 0.22})`;
      const x = i * (bw + gap);
      const r = Math.min(bw / 2, 6 * dpr);
      vctx.beginPath();
      vctx.roundRect(x, viz.height - bh, bw, bh, [r, r, 0, 0]);
      vctx.fill();
    }
  }

  function loop() {
    if (!open) return;
    requestAnimationFrame(loop);
    if (seeking || !player.track) return;
    const time = player.time;
    const duration = player.duration || 1;
    seek.value = String(time);
    cur.textContent = fmtTime(time);
    remaining.textContent = `-${fmtTime(Math.max(0, duration - time))}`;
    paintRange(seek, (time / duration) * 100);
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
      requestAnimationFrame(drawViz);
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

  onPlayer('track', () => { paintTrack(); });
  onPlayer('state', () => { paintState(); wake(); });
  onPlayer('queue', () => { if (open && tab === 'queue') renderQueue(); });
  onPlayer('settings', paintSettings);
  subscribe(() => refreshScope(scope));
  paintTrack();
  paintState();
  paintSettings();
  return api;
}

// Barre de lecture flottante en verre : morceau en cours, transport, progression, volume.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { fmtTime } from '../format.js';
import { settings } from '../settings.js';
import { player, on as onPlayer } from '../player.js';
import { coverEl } from './cover.js';
import { navigate } from '../router.js';

function paintRange(input, pct) {
  input.style.setProperty('--from', '0%');
  input.style.setProperty('--to', `${pct}%`);
}

export function createPlayerBar(root, { openNowPlaying, openQueue }) {
  const art = h('button.pb-art', { type: 'button', 'data-tip': t('np.open') });
  const title = h('div.pb-title');
  const sub = h('button.pb-sub', { type: 'button' });
  const info = h('div.pb-info', {}, title, sub);

  const btn = (name, ic, tip, onClick, extra = '') => h(`button.pb-btn${extra}`, { type: 'button', 'data-tip': tip, html: icon(ic, 18, 1.9), onclick: (e) => { onClick(e); e.currentTarget.blur(); } });
  const shuffle = btn('shuffle', 'shuffle', t('player.shuffle'), () => player.toggleShuffle(), '.toggle');
  const prev = btn('prev', 'skip-back', t('player.prev'), () => player.prev(), '.skip');
  const playBtn = h('button.pb-play', { type: 'button', 'data-tip': t('player.play'), 'data-kbd': 'Espace', onclick: (e) => { player.toggle(); e.currentTarget.blur(); } });
  const next = btn('next', 'skip-forward', t('player.next'), () => player.next(), '.skip');
  const repeat = btn('repeat', 'repeat', t('player.repeat'), () => player.cycleRepeat(), '.toggle');

  const cur = h('span.pb-time', { text: '0:00' });
  const total = h('span.pb-time.right', { text: '0:00' });
  const seek = h('input.slider.seek', { type: 'range', min: '0', max: '1', step: '0.05', value: '0', 'aria-label': t('player.seek') });
  let seeking = false;
  seek.addEventListener('pointerdown', () => { seeking = true; });
  seek.addEventListener('input', () => {
    cur.textContent = fmtTime(Number(seek.value));
    paintRange(seek, (Number(seek.value) / Math.max(0.001, Number(seek.max))) * 100);
  });
  seek.addEventListener('change', () => {
    player.seek(Number(seek.value));
    seeking = false;
  });
  seek.addEventListener('pointerup', () => { seeking = false; });

  const volIcon = h('button.pb-btn', { type: 'button', 'data-tip': t('player.mute'), onclick: (e) => { player.toggleMute(); e.currentTarget.blur(); } });
  const volume = h('input.slider.volume', { type: 'range', min: '0', max: '1', step: '0.01', value: String(settings.volume), 'aria-label': t('player.volume') });
  volume.addEventListener('input', () => player.setVolume(Number(volume.value)));
  volume.addEventListener('wheel', (e) => {
    e.preventDefault();
    player.setVolume(settings.volume - Math.sign(e.deltaY) * 0.05);
  }, { passive: false });

  const queueBtn = btn('queue', 'list-music', t('player.queue'), () => openQueue(), '.toggle');
  const expand = btn('expand', 'maximize-2', t('np.open'), () => openNowPlaying());

  root.replaceChildren(
    h('div.pb-left', {}, art, info),
    h('div.pb-center', {},
      h('div.pb-transport', {}, shuffle, prev, playBtn, next, repeat),
      h('div.pb-seek', {}, cur, seek, total)),
    h('div.pb-right', {}, queueBtn, volIcon, volume, expand));

  art.addEventListener('click', () => openNowPlaying());
  info.addEventListener('click', (e) => { if (!e.target.closest('.pb-sub')) openNowPlaying(); });
  sub.addEventListener('click', (e) => {
    e.stopPropagation();
    const track = player.track;
    if (track?.albumKey) navigate('album', track.albumKey);
  });

  function paintTrack() {
    const track = player.track;
    root.classList.toggle('no-track', !track);
    art.replaceChildren(track ? coverEl(track, { small: true }) : h('div.cover.placeholder', {}, h('span.cover-icon', { html: icon('music-2', 20, 1.6) })));
    title.textContent = track ? track.title : t('player.nothing');
    sub.textContent = track ? [track.artist, track.album].filter(Boolean).join(' — ') : t('player.nothingSub');
    total.textContent = fmtTime(player.duration);
    seek.max = String(player.duration || 1);
  }

  function paintState() {
    const playing = player.playing;
    playBtn.innerHTML = icon(playing ? 'pause' : 'play', 22, 0);
    playBtn.dataset.tip = playing ? t('player.pause') : t('player.play');
    root.classList.toggle('playing', playing);
    root.classList.toggle('loading', player.state.loading && playing);
    total.textContent = fmtTime(player.duration);
    seek.max = String(player.duration || 1);
  }

  function paintSettings() {
    shuffle.classList.toggle('on', !!settings.shuffle);
    repeat.classList.toggle('on', settings.repeat !== 'off');
    repeat.innerHTML = icon(settings.repeat === 'one' ? 'repeat-1' : 'repeat', 18, 1.9);
    volume.value = String(settings.muted ? 0 : settings.volume);
    paintRange(volume, (settings.muted ? 0 : settings.volume) * 100);
    const level = settings.muted || settings.volume === 0 ? 'volume-x' : settings.volume < 0.4 ? 'volume-1' : 'volume-2';
    volIcon.innerHTML = icon(level, 18, 1.9);
  }

  function loop() {
    requestAnimationFrame(loop);
    if (seeking || !player.track) return;
    const time = player.time;
    const duration = player.duration || 1;
    seek.value = String(time);
    cur.textContent = fmtTime(time);
    paintRange(seek, (time / duration) * 100);
  }

  onPlayer('track', () => { paintTrack(); paintState(); });
  onPlayer('state', paintState);
  onPlayer('settings', paintSettings);
  paintTrack();
  paintState();
  paintSettings();
  requestAnimationFrame(loop);

  return { rerender: () => { paintTrack(); paintState(); paintSettings(); } };
}

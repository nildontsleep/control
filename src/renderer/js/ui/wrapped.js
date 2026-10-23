// Récap annuel : une suite de cartes plein écran (musique, habitudes, films), et une image à enregistrer.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t, currentLanguage } from '../i18n.js';
import { count } from '../format.js';
import { toast } from './feedback.js';
import { coverUrl, gradientFor } from './cover.js';
import { fmtDuration } from '../stats/format.js';

const api = window.control.stats;

// Un décor différent par carte (dégradés qui se fondent d'une carte à l'autre).
const PALETTES = [
  ['#ff375f', '#7a3cff', '#ff9f0a'],
  ['#0a84ff', '#bf5af2', '#30d158'],
  ['#ff9f0a', '#ff375f', '#5e5ce6'],
  ['#30d158', '#0a84ff', '#64d2ff'],
  ['#bf5af2', '#ff375f', '#ff9f0a'],
  ['#64d2ff', '#5e5ce6', '#ff375f'],
];

const hoursOf = (s) => Math.round(s / 360) / 10;

/** Compte jusqu'à la valeur, en ralentissant à l'arrivée. */
function countUp(el, to, { duration = 1300, format = (n) => count(Math.round(n)) } = {}) {
  const start = performance.now();
  const step = (now) => {
    const k = Math.min(1, (now - start) / duration);
    el.textContent = format(to * (1 - (1 - k) ** 4));
    if (k < 1 && el.isConnected) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

function art(cover, { round = false, size = 'm' } = {}) {
  const el = h(`div.w-art.${size}${round ? '.round' : ''}`, { style: { background: gradientFor(cover || 'x') } });
  if (cover) el.append(h('img', { src: coverUrl(cover, size === 's'), alt: '', draggable: 'false' }));
  return el;
}

function buildSlides(year, data, onSave) {
  const { music, video } = data;
  const slides = [];
  const lang = currentLanguage();

  slides.push({
    id: 'intro',
    build: () => h('div.w-slide-in', {},
      h('div.w-eyebrow', { text: 'control' }),
      h('div.w-huge', { text: String(year) }),
      h('div.w-lead', { text: music.plays ? t('wrapped.introMusic') : t('wrapped.introVideo') }),
      h('div.w-hint', {}, h('span', { text: t('wrapped.tap') }), h('span', { html: icon('chevron-right', 16, 2.2) }))),
  });

  if (music.plays) {
    slides.push({
      id: 'time',
      build: () => {
        const num = h('div.w-giant', { text: '0' });
        setTimeout(() => countUp(num, Math.round(music.seconds / 60)), 250);
        return h('div.w-slide-in', {},
          h('div.w-eyebrow', { text: t('wrapped.youListened') }),
          num,
          h('div.w-title', { text: t('wrapped.minutes') }),
          h('div.w-lead', { text: t('wrapped.timeDetail', { h: count(hoursOf(music.seconds)), tracks: count(music.tracks), plays: count(music.plays) }) }));
      },
    });

    const artist = music.topArtists[0];
    if (artist) {
      slides.push({
        id: 'artist',
        build: () => h('div.w-slide-in', {},
          h('div.w-eyebrow', { text: t('wrapped.topArtist') }),
          art(artist.cover, { round: true, size: 'l' }),
          h('div.w-big', { text: artist.name }),
          h('div.w-lead', { text: t('wrapped.artistDetail', { plays: count(artist.plays), time: fmtDuration(artist.seconds) }) }),
          music.topArtists.length > 1 ? h('div.w-chips', {}, ...music.topArtists.slice(1, 5).map((a, i) => h('span.w-chip', { style: { '--d': `${0.5 + i * 0.1}s` }, text: `${i + 2}. ${a.name}` }))) : null),
      });
    }

    if (music.topTracks.length) {
      slides.push({
        id: 'tracks',
        build: () => h('div.w-slide-in', {},
          h('div.w-eyebrow', { text: t('wrapped.topTracks') }),
          h('ol.w-list', {}, ...music.topTracks.slice(0, 5).map((x, i) => h('li', { style: { '--d': `${0.15 + i * 0.1}s` } },
            h('span.w-n', { text: String(i + 1) }),
            art(x.cover, { size: 's' }),
            h('div.w-li-text', {}, h('strong', { text: x.title }), h('span', { text: x.artist || '—' })),
            h('span.w-li-val', { text: t('stats.plays', { n: count(x.plays) }) }))))),
      });
    }

    const genre = music.genres[0];
    const album = music.topAlbums[0];
    if (genre || album) {
      slides.push({
        id: 'sound',
        build: () => h('div.w-slide-in', {},
          genre ? h('div.w-eyebrow', { text: t('wrapped.topGenre') }) : null,
          genre ? h('div.w-big', { text: genre.name }) : null,
          genre && music.genres.length > 1 ? h('div.w-chips', {}, ...music.genres.slice(1, 4).map((g, i) => h('span.w-chip', { style: { '--d': `${0.4 + i * 0.1}s` }, text: g.name }))) : null,
          album ? h('div.w-album', {}, art(album.cover, { size: 'l' }), h('div', {}, h('div.w-eyebrow', { text: t('wrapped.topAlbum') }), h('strong', { text: album.album }), h('span', { text: album.artist || '—' }))) : null),
      });
    }

    const busiest = [...music.byDay].sort((a, b) => b.seconds - a.seconds)[0];
    const maxHour = Math.max(...music.byHour, 1);
    const peakHour = music.byHour.indexOf(Math.max(...music.byHour));
    slides.push({
      id: 'habits',
      build: () => {
        const date = busiest ? new Date(Number(busiest.day.slice(0, 4)), Number(busiest.day.slice(5, 7)) - 1, Number(busiest.day.slice(8, 10))) : null;
        return h('div.w-slide-in', {},
          h('div.w-eyebrow', { text: t('wrapped.habits') }),
          date ? h('div.w-big.small', { text: date.toLocaleDateString(lang, { day: 'numeric', month: 'long' }) }) : null,
          date ? h('div.w-lead', { text: t('wrapped.busiestDay', { time: fmtDuration(busiest.seconds) }) }) : null,
          h('div.w-hours', {}, ...music.byHour.map((v, i) => h(`span.w-hour${i === peakHour ? '.peak' : ''}`, { style: { '--h': `${Math.max(4, (v / maxHour) * 100)}%`, '--d': `${i * 0.02}s` } }))),
          h('div.w-lead', { text: t('wrapped.peakHour', { h: peakHour }) }),
          data.streak.longest > 1 ? h('div.w-pill', {}, h('span', { html: icon('flame', 16, 2) }), h('span', { text: t('wrapped.streak', { n: data.streak.longest }) })) : null);
      },
    });
  }

  if (video.seconds >= 60) {
    const top = video.top[0];
    slides.push({
      id: 'video',
      build: () => {
        const num = h('div.w-giant', { text: '0' });
        setTimeout(() => countUp(num, hoursOf(video.seconds), { format: (n) => count(Math.round(n * 10) / 10) }), 250);
        return h('div.w-slide-in', {},
          h('div.w-eyebrow', { text: t('wrapped.youWatched') }),
          num,
          h('div.w-title', { text: t('wrapped.hours') }),
          top ? h('div.w-album', {},
            top.poster ? h('div.w-art.l', {}, h('img', { src: top.poster, alt: '', draggable: 'false' })) : null,
            h('div', {}, h('div.w-eyebrow', { text: t('wrapped.topTitle') }), h('strong', { text: top.title || '—' }), h('span', { text: fmtDuration(top.seconds) }))) : null);
      },
    });
  }

  slides.push({
    id: 'summary',
    build: () => h('div.w-slide-in', {},
      h('div.w-eyebrow', { text: t('wrapped.summary') }),
      h('div.w-summary', {},
        h('div.w-sum-head', {}, h('strong', { text: String(year) }), h('span', { text: 'control' })),
        music.plays ? h('div.w-sum-stats', {}, h('div', {}, h('b', { text: count(Math.round(music.seconds / 60)) }), h('span', { text: t('wrapped.minutes') })), h('div', {}, h('b', { text: count(music.plays) }), h('span', { text: t('stats.playsLabel') }))) : null,
        music.topArtists[0] ? h('div.w-sum-line', {}, h('span', { text: t('wrapped.topArtist') }), h('strong', { text: music.topArtists[0].name })) : null,
        music.topTracks[0] ? h('div.w-sum-line', {}, h('span', { text: t('wrapped.topTrack') }), h('strong', { text: music.topTracks[0].title })) : null,
        music.genres[0] ? h('div.w-sum-line', {}, h('span', { text: t('wrapped.topGenre') }), h('strong', { text: music.genres[0].name })) : null,
        video.seconds >= 60 ? h('div.w-sum-line', {}, h('span', { text: t('wrapped.youWatched') }), h('strong', { text: fmtDuration(video.seconds) })) : null),
      h('button.btn.btn-primary.w-save', { type: 'button', onclick: onSave }, h('span.btn-icon', { html: icon('download', 15, 2.2) }), h('span', { text: t('wrapped.save') }))),
  });

  return slides;
}

// ─── Image à partager ────────────────────────────────────────────────────────
function loadImage(src) {
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => resolve(null);
    img.src = src;
  });
}

function roundRect(ctx, x, y, w, h2, r) {
  ctx.beginPath();
  ctx.roundRect(x, y, w, h2, r);
}

export async function renderCard(year, data) {
  const { music, video } = data;
  const W = 1080;
  const H = 1350;
  const canvas = document.createElement('canvas');
  canvas.width = W;
  canvas.height = H;
  const ctx = canvas.getContext('2d');
  await Promise.all(['800 120px "Inter Var"', '700 44px "Inter Var"', '500 34px "Inter Var"'].map((f) => document.fonts.load(f).catch(() => {})));
  const font = (weight, size) => `${weight} ${size}px "Inter Var", "Segoe UI", system-ui, sans-serif`;
  const accent = getComputedStyle(document.documentElement).getPropertyValue('--accent-rgb').trim().split(/\s+/).join(',') || '255,55,95';

  ctx.fillStyle = '#0d0d12';
  ctx.fillRect(0, 0, W, H);
  const glow = (x, y, r, color) => {
    const g = ctx.createRadialGradient(x, y, 0, x, y, r);
    g.addColorStop(0, color);
    g.addColorStop(1, 'rgba(0,0,0,0)');
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, W, H);
  };
  glow(180, 140, 760, `rgba(${accent},0.55)`);
  glow(980, 1180, 820, 'rgba(120,80,255,0.42)');
  glow(900, 120, 420, 'rgba(255,170,60,0.22)');

  ctx.fillStyle = 'rgba(255,255,255,0.7)';
  ctx.font = font(700, 36);
  ctx.fillText('control', 80, 120);
  ctx.textAlign = 'right';
  ctx.fillText(String(year), W - 80, 120);
  ctx.textAlign = 'left';

  ctx.fillStyle = '#fff';
  ctx.font = font(800, 190);
  const minutes = Math.round(music.seconds / 60);
  ctx.fillText(count(minutes), 76, 330);
  ctx.fillStyle = 'rgba(255,255,255,0.78)';
  ctx.font = font(600, 46);
  ctx.fillText(t('wrapped.minutes'), 80, 396);

  const stat = (label, value, x, y) => {
    ctx.fillStyle = 'rgba(255,255,255,0.5)';
    ctx.font = font(600, 28);
    ctx.fillText(label.toUpperCase(), x, y);
    ctx.fillStyle = '#fff';
    ctx.font = font(700, 50);
    ctx.fillText(value.length > 20 ? `${value.slice(0, 19)}…` : value, x, y + 58);
  };
  stat(t('wrapped.topArtist'), music.topArtists[0]?.name || '—', 80, 500);
  stat(t('wrapped.topGenre'), music.genres[0]?.name || '—', 580, 500);
  stat(t('wrapped.topTrack'), music.topTracks[0]?.title || '—', 80, 640);
  stat(video.seconds >= 60 ? t('wrapped.youWatched') : t('stats.playsLabel'), video.seconds >= 60 ? fmtDuration(video.seconds) : count(music.plays), 580, 640);

  // Top 5 des morceaux
  const rows = music.topTracks.slice(0, 5);
  const images = await Promise.all(rows.map((r) => (r.cover ? loadImage(coverUrl(r.cover, true)) : null)));
  ctx.fillStyle = 'rgba(255,255,255,0.5)';
  ctx.font = font(600, 28);
  ctx.fillText(t('wrapped.topTracks').toUpperCase(), 80, 800);
  rows.forEach((r, i) => {
    const y = 840 + i * 92;
    ctx.save();
    roundRect(ctx, 80, y, 72, 72, 14);
    ctx.clip();
    if (images[i]) ctx.drawImage(images[i], 80, y, 72, 72);
    else {
      const g = ctx.createLinearGradient(80, y, 152, y + 72);
      g.addColorStop(0, `rgba(${accent},0.9)`);
      g.addColorStop(1, 'rgba(120,80,255,0.9)');
      ctx.fillStyle = g;
      ctx.fillRect(80, y, 72, 72);
    }
    ctx.restore();
    ctx.fillStyle = '#fff';
    ctx.font = font(700, 36);
    const title = r.title.length > 30 ? `${r.title.slice(0, 29)}…` : r.title;
    ctx.fillText(title, 176, y + 34);
    ctx.fillStyle = 'rgba(255,255,255,0.6)';
    ctx.font = font(500, 28);
    const artist = (r.artist || '—').length > 34 ? `${r.artist.slice(0, 33)}…` : (r.artist || '—');
    ctx.fillText(artist, 176, y + 68);
    ctx.textAlign = 'right';
    ctx.fillStyle = 'rgba(255,255,255,0.85)';
    ctx.font = font(700, 32);
    ctx.fillText(String(r.plays), W - 80, y + 48);
    ctx.textAlign = 'left';
  });
  return canvas.toDataURL('image/png');
}

// ─── Fenêtre ─────────────────────────────────────────────────────────────────
export function openWrapped(year, data, { years = [], onYear } = {}) {
  const layer = document.getElementById('layer');
  const overlay = h('div.wrapped');
  const stage = h('div.w-stage');
  const bars = h('div.w-bars');
  const closeBtn = h('button.w-close', { type: 'button', 'aria-label': t('common.close'), html: icon('x', 18, 2.2) });
  const prev = h('button.w-nav.prev', { type: 'button', 'aria-label': t('tag.prev'), html: icon('chevron-left', 22, 2.2) });
  const next = h('button.w-nav.next', { type: 'button', 'aria-label': t('tag.next'), html: icon('chevron-right', 22, 2.2) });

  const empty = !data.music.plays && data.video.seconds < 60;
  const slides = empty
    ? [{ id: 'empty', build: () => h('div.w-slide-in', {}, h('div.w-eyebrow', { text: 'control' }), h('div.w-huge', { text: String(year) }), h('div.w-lead', { text: t('wrapped.empty') })) }]
    : buildSlides(year, data, async () => {
      const dataUrl = await renderCard(year, data);
      const saved = await api.saveImage(dataUrl, `control-${year}`);
      if (saved) toast(t('wrapped.saved'), 'image');
    });

  let index = 0;
  let closing = false;
  const segs = slides.map(() => h('span.w-bar', {}, h('i')));
  bars.append(...segs);

  const yearPicker = years.length > 1 ? h('div.w-years', {}, ...years.slice(0, 6).map((y) => h(`button.w-year${y === year ? '.on' : ''}`, { type: 'button', text: String(y), onclick: () => { if (y !== year) { close(); onYear?.(y); } } }))) : null;

  function show(i, dir = 1) {
    index = Math.max(0, Math.min(slides.length - 1, i));
    stage.dataset.slide = slides[index].id;
    stage.dataset.i = String(index % 6);
    const el = h('div.w-slide', {}, slides[index].build());
    el.classList.add(dir > 0 ? 'fwd' : 'back');
    stage.replaceChildren(el);
    const [a, b, c] = PALETTES[index % PALETTES.length];
    overlay.style.setProperty('--g1', a);
    overlay.style.setProperty('--g2', b);
    overlay.style.setProperty('--g3', c);
    segs.forEach((s, k) => s.classList.toggle('done', k < index));
    segs.forEach((s, k) => s.classList.toggle('now', k === index));
    prev.hidden = index === 0;
    next.hidden = index === slides.length - 1;
  }

  const go = (d) => {
    const target = index + d;
    if (target < 0 || target >= slides.length) return;
    show(target, d);
  };

  function close() {
    if (closing) return;
    closing = true;
    document.removeEventListener('keydown', onKey, true);
    overlay.classList.remove('open');
    setTimeout(() => overlay.remove(), 350);
  }

  const onKey = (e) => {
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(); }
    else if (e.key === 'ArrowRight' || e.key === ' ' || e.key === 'Enter') { e.preventDefault(); e.stopPropagation(); go(1); }
    else if (e.key === 'ArrowLeft') { e.preventDefault(); e.stopPropagation(); go(-1); }
  };
  document.addEventListener('keydown', onKey, true);

  stage.addEventListener('click', (e) => {
    if (e.target.closest('button')) return;
    const rect = stage.getBoundingClientRect();
    go(e.clientX - rect.left < rect.width * 0.3 ? -1 : 1);
  });
  prev.addEventListener('click', () => go(-1));
  next.addEventListener('click', () => go(1));
  closeBtn.addEventListener('click', close);

  overlay.append(h('div.w-bg'), bars, closeBtn, yearPicker, stage, prev, next);
  layer.append(overlay);
  show(0);
  requestAnimationFrame(() => overlay.classList.add('open'));
  return { close };
}

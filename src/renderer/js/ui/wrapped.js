// Récap annuel ou mensuel : une suite de cartes plein écran (musique, habitudes, films), et une image à enregistrer.

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

// ─── Périodes ───
// { kind: 'year', year } | { kind: 'month', year, month } (mois de 1 à 12)
// | { kind: 'week', start: 'AAAA-MM-JJ' (le lundi) } | { kind: 'day', date: 'AAAA-MM-JJ' }
const cap = (s) => s.charAt(0).toLocaleUpperCase() + s.slice(1);
const pad2 = (n) => String(n).padStart(2, '0');
const parseDay = (s) => new Date(Number(s.slice(0, 4)), Number(s.slice(5, 7)) - 1, Number(s.slice(8, 10)));
const dayStr = (d) => `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
const shift = (s, n) => { const d = parseDay(s); d.setDate(d.getDate() + n); return dayStr(d); };

export const monthName = (p) => cap(new Date(p.year, p.month - 1, 1).toLocaleDateString(currentLanguage(), { month: 'long' }));
export const mondayOf = (s) => shift(s, -((parseDay(s).getDay() + 6) % 7));
/** Lundis des semaines qui ont de l'activité, la plus récente d'abord. */
export const weeksOf = (days) => [...new Set(days.map(mondayOf))].sort().reverse();

export function isoWeek(date) {
  const d = new Date(Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()));
  d.setUTCDate(d.getUTCDate() + 4 - (d.getUTCDay() || 7));
  const first = new Date(Date.UTC(d.getUTCFullYear(), 0, 1));
  return Math.ceil(((d - first) / 86400000 + 1) / 7);
}

/** « 20 – 26 octobre » ou « 28 sept. – 4 oct. » (avec l'année si demandé). */
function weekSpan(start, withYear = false) {
  const lang = currentLanguage();
  const a = parseDay(start);
  const b = parseDay(shift(start, 6));
  const text = a.getMonth() === b.getMonth()
    ? `${a.getDate()} – ${b.toLocaleDateString(lang, { day: 'numeric', month: 'long' })}`
    : `${a.toLocaleDateString(lang, { day: 'numeric', month: 'short' })} – ${b.toLocaleDateString(lang, { day: 'numeric', month: 'short' })}`;
  return withYear ? `${text} ${b.getFullYear()}` : text;
}

/** Titre court, en grand sur la première carte. */
export function periodTitle(p) {
  if (p.kind === 'month') return monthName(p);
  if (p.kind === 'week') return t('wrapped.weekN', { n: isoWeek(parseDay(p.start)) });
  if (p.kind === 'day') return cap(parseDay(p.date).toLocaleDateString(currentLanguage(), { weekday: 'long', day: 'numeric' }));
  return String(p.year);
}

/** Précision sous « control » : l'année, la plage de jours, le mois. */
export function periodSub(p) {
  if (p.kind === 'month') return String(p.year);
  if (p.kind === 'week') return weekSpan(p.start, true);
  if (p.kind === 'day') return cap(parseDay(p.date).toLocaleDateString(currentLanguage(), { month: 'long', year: 'numeric' }));
  return '';
}

export function periodLabel(p) {
  if (p.kind === 'month') return `${monthName(p)} ${p.year}`;
  if (p.kind === 'week') return `${t('wrapped.weekN', { n: isoWeek(parseDay(p.start)) })} · ${weekSpan(p.start)}`;
  if (p.kind === 'day') return parseDay(p.date).toLocaleDateString(currentLanguage(), { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' });
  return String(p.year);
}

/** Libellé de menu : « Aujourd'hui », « Hier », « Cette semaine »… sinon le libellé complet. */
export function periodMenuLabel(p, now = new Date()) {
  const today = dayStr(now);
  if (p.kind === 'day') {
    if (p.date === today) return t('wrapped.today');
    if (p.date === shift(today, -1)) return t('wrapped.yesterday');
    return cap(parseDay(p.date).toLocaleDateString(currentLanguage(), { weekday: 'long', day: 'numeric', month: 'long' }));
  }
  if (p.kind === 'week') {
    if (p.start === mondayOf(today)) return t('wrapped.thisWeek');
    if (p.start === mondayOf(shift(today, -7))) return t('wrapped.lastWeek');
  }
  return periodLabel(p);
}

export function periodRange(p) {
  if (p.kind === 'year') return { from: `${p.year}-01-01`, to: `${p.year}-12-31` };
  if (p.kind === 'week') return { from: p.start, to: shift(p.start, 6) };
  if (p.kind === 'day') return { from: p.date, to: p.date };
  const last = new Date(p.year, p.month, 0).getDate();
  return { from: `${p.year}-${pad2(p.month)}-01`, to: `${p.year}-${pad2(p.month)}-${pad2(last)}` };
}

export const monthKey = (p) => `${p.year}-${pad2(p.month)}`;
export const parseMonthKey = (key) => ({ kind: 'month', year: Number(key.slice(0, 4)), month: Number(key.slice(5, 7)) });

/** Clé d'une période (nom de fichier, mémoire des récaps proposés) et son inverse. */
export function periodKey(p) {
  if (p.kind === 'month') return monthKey(p);
  if (p.kind === 'week') return p.start;
  if (p.kind === 'day') return p.date;
  return String(p.year);
}

export function parsePeriod(kind, key) {
  if (kind === 'month') return parseMonthKey(key);
  if (kind === 'week') return { kind, start: key };
  if (kind === 'day') return { kind, date: key };
  return { kind: 'year', year: Number(key) };
}

/** Classe du grand titre : l'année en géant, les titres plus longs un peu plus petits. */
const titleClass = (p) => (p.kind === 'year' ? 'div.w-huge' : periodTitle(p).length > 9 ? 'div.w-huge.month.long' : 'div.w-huge.month');

/** Suffixe des textes propres à la durée de la période (année : aucun). */
const SUFFIX = { year: '', month: 'Month', week: 'Week', day: 'Day' };

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

function buildSlides(period, data, onSave) {
  const { music, video } = data;
  const slides = [];
  const lang = currentLanguage();
  const sfx = SUFFIX[period.kind];
  const sub = periodSub(period);
  // « Aujourd'hui », « Cette semaine » quand la période est en cours, sinon « Ce jour-là », « Cette semaine-là ».
  const today = dayStr(new Date());
  const listened = period.kind === 'day' && period.date === today ? 'wrapped.youListenedToday'
    : period.kind === 'week' && period.start === mondayOf(today) ? 'wrapped.youListenedThisWeek'
      : `wrapped.youListened${sfx}`;

  slides.push({
    id: 'intro',
    build: () => h('div.w-slide-in', {},
      h('div.w-eyebrow', { text: sub ? `control · ${sub}` : 'control' }),
      h(titleClass(period), { text: periodTitle(period) }),
      h('div.w-lead', { text: t(music.plays ? `wrapped.introMusic${sfx}` : `wrapped.introVideo${sfx}`) }),
      h('div.w-hint', {}, h('span', { text: t('wrapped.tap') }), h('span', { html: icon('chevron-right', 16, 2.2) }))),
  });

  if (music.plays) {
    slides.push({
      id: 'time',
      build: () => {
        const num = h('div.w-giant', { text: '0' });
        setTimeout(() => countUp(num, Math.round(music.seconds / 60)), 250);
        return h('div.w-slide-in', {},
          h('div.w-eyebrow', { text: t(listened) }),
          num,
          h('div.w-title', { text: t('wrapped.minutes') }),
          h('div.w-lead', { text: music.seconds < 3600
            ? t('wrapped.timeDetailShort', { tracks: count(music.tracks), plays: count(music.plays) })
            : t('wrapped.timeDetail', { h: count(hoursOf(music.seconds)), tracks: count(music.tracks), plays: count(music.plays) }) }));
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

    // Pour une seule journée, « la journée la plus musicale » n'a pas de sens : seules les heures comptent.
    const busiest = period.kind === 'day' ? null : [...music.byDay].sort((a, b) => b.seconds - a.seconds)[0];
    const maxHour = Math.max(...music.byHour, 1);
    const peakHour = music.byHour.indexOf(Math.max(...music.byHour));
    slides.push({
      id: 'habits',
      build: () => {
        const date = busiest ? new Date(Number(busiest.day.slice(0, 4)), Number(busiest.day.slice(5, 7)) - 1, Number(busiest.day.slice(8, 10))) : null;
        return h('div.w-slide-in', {},
          h('div.w-eyebrow', { text: t('wrapped.habits') }),
          date ? h('div.w-big.small', { text: date.toLocaleDateString(lang, period.kind === 'week' ? { weekday: 'long', day: 'numeric', month: 'long' } : { day: 'numeric', month: 'long' }) }) : null,
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
        h('div.w-sum-head', {}, h('strong', { text: periodTitle(period) }), h('span', { text: sub ? `control · ${sub}` : 'control' })),
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

export async function renderCard(period, data) {
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
  ctx.fillText(periodLabel(period), W - 80, 120);
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
/**
 * period : voir « Périodes » plus haut.
 * data : résultat de stats.get pour cette période ; years, months, weeks (lundis) et days : périodes qui ont de l'activité.
 * load(range) : charge une autre période sans fermer la fenêtre.
 */
export function openWrapped(initial, initialData, { years = [], months = [], weeks = [], days = [], load } = {}) {
  const layer = document.getElementById('layer');
  const overlay = h('div.wrapped');
  const stage = h('div.w-stage');
  const bars = h('div.w-bars');
  const head = h('div.w-head');
  const closeBtn = h('button.w-close', { type: 'button', 'aria-label': t('common.close'), html: icon('x', 18, 2.2) });
  const prev = h('button.w-nav.prev', { type: 'button', 'aria-label': t('tag.prev'), html: icon('chevron-left', 22, 2.2) });
  const next = h('button.w-nav.next', { type: 'button', 'aria-label': t('tag.next'), html: icon('chevron-right', 22, 2.2) });

  let period = initial;
  let slides = [];
  let segs = [];
  let index = 0;
  let closing = false;
  let busy = false;

  const isEmpty = (d) => !d.music.plays && d.video.seconds < 60;

  function makeSlides(data) {
    if (isEmpty(data)) {
      return [{
        id: 'empty',
        build: () => h('div.w-slide-in', {},
          h('div.w-eyebrow', { text: 'control' }),
          h(titleClass(period), { text: periodTitle(period) }),
          h('div.w-lead', { text: t(`wrapped.empty${SUFFIX[period.kind]}`) })),
      }];
    }
    return buildSlides(period, data, async () => {
      const dataUrl = await renderCard(period, data);
      const saved = await api.saveImage(dataUrl, `control-${period.kind === 'year' ? '' : `${period.kind}-`}${periodKey(period)}`);
      if (saved) toast(t('wrapped.saved'), 'image');
    });
  }

  function show(i, dir = 1) {
    index = Math.max(0, Math.min(slides.length - 1, i));
    stage.dataset.slide = slides[index].id;
    const el = h('div.w-slide', {}, slides[index].build());
    el.classList.add(dir > 0 ? 'fwd' : 'back');
    stage.replaceChildren(el);
    const [a, b, c] = PALETTES[index % PALETTES.length];
    overlay.style.setProperty('--g1', a);
    overlay.style.setProperty('--g2', b);
    overlay.style.setProperty('--g3', c);
    segs.forEach((s, k) => {
      s.classList.toggle('done', k < index);
      s.classList.toggle('now', k === index);
    });
    prev.hidden = index === 0;
    next.hidden = index === slides.length - 1;
  }

  function rebuild(data) {
    slides = makeSlides(data);
    segs = slides.map(() => h('span.w-bar', {}, h('i')));
    bars.replaceChildren(...segs);
    paintHead();
    show(0);
  }

  async function setPeriod(next) {
    if (busy || !load) return;
    busy = true;
    stage.classList.add('loading');
    try {
      const data = await load(periodRange(next));
      period = next;
      rebuild(data);
    } catch {
      // On reste sur la période affichée.
    } finally {
      stage.classList.remove('loading');
      busy = false;
    }
  }

  // ─── Sélecteur de période (en haut à gauche) ───────────────────────────────
  const pill = (label, on, onClick, extra = '') => h(`button.w-year${on ? '.on' : ''}${extra}`, { type: 'button', text: label, onclick: onClick });

  const KINDS = [
    { kind: 'year', label: 'wrapped.modeYear', list: () => years.map(String), before: '', after: '' },
    { kind: 'month', label: 'wrapped.modeMonth', list: () => months, before: 'wrapped.monthBefore', after: 'wrapped.monthAfter' },
    { kind: 'week', label: 'wrapped.modeWeek', list: () => weeks, before: 'wrapped.weekBefore', after: 'wrapped.weekAfter' },
    { kind: 'day', label: 'wrapped.modeDay', list: () => days, before: 'wrapped.dayBefore', after: 'wrapped.dayAfter' },
  ];
  const listOf = (kind) => KINDS.find((k) => k.kind === kind).list();

  /** Passer d'une durée à une autre : la période de la nouvelle durée qui recoupe celle qu'on regarde, sinon la plus proche avant. */
  function switchKind(kind) {
    const list = listOf(kind);
    if (kind === period.kind || !list.length) return;
    const here = periodRange(period);
    const overlaps = (key) => { const r = periodRange(parsePeriod(kind, key)); return r.from <= here.to && r.to >= here.from; };
    const key = list.find(overlaps) || list.find((k) => periodRange(parsePeriod(kind, k)).to < here.from) || list[list.length - 1];
    setPeriod(parsePeriod(kind, key));
  }

  function paintHead() {
    const parts = [];
    const available = KINDS.filter((k) => k.list().length);
    if (available.length > 1) {
      parts.push(h('div.w-modes', {}, ...available.map((k) => pill(t(k.label), period.kind === k.kind, () => switchKind(k.kind)))));
    }
    if (period.kind === 'year') {
      if (years.length > 1) {
        parts.push(h('div.w-years', {}, ...years.slice(0, 6).map((y) => pill(String(y), y === period.year, () => y !== period.year && setPeriod({ kind: 'year', year: y })))));
      }
    } else {
      const spec = KINDS.find((k) => k.kind === period.kind);
      const list = spec.list();
      const key = periodKey(period);
      // Précédent / suivant parmi les périodes qui ont de l'activité (la liste va du plus récent au plus ancien).
      const older = list.find((k) => k < key);
      const newer = [...list].reverse().find((k) => k > key);
      const arrow = (ic, target, label) => h('button.w-mstep', { type: 'button', 'aria-label': t(label), disabled: !target, html: icon(ic, 15, 2.4), onclick: () => target && setPeriod(parsePeriod(period.kind, target)) });
      parts.push(h('div.w-month-nav', {},
        arrow('chevron-left', older, spec.before),
        h('span.w-month-label', { text: periodLabel(period) }),
        arrow('chevron-right', newer, spec.after)));
    }
    head.replaceChildren(...parts);
  }

  // ─── Fermeture et clavier ──────────────────────────────────────────────────
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

  overlay.append(h('div.w-bg'), bars, head, closeBtn, stage, prev, next);
  layer.append(overlay);
  rebuild(initialData);
  requestAnimationFrame(() => overlay.classList.add('open'));
  return { close };
}

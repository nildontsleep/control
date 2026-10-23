// Statistiques : temps d'écoute, activité, tops, genres, films et séries ; accès au récap annuel.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t, currentLanguage } from '../i18n.js';
import { count } from '../format.js';
import { lib } from '../data.js';
import { settings, update } from '../settings.js';
import { navigate } from '../router.js';
import { player } from '../player.js';
import { segmented, button, createScope, withScope, refreshScope } from '../ui/controls.js';
import { coverEl } from '../ui/cover.js';
import { openWrapped, periodRange, periodLabel, parseMonthKey } from '../ui/wrapped.js';
import { openMenu } from '../ui/menu.js';
import { pageHead, emptyState } from './shared.js';
import { fmtDuration, dayLabel, shortDay } from '../stats/format.js';

const api = window.control.stats;

const pad = (n) => String(n).padStart(2, '0');
const keyOf = (d) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const addDays = (d, n) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);

export function rangeFor(kind, now = new Date()) {
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  if (kind === '7d') return { from: keyOf(addDays(today, -6)), to: keyOf(today), days: 7 };
  if (kind === '30d') return { from: keyOf(addDays(today, -29)), to: keyOf(today), days: 30 };
  if (kind === 'year') return { from: `${today.getFullYear()}-01-01`, to: keyOf(today), days: null };
  return { from: '', to: '', days: null };
}

// ─── Info-bulle des graphiques (immédiate, suit le pointeur) ────────────────
let tipEl = null;
function chartTip() {
  if (!tipEl) {
    tipEl = h('div.chart-tip');
    document.body.append(tipEl);
    // Si la page change sous le pointeur, aucun « pointerleave » ne vient : on la cache aussi au clic, à la touche et au défilement.
    const hide = () => tipEl.classList.remove('show');
    document.addEventListener('pointerdown', hide, true);
    document.addEventListener('keydown', hide, true);
    document.addEventListener('wheel', hide, { passive: true, capture: true });
  }
  return tipEl;
}
function bindTip(el, getContent) {
  el.addEventListener('pointerenter', () => {
    const tip = chartTip();
    tip.replaceChildren(...[].concat(getContent()));
    tip.classList.add('show');
  });
  el.addEventListener('pointermove', (e) => {
    const tip = chartTip();
    const w = tip.offsetWidth;
    const x = Math.min(window.innerWidth - w - 10, Math.max(10, e.clientX - w / 2));
    tip.style.transform = `translate(${Math.round(x)}px, ${Math.round(e.clientY - tip.offsetHeight - 14)}px)`;
  });
  el.addEventListener('pointerleave', () => tipEl?.classList.remove('show'));
}
const tipLines = (title, value) => [h('strong', { text: title }), h('span', { text: value })];

// ─── Briques ─────────────────────────────────────────────────────────────────
function kpi(label, value, sub, ic) {
  return h('div.st-kpi', {},
    h('span.st-kpi-icon', { html: icon(ic, 17, 1.9) }),
    h('div.st-kpi-value', { text: value }),
    h('div.st-kpi-label', { text: label }),
    sub ? h('div.st-kpi-sub', { text: sub }) : null);
}

function card(title, ...children) {
  return h('section.st-card', {}, title ? h('h2.st-card-title', { text: title }) : null, ...children);
}

/** Paliers « jolis » pour l'axe : 15 min, 30 min, 1 h, 2 h… */
function niceMax(seconds) {
  const steps = [600, 900, 1800, 3600, 7200, 10800, 14400, 21600, 28800, 43200, 86400];
  return steps.find((s) => s >= seconds) || Math.ceil(seconds / 3600) * 3600;
}

/** Histogramme à barres fines : items = [{ label, value, title, sub }], valeur en secondes. */
function barChart(items, { labelEvery = 1, height = 150, axis = true } = {}) {
  const max = niceMax(Math.max(...items.map((i) => i.value), 1));
  const bars = h('div.st-bars', { style: { '--n': items.length } });
  items.forEach((item, i) => {
    const pct = item.value > 0 ? Math.max(2.5, (item.value / max) * 100) : 0;
    const bar = h('div.st-bar', {}, h('span.st-bar-fill', { style: { height: `${pct}%` } }), h('span.st-bar-label', { text: i % labelEvery === 0 ? item.label : '' }));
    bindTip(bar, () => tipLines(item.title || item.label, item.sub ?? fmtDuration(item.value)));
    bars.append(bar);
  });
  const grid = axis ? h('div.st-grid', {}, [1, 0.5, 0].map((f) => h('div.st-gridline', { style: { bottom: `${f * 100}%` } }, h('span', { text: f === 0 ? '' : fmtDuration(max * f) })))) : null;
  return h('div.st-chart', { style: { '--h': `${height}px` } }, grid, bars);
}

/** Calendrier façon « contributions » : une colonne par semaine, du lundi au dimanche. */
function heatmap(byDay, { weeks = 53 } = {}) {
  const map = new Map(byDay.map((d) => [d.day, d.seconds]));
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const mondayOffset = (today.getDay() + 6) % 7;
  const start = addDays(today, -(weeks - 1) * 7 - mondayOffset);
  const values = [...map.values()].filter((v) => v > 0).sort((a, b) => a - b);
  const q = (p) => values[Math.min(values.length - 1, Math.floor(values.length * p))] || 0;
  const cuts = [q(0.25), q(0.5), q(0.75)];
  const level = (v) => (v <= 0 ? 0 : v <= cuts[0] ? 1 : v <= cuts[1] ? 2 : v <= cuts[2] ? 3 : 4);

  const grid = h('div.st-heat');
  const months = h('div.st-heat-months');
  let lastMonth = -1;
  for (let w = 0; w < weeks; w++) {
    const col = h('div.st-heat-col');
    for (let d = 0; d < 7; d++) {
      const date = addDays(start, w * 7 + d);
      if (date > today) { col.append(h('span.st-cell.void')); continue; }
      const key = keyOf(date);
      const v = map.get(key) || 0;
      const cell = h(`span.st-cell.l${level(v)}`);
      bindTip(cell, () => tipLines(dayLabel(date), v ? fmtDuration(v) : t('stats.nothing')));
      col.append(cell);
    }
    grid.append(col);
    const first = addDays(start, w * 7);
    const m = first.getMonth();
    months.append(h('span', { text: m !== lastMonth && first.getDate() <= 7 ? first.toLocaleDateString(currentLanguage(), { month: 'short' }) : '' }));
    if (first.getDate() <= 7) lastMonth = m;
  }
  const legend = h('div.st-heat-legend', {}, h('span', { text: t('stats.less') }), ...[0, 1, 2, 3, 4].map((l) => h(`span.st-cell.l${l}`)), h('span', { text: t('stats.more') }));
  return h('div.st-heatwrap', {}, months, grid, legend);
}

/** Liste classée avec barre de proportion. */
function rankList(items, { render, max }) {
  const top = max ?? Math.max(...items.map((i) => i.plays ?? i.seconds), 1);
  return h('ol.st-rank', {}, ...items.map((item, i) => {
    const row = render(item, i);
    const pct = ((item.plays ?? item.seconds) / top) * 100;
    row.style.setProperty('--pct', `${Math.max(4, pct)}%`);
    return row;
  }));
}

function rankRow({ index, art, title, sub, value, onClick, round = false }) {
  const row = h('li.st-row', { tabindex: onClick ? '0' : null, role: onClick ? 'link' : null },
    h('span.st-rank-n', { text: String(index + 1) }),
    h(`div.st-row-art${round ? '.round' : ''}`, {}, art),
    h('div.st-row-text', {}, h('div.st-row-title', { text: title }), sub ? h('div.st-row-sub', { text: sub }) : null),
    h('div.st-row-value', { text: value }),
    h('span.st-row-bar'));
  if (onClick) {
    row.addEventListener('click', onClick);
    row.addEventListener('keydown', (e) => { if (e.key === 'Enter') onClick(); });
  }
  return row;
}

const plays = (n) => t('stats.plays', { n: count(n) });

// ─── Vue ─────────────────────────────────────────────────────────────────────
export function statsView(root) {
  let destroyed = false;
  const body = h('div.stats');
  const state = { kind: settings.statsRange || '30d' };

  const scope = createScope();
  const periodSeg = withScope(scope, () => segmented({
    get: () => state.kind,
    set: (v) => { state.kind = v; update({ statsRange: v }); refreshScope(scope); load(); },
  }, [
    { value: '7d', label: t('stats.p7') },
    { value: '30d', label: t('stats.p30') },
    { value: 'year', label: t('stats.pYear') },
    { value: 'all', label: t('stats.pAll') },
  ], { compact: true }));

  const wrappedBtn = button(t('stats.wrappedShort'), { icon: 'sparkles', kind: 'primary', onClick: () => openRecapMenu() });
  wrappedBtn.hidden = true;

  root.append(
    pageHead({ title: t('nav.stats'), subtitle: t('stats.subtitle'), actions: h('div.toolbar-inline', {}, periodSeg, wrappedBtn) }),
    body);

  let years = [];
  let months = [];
  const refreshWrappedButton = (data) => {
    years = data.years;
    months = data.months || [];
    wrappedBtn.hidden = !years.length;
  };

  async function startWrapped(period) {
    const res = await api.get(periodRange(period));
    if (!res.ok) return;
    openWrapped(period, res.data, {
      years,
      months,
      load: async (range) => {
        const r = await api.get(range);
        if (!r.ok) throw new Error(r.message);
        return r.data;
      },
    });
  }

  /** Un récap par année et un par mois d'activité. */
  function openRecapMenu() {
    const items = years.slice(0, 3).map((year) => ({ label: t('stats.wrapped', { year }), icon: 'sparkles', action: () => startWrapped({ kind: 'year', year }) }));
    if (months.length) {
      items.push({ separator: true }, { header: t('stats.byMonth') });
      for (const key of months.slice(0, 12)) {
        const period = parseMonthKey(key);
        items.push({ label: periodLabel(period), icon: 'calendar', action: () => startWrapped(period) });
      }
    }
    openMenu(wrappedBtn, items, { align: 'end', minWidth: 200 });
  }

  async function load() {
    body.classList.add('loading');
    const range = rangeFor(state.kind);
    const res = await api.get({ from: range.from, to: range.to });
    if (destroyed) return;
    body.classList.remove('loading');
    if (!res.ok) {
      body.replaceChildren(emptyState({ icon: 'triangle-alert', title: t('err.title'), text: res.message }));
      return;
    }
    refreshWrappedButton(res.data);
    paint(res.data, range);
  }

  function paint(data, range) {
    const { music, video } = data;
    // Sans aucun historique, un écran d'accueil plutôt que des zéros.
    if (!music.plays && !video.seconds && !data.years.length) {
      body.replaceChildren(emptyState({ icon: 'chart-column', title: t('stats.emptyTitle'), text: t('stats.emptyText') }));
      return;
    }

    const sections = [];

    // Musique — chiffres clés
    sections.push(h('section.st-block', {},
      h('h2.st-block-title', { text: t('stats.music') }),
      h('div.st-kpis', {},
        kpi(t('stats.listened'), fmtDuration(music.seconds), null, 'headphones'),
        kpi(t('stats.playsLabel'), count(music.plays), music.completed ? t('stats.completed', { n: count(music.completed) }) : null, 'play'),
        kpi(t('stats.tracksLabel'), count(music.tracks), t('stats.albumsOf', { n: count(music.albums) }), 'music'),
        kpi(t('stats.artistsLabel'), count(music.artists), null, 'users'),
        kpi(t('stats.streak'), t('stats.days', { n: data.streak.current }), t('stats.bestStreak', { n: data.streak.longest }), 'flame'))));

    // Activité
    const byDay = new Map(music.byDay.map((d) => [d.day, d.seconds]));
    let activity;
    if (range.days) {
      const items = [];
      const from = new Date(Number(range.from.slice(0, 4)), Number(range.from.slice(5, 7)) - 1, Number(range.from.slice(8, 10)));
      for (let i = 0; i < range.days; i++) {
        const d = addDays(from, i);
        const v = byDay.get(keyOf(d)) || 0;
        items.push({ label: shortDay(d, range.days), value: v, title: dayLabel(d), sub: v ? fmtDuration(v) : t('stats.nothing') });
      }
      activity = barChart(items, { labelEvery: range.days > 14 ? 5 : 1 });
    } else {
      activity = heatmap(music.byDay);
    }
    sections.push(card(t('stats.activity'), activity));

    // Heures et jours
    const hours = music.byHour.map((v, i) => ({ label: i % 6 === 0 ? `${i} h` : '', value: v, title: `${pad(i)}:00 – ${pad(i)}:59`, sub: v ? fmtDuration(v) : t('stats.nothing') }));
    const weekdays = music.byWeekday.map((v, i) => {
      const d = new Date(2024, 0, 1 + i); // un lundi
      const name = d.toLocaleDateString(currentLanguage(), { weekday: 'long' });
      return { label: d.toLocaleDateString(currentLanguage(), { weekday: 'short' }).replace('.', ''), value: v, title: name, sub: v ? fmtDuration(v) : t('stats.nothing') };
    });
    const peakHour = music.byHour.indexOf(Math.max(...music.byHour));
    const peakDay = music.byWeekday.indexOf(Math.max(...music.byWeekday));
    sections.push(h('div.st-grid2', {},
      card(t('stats.byHour'), barChart(hours, { height: 120, labelEvery: 1, axis: false }), music.seconds ? h('p.st-note', { text: t('stats.peakHour', { h: peakHour }) }) : null),
      card(t('stats.byWeekday'), barChart(weekdays, { height: 120, axis: false }), music.seconds ? h('p.st-note', { text: t('stats.peakDay', { d: new Date(2024, 0, 1 + peakDay).toLocaleDateString(currentLanguage(), { weekday: 'long' }) }) }) : null)));

    // Tops
    if (music.plays) {
      const playTracks = (index) => {
        const tracks = music.topTracks.map((x) => lib.byId.get(x.id)).filter(Boolean);
        const target = lib.byId.get(music.topTracks[index].id);
        if (tracks.length && target) player.playQueue(tracks, Math.max(0, tracks.indexOf(target)), { shuffle: false });
      };
      sections.push(h('div.st-grid3', {},
        card(t('stats.topTracks'), rankList(music.topTracks, {
          render: (x, i) => rankRow({
            index: i, art: coverEl({ cover: x.cover, title: x.title, album: x.album }, { small: true }), title: x.title, sub: x.artist || '—',
            value: plays(x.plays), onClick: () => playTracks(i),
          }),
        })),
        card(t('stats.topArtists'), rankList(music.topArtists, {
          render: (x, i) => rankRow({
            index: i, round: true, art: coverEl({ cover: x.cover, title: x.name, isArtist: true }, { small: true, round: true }), title: x.name, sub: fmtDuration(x.seconds),
            value: plays(x.plays), onClick: () => { const a = lib.artists.find((y) => y.name === x.name); if (a) navigate('artist', a.key); },
          }),
        })),
        card(t('stats.topAlbums'), rankList(music.topAlbums, {
          render: (x, i) => rankRow({
            index: i, art: coverEl({ cover: x.cover, title: x.album, album: x.album }, { small: true }), title: x.album, sub: x.artist || '—',
            value: plays(x.plays), onClick: () => { const key = lib.byId.get(x.trackId)?.albumKey; if (key) navigate('album', key); },
          }),
        }))));

      if (music.genres.length) {
        sections.push(card(t('stats.genres'), rankList(music.genres, {
          render: (x, i) => rankRow({ index: i, art: h('span.st-genre-dot', { style: { opacity: String(1 - i * 0.1) } }), title: x.name, sub: fmtDuration(x.seconds), value: plays(x.plays) }),
        })));
      }
    }

    // Films, séries, animés
    if (video.seconds || video.top.length) {
      const tiles = h('div.st-kpis.three', {},
        kpi(t('stats.watched'), fmtDuration(video.seconds), null, 'popcorn'),
        kpi(t('stats.titlesLabel'), count(video.titles), null, 'clapperboard'),
        kpi(t('stats.finished'), count(video.finished), t('stats.finishedSub'), 'circle-check'));
      const posters = h('div.st-posters', {}, ...video.top.slice(0, 8).map((x, i) => {
        const art = x.poster ? h('img', { src: x.poster, alt: '', loading: 'lazy', draggable: 'false' }) : h('div.st-poster-ph', { html: icon('film', 26, 1.4) });
        const el = h('button.st-poster', { type: 'button', title: x.title }, h('span.st-poster-art', {}, art, h('span.st-poster-rank', { text: String(i + 1) })), h('span.st-poster-title', { text: x.title || '—' }), h('span.st-poster-sub', { text: fmtDuration(x.seconds) }));
        el.addEventListener('click', () => {
          if (x.type === 'local') { const tr = lib.byId.get(Number(x.id)); if (tr) player.playQueue([tr], 0); } else navigate('title', `${x.type}:${x.id}`);
        });
        return el;
      }));
      sections.push(h('section.st-block', {},
        h('h2.st-block-title', { text: t('stats.video') }),
        tiles,
        card(t('stats.mostWatched'), posters),
        video.genres.length ? card(t('stats.videoGenres'), rankList(video.genres, {
          render: (x, i) => rankRow({ index: i, art: h('span.st-genre-dot', { style: { opacity: String(1 - i * 0.12) } }), title: x.name, sub: t('stats.titles', { n: x.titles }), value: fmtDuration(x.seconds) }),
        })) : null));
    }

    sections.push(h('div.page-foot'));
    body.replaceChildren(...sections);
  }

  load();
  return { destroy() { destroyed = true; tipEl?.classList.remove('show'); } };
}

// Comparer ses récaps avec ceux d'un autre compte (version web) : chiffres côte à côte, goûts communs, tops, rythmes.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t, currentLanguage } from '../i18n.js';
import { count } from '../format.js';
import { settings, update } from '../settings.js';
import { navigate } from '../router.js';
import { segmented, createScope, withScope, refreshScope } from '../ui/controls.js';
import { openMenu } from '../ui/menu.js';
import { coverEl } from '../ui/cover.js';
import { avatarEl } from '../ui/avatar.js';
import { fmtDuration } from '../stats/format.js';
import { rangeFor } from './stats.js';
import { pageHead, emptyState } from './shared.js';

const web = window.control.web;

/** Anneau du pourcentage de goûts communs. */
function scoreRing(score) {
  const r = 52;
  const c = 2 * Math.PI * r;
  const svg = `<svg viewBox="0 0 120 120" width="132" height="132" aria-hidden="true">
    <circle cx="60" cy="60" r="${r}" fill="none" stroke="currentColor" stroke-opacity="0.14" stroke-width="9"/>
    <circle cx="60" cy="60" r="${r}" fill="none" stroke="var(--accent)" stroke-width="9" stroke-linecap="round"
      stroke-dasharray="${((score / 100) * c).toFixed(1)} ${c.toFixed(1)}" transform="rotate(-90 60 60)"/>
  </svg>`;
  return h('div.cmp-ring', {}, h('div.cmp-ring-svg', { html: svg }), h('div.cmp-ring-text', {}, h('strong', { text: `${score} %` }), h('span', { text: t('cmp.common') })));
}

/** Une ligne « moi | barre | lui » : la barre se partage au prorata des deux valeurs. */
function duelRow(label, a, b, fmt) {
  const total = a + b;
  const left = total ? (a / total) * 100 : 50;
  return h('div.cmp-duel', {},
    h('div.cmp-val.a', { class: a > b ? 'win' : '', text: fmt(a) }),
    h('div.cmp-mid', {}, h('span.cmp-label', { text: label }), h('div.cmp-bar', {}, h('i.a', { style: { width: `${left}%` } }), h('i.b', { style: { width: `${100 - left}%` } }))),
    h('div.cmp-val.b', { class: b > a ? 'win' : '', text: fmt(b) }));
}

/** Deux séries sur les mêmes graduations (jours, heures) : barres jumelées. */
function dualBars(a, b, labels, { title }) {
  const max = Math.max(...a, ...b, 1);
  const groups = a.map((v, i) => h('div.cmp-group', { title: `${labels[i] || ''} — ${fmtDuration(v)} / ${fmtDuration(b[i])}` },
    h('div.cmp-pair', {}, h('i.a', { style: { height: `${v ? Math.max(3, (v / max) * 100) : 0}%` } }), h('i.b', { style: { height: `${b[i] ? Math.max(3, (b[i] / max) * 100) : 0}%` } })),
    h('span', { text: labels[i] || '' })));
  return h('section.cmp-card', {}, h('h2', { text: title }), h('div.cmp-bars', { style: { '--n': a.length } }, ...groups));
}

function topList(items, { round = false, sub }) {
  if (!items.length) return h('p.cmp-none', { text: t('stats.nothing') });
  return h('ol.cmp-top', {}, ...items.map((x, i) => h('li', {},
    h('span.cmp-n', { text: String(i + 1) }),
    h(`div.cmp-art${round ? '.round' : ''}`, {}, coverEl({ cover: x.cover, title: x.name || x.title, isArtist: round }, { small: true, round })),
    h('div.cmp-text', {}, h('strong', { text: x.name || x.title }), sub(x) ? h('span', { text: sub(x) }) : null),
    h('span.cmp-plays', { text: t('stats.plays', { n: count(x.plays) }) }))));
}

export function compareView(root, param) {
  const otherId = String(param || '').split('|')[0];
  let destroyed = false;
  const state = { kind: settings.statsRange || '30d' };
  const body = h('div.cmp.loading');
  const scope = createScope();
  const seg = withScope(scope, () => segmented({
    get: () => state.kind,
    set: (v) => { state.kind = v; update({ statsRange: v }); refreshScope(scope); load(); },
  }, [{ value: '7d', label: t('stats.p7') }, { value: '30d', label: t('stats.p30') }, { value: 'year', label: t('stats.pYear') }, { value: 'all', label: t('stats.pAll') }], { compact: true }));

  const who = h('button.btn.btn-secondary', { type: 'button' }, h('span', { text: t('cmp.with') }), h('span.btn-icon', { html: icon('chevrons-up-down', 14, 2) }));
  who.addEventListener('click', async () => {
    const others = (await web.social.others()).filter((p) => p.access.compare);
    openMenu(who, others.length
      ? others.map((p) => ({ label: p.name, icon: 'user', checked: p.id === otherId, action: () => navigate('compare', p.id) }))
      : [{ label: t('cmp.nobody'), disabled: true }], { align: 'end', minWidth: 200 });
  });

  root.append(pageHead({ title: t('cmp.title'), subtitle: t('cmp.subtitle'), actions: h('div.toolbar-inline', {}, seg, who) }), body, h('div.page-foot'));

  async function load() {
    body.classList.add('loading');
    const range = rangeFor(state.kind);
    let data;
    try {
      data = await web.social.compare(otherId, { from: range.from, to: range.to });
    } catch (err) {
      if (destroyed) return;
      body.classList.remove('loading');
      body.replaceChildren(emptyState({ icon: 'lock', title: t('cmp.deniedTitle'), text: err.message }));
      return;
    }
    if (destroyed) return;
    body.classList.remove('loading');
    paint(data);
  }

  function paint({ with: other, me, other: them, shared, score }) {
    const mine = web.user;
    who.firstChild.textContent = other.name;
    if (!me.plays && !them.plays) {
      body.replaceChildren(emptyState({ icon: 'chart-column', title: t('cmp.emptyTitle'), text: t('cmp.emptyText') }));
      return;
    }
    const day = (i) => new Date(2024, 0, 1 + i).toLocaleDateString(currentLanguage(), { weekday: 'short' }).replace('.', '');
    const verdict = score >= 70 ? t('cmp.verdictHigh') : score >= 40 ? t('cmp.verdictMid') : score > 0 ? t('cmp.verdictLow') : t('cmp.verdictNone');

    const sections = [
      h('header.cmp-head', {},
        h('div.cmp-who', {}, avatarEl(mine, { size: 72 }), h('strong', { text: mine.name }), h('span', { text: t('cmp.you') })),
        h('div.cmp-center', {}, scoreRing(score), h('p', { text: verdict })),
        h('div.cmp-who', {}, avatarEl(other, { size: 72 }), h('strong', { text: other.name }), h('span', { text: `@${other.username}` }))),
      h('section.cmp-card', {},
        h('h2', { text: t('cmp.numbers') }),
        duelRow(t('stats.listened'), me.seconds, them.seconds, fmtDuration),
        duelRow(t('stats.playsLabel'), me.plays, them.plays, count),
        duelRow(t('stats.tracksLabel'), me.tracks, them.tracks, count),
        duelRow(t('stats.artistsLabel'), me.artists, them.artists, count),
        duelRow(t('nav.albums'), me.albums, them.albums, count),
        duelRow(t('cmp.longestStreak'), me.streak.longest, them.streak.longest, (n) => t('stats.days', { n })),
        me.video.seconds || them.video.seconds ? duelRow(t('stats.watched'), me.video.seconds, them.video.seconds, fmtDuration) : null),
    ];

    const sharedBlocks = [];
    if (shared.artists.length) {
      sharedBlocks.push(h('div.cmp-chips', {}, ...shared.artists.map((a) => h('div.cmp-chip', { title: `${a.mine} / ${a.theirs}` },
        h('div.cmp-art.round', {}, coverEl({ cover: a.cover, title: a.name, isArtist: true }, { small: true, round: true })),
        h('strong', { text: a.name })))));
    }
    if (shared.tracks.length) {
      sharedBlocks.push(h('h3', { text: t('cmp.sharedTracks') }), topList(shared.tracks.map((x) => ({ ...x, plays: x.mine + x.theirs })), { sub: (x) => x.artist }));
    }
    if (shared.genres.length) sharedBlocks.push(h('div.cmp-tags', {}, ...shared.genres.map((g) => h('span.cmp-tag', { text: g.name }))));
    sections.push(h('section.cmp-card.cmp-shared', {}, h('h2', { text: t('cmp.sharedTitle') }),
      sharedBlocks.length ? h('div', {}, ...sharedBlocks) : h('p.cmp-none', { text: t('cmp.sharedNone') })));

    sections.push(h('div.cmp-cols', {},
      h('section.cmp-card', {}, h('h2', { text: t('cmp.yourTop') }), h('h3', { text: t('stats.topArtists') }), topList(me.topArtists.slice(0, 5), { round: true, sub: (x) => fmtDuration(x.seconds) }), h('h3', { text: t('stats.topTracks') }), topList(me.topTracks.slice(0, 5), { sub: (x) => x.artist })),
      h('section.cmp-card', {}, h('h2', { text: t('cmp.theirTop', { name: other.name }) }), h('h3', { text: t('stats.topArtists') }), topList(them.topArtists.slice(0, 5), { round: true, sub: (x) => fmtDuration(x.seconds) }), h('h3', { text: t('stats.topTracks') }), topList(them.topTracks.slice(0, 5), { sub: (x) => x.artist }))));

    sections.push(h('div.cmp-cols', {},
      dualBars(me.byWeekday, them.byWeekday, me.byWeekday.map((_, i) => day(i)), { title: t('stats.byWeekday') }),
      dualBars(me.byHour, them.byHour, me.byHour.map((_, i) => (i % 6 === 0 ? `${i} h` : '')), { title: t('stats.byHour') })));

    body.replaceChildren(...sections.filter(Boolean));
  }

  load();
  return { destroy: () => { destroyed = true; } };
}

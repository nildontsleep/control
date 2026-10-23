// Composants des pages Films, séries et animés : affiches, rangées, bannière, états de chargement.

import { h, clamp } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { navigate } from '../router.js';
import { fmtLong } from '../format.js';
import { MediaFail, inList, toggleList, onList, TYPE_ICON } from './api.js';
import { openContextMenu } from '../ui/actions.js';
import { toast } from '../ui/feedback.js';
import { button } from '../ui/controls.js';

export const titleRoute = (item) => navigate('title', `${item.type}:${item.id}`);

const KIND_LABEL = { movie: 'kind.movie', tv: 'kind.tv', anime: 'kind.anime' };
export const kindLabel = (type) => t(KIND_LABEL[type] || 'kind.movie');

function artImg(src, className = '') {
  const img = h(`img${className ? `.${className}` : ''}`, { src, loading: 'lazy', decoding: 'async', alt: '', draggable: 'false', referrerpolicy: 'no-referrer' });
  img.addEventListener('load', () => img.parentElement?.classList.add('loaded'));
  img.addEventListener('error', () => img.remove());
  return img;
}

function placeholder(item) {
  return h('div.pcard-blank', {}, h('span', { html: icon(TYPE_ICON[item.type] || 'film', 28, 1.4) }), h('em', { text: item.title }));
}

export function itemMenu(event, item) {
  openContextMenu(event, [
    { label: t('menu.openTitle'), icon: 'info', action: () => titleRoute(item) },
    { separator: true },
    {
      label: inList(item.type, item.id) ? t('list.remove') : t('list.add'),
      icon: inList(item.type, item.id) ? 'bookmark-check' : 'bookmark-plus',
      action: async () => {
        const added = await toggleList(item);
        toast(added ? t('toast.listAdded', { title: item.title }) : t('toast.listRemoved', { title: item.title }), added ? 'bookmark-check' : 'bookmark');
      },
    },
  ]);
}

/** Affiche 2:3. options : { progress: 0..1, showKind } */
export function posterCard(item, { progress = null, showKind = false } = {}) {
  const art = h('div.pcard-art', {}, placeholder(item));
  if (item.poster) art.append(artImg(item.poster));
  if (item.rating) art.append(h('span.pcard-rating', { html: `${icon('star', 11, 0)}<span>${item.rating.toFixed(1)}</span>` }));
  if (progress != null) art.append(h('div.pcard-progress', {}, h('span', { style: { width: `${Math.round(clamp(progress, 0, 1) * 100)}%` } })));
  const card = h('div.pcard', { tabindex: '0', role: 'link' },
    art,
    h('div.pcard-title', { text: item.title }),
    h('div.pcard-sub', { text: [showKind ? kindLabel(item.type) : null, item.year].filter(Boolean).join(' · ') }));
  card.addEventListener('click', () => titleRoute(item));
  card.addEventListener('keydown', (e) => { if (e.key === 'Enter') titleRoute(item); });
  card.addEventListener('contextmenu', (e) => itemMenu(e, item));
  return card;
}

/** Carte 16:9 de « Continuer à regarder ». */
export function resumeCard(entry, { onPlay, onRemove }) {
  const left = Math.max(0, entry.duration - entry.position);
  const label = entry.season != null ? `S${entry.season} · E${entry.episode}` : entry.episode != null ? `${t('ep.short')} ${entry.episode}` : null;
  const art = h('div.rcard-art', {}, placeholder({ type: entry.type, title: entry.title }));
  const src = entry.backdrop || entry.poster;
  if (src) art.append(artImg(src));
  art.append(
    h('button.rcard-play', { type: 'button', 'data-tip': t('common.play'), html: icon('play', 22, 0), onclick: (e) => { e.stopPropagation(); onPlay(); } }),
    h('div.pcard-progress.thick', {}, h('span', { style: { width: `${entry.duration ? Math.round(clamp(entry.position / entry.duration, 0, 1) * 100) : 0}%` } })));
  const card = h('div.rcard', { tabindex: '0' },
    art,
    h('div.rcard-title', { text: entry.title }),
    h('div.rcard-sub', { text: [label, left ? t('resume.left', { t: fmtLong(left) }) : null].filter(Boolean).join(' · ') }));
  card.addEventListener('click', onPlay);
  card.addEventListener('keydown', (e) => { if (e.key === 'Enter') onPlay(); });
  card.addEventListener('contextmenu', (e) => openContextMenu(e, [
    { label: t('common.play'), icon: 'play', action: onPlay },
    { label: t('menu.openTitle'), icon: 'info', action: () => navigate('title', `${entry.type}:${entry.id}`) },
    { separator: true },
    { label: t('resume.remove'), icon: 'x', action: onRemove },
  ]));
  return card;
}

/** Rangée horizontale avec flèches. */
export function shelf({ title, items, onMore, build }) {
  const row = h('div.shelf-row', {}, ...items.map(build));
  const prev = h('button.shelf-arrow.prev', { type: 'button', 'aria-label': t('shelf.prev'), html: icon('chevron-left', 22, 2.2) });
  const next = h('button.shelf-arrow.next', { type: 'button', 'aria-label': t('shelf.next'), html: icon('chevron-right', 22, 2.2) });
  const scrollBy = (dir) => row.scrollBy({ left: dir * row.clientWidth * 0.85, behavior: 'smooth' });
  prev.addEventListener('click', () => scrollBy(-1));
  next.addEventListener('click', () => scrollBy(1));
  const sync = () => {
    prev.classList.toggle('hide', row.scrollLeft < 8);
    next.classList.toggle('hide', row.scrollLeft + row.clientWidth >= row.scrollWidth - 8);
  };
  row.addEventListener('scroll', sync, { passive: true });
  requestAnimationFrame(sync);
  new ResizeObserver(sync).observe(row);
  return h('section.shelf', {},
    h('div.shelf-head', {}, h('h2', { text: title }), onMore ? h('button.shelf-more', { type: 'button', onclick: onMore }, h('span', { text: t('shelf.seeAll') }), h('span', { html: icon('chevron-right', 14, 2.4) })) : null),
    h('div.shelf-body', {}, prev, row, next));
}

export const posterShelf = (opts) => shelf({ ...opts, build: (item) => posterCard(item, opts.card) });

/** Bannière d'accueil : quelques titres en rotation. */
export function heroBanner(items, { onPlay }) {
  const slides = items.filter((i) => i.backdrop).slice(0, 5);
  if (!slides.length) return null;
  let index = 0;
  let timer = null;

  const layers = slides.map((item, i) => {
    const el = h('div.hero-slide', {}, artImg(item.backdrop), h('div.hero-shade'));
    if (i === 0) el.classList.add('on');
    return el;
  });
  const info = h('div.hero-info');
  const dots = h('div.hero-dots', {}, ...slides.map((_, i) => h('button.hero-dot', { type: 'button', 'aria-label': String(i + 1), onclick: () => go(i, true) })));

  function paintInfo() {
    const item = slides[index];
    const meta = [kindLabel(item.type), item.year, item.rating ? `★ ${item.rating.toFixed(1)}` : null].filter(Boolean).join('  ·  ');
    const listed = inList(item.type, item.id);
    info.replaceChildren(
      h('div.hero-meta', { text: meta }),
      h('h2.hero-title', { text: item.title }),
      item.overview ? h('p.hero-overview', { text: item.overview }) : null,
      h('div.hero-actions', {},
        button(t('common.play'), { icon: 'play', kind: 'primary', onClick: () => onPlay(item) }),
        button(t('hero.moreInfo'), { icon: 'info', onClick: () => titleRoute(item) }),
        button(listed ? t('list.inList') : t('list.add'), { icon: listed ? 'bookmark-check' : 'bookmark-plus', onClick: async () => { await toggleList(item); } })));
    [...dots.children].forEach((d, i) => d.classList.toggle('on', i === index));
    info.classList.remove('swap');
    void info.offsetWidth;
    info.classList.add('swap');
  }

  function go(i, manual = false) {
    layers[index].classList.remove('on');
    index = (i + slides.length) % slides.length;
    layers[index].classList.add('on');
    paintInfo();
    if (manual) restart();
  }

  function restart() {
    clearInterval(timer);
    if (slides.length > 1) timer = setInterval(() => go(index + 1), 8000);
  }

  const el = h('section.hero-banner', {}, ...layers, info, slides.length > 1 ? dots : null);
  el.addEventListener('pointerenter', () => clearInterval(timer));
  el.addEventListener('pointerleave', restart);
  const offList = onList(paintInfo);
  paintInfo();
  restart();
  el.destroy = () => {
    clearInterval(timer);
    offList();
  };
  return el;
}

export function genreChips(genres, { onPick }) {
  return h('div.genre-chips', {}, ...genres.map((g) => h('button.chip', { type: 'button', text: g.label || g.name, onclick: () => onPick(g) })));
}

export function skeletonShelf() {
  return h('section.shelf.skeleton', {}, h('div.shelf-head', {}, h('h2.sk', { text: ' '.repeat(18) })),
    h('div.shelf-row', {}, ...Array.from({ length: 9 }, () => h('div.pcard', {}, h('div.pcard-art.sk'), h('div.pcard-title.sk', { text: ' ' })))));
}

const ERROR_KEYS = {
  nokey: 'err.nokey', auth: 'err.auth', network: 'err.network', timeout: 'err.network', ratelimit: 'err.ratelimit', notfound: 'err.notfound',
};

export function errorState(error, onRetry) {
  const code = error instanceof MediaFail ? error.code : 'error';
  return h('div.empty.compact', {},
    h('div.empty-art', { html: icon('triangle-alert', 30, 1.5) }),
    h('h2', { text: t('err.title') }),
    h('p', { text: ERROR_KEYS[code] ? t(ERROR_KEYS[code]) : (error?.message || t('err.unknown')) }),
    onRetry ? h('div.empty-actions', {}, button(t('action.retry'), { icon: 'refresh-cw', kind: 'primary', onClick: onRetry })) : null);
}

/** Libellés français des genres AniList (anglais en anglais). */
const ANIME_GENRES_FR = {
  Action: 'Action', Adventure: 'Aventure', Comedy: 'Comédie', Drama: 'Drame', Ecchi: 'Ecchi', Fantasy: 'Fantastique', Horror: 'Horreur',
  'Mahou Shoujo': 'Magical girl', Mecha: 'Mecha', Music: 'Musique', Mystery: 'Mystère', Psychological: 'Psychologique', Romance: 'Romance',
  'Sci-Fi': 'Science-fiction', 'Slice of Life': 'Tranche de vie', Sports: 'Sport', Supernatural: 'Surnaturel', Thriller: 'Thriller',
};
export const animeGenreLabel = (name, lang) => (lang === 'fr' ? ANIME_GENRES_FR[name] || name : name);

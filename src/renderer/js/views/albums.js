// Vues « Albums » (grille) et détail d’un album.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { fmtLong } from '../format.js';
import { lib, cmp } from '../data.js';
import { settings, update } from '../settings.js';
import { navigate } from '../router.js';
import { player } from '../player.js';
import { createTrackList } from '../ui/tracklist.js';
import { albumCard } from '../ui/cards.js';
import { coverEl } from '../ui/cover.js';
import { openContextMenu, trackMenuItems } from '../ui/actions.js';
import { popup, slider } from '../ui/controls.js';
import { pageHead, playButtons, emptyLibrary } from './shared.js';

const SORTERS = {
  title: (a, b) => cmp(a.title, b.title),
  artist: (a, b) => cmp(a.artist || '~', b.artist || '~') || cmp(a.title, b.title),
  year: (a, b) => (b.year || 0) - (a.year || 0) || cmp(a.title, b.title),
  addedAt: (a, b) => b.addedAt - a.addedAt,
  playCount: (a, b) => b.playCount - a.playCount || cmp(a.title, b.title),
};

export function albumGrid(albums, { showArtist = true } = {}) {
  return h('div.grid.albums', {}, ...albums.map((a) => albumCard(a, { showArtist })));
}

export function albumsView(root) {
  if (!lib.albums.length) {
    root.append(pageHead({ title: t('nav.albums') }), emptyLibrary());
    return {};
  }
  const apply = () => root.style.setProperty('--album-size', `${settings.albumSize}px`);
  const build = () => {
    const sorted = [...lib.albums].sort(SORTERS[settings.sort.albums] || SORTERS.title);
    const sortOptions = ['title', 'artist', 'year', 'addedAt', 'playCount'].map((k) => ({ value: k, label: t(`sort.${k}`) }));
    const sortPopup = popup({ get: () => settings.sort.albums, set: (v) => { update({ sort: { albums: v } }); build(); } }, sortOptions);
    const { input } = slider('albumSize', { min: 140, max: 280, step: 5, format: () => '' });
    input.classList.add('size-slider');
    input.addEventListener('input', apply);
    root.replaceChildren(
      pageHead({
        title: t('nav.albums'),
        subtitle: t('count.albums', { n: sorted.length }),
        actions: h('div.toolbar-inline', {}, h('span.toolbar-label', { text: t('sort.by') }), sortPopup, h('span.size-icons', {}, h('span.size-small', { html: icon('grid-2x2', 13, 2) }), input, h('span.size-large', { html: icon('layout-grid', 16, 2) }))),
      }),
      albumGrid(sorted),
      h('div.page-foot'),
    );
  };
  apply();
  build();
  return {};
}

export function albumView(root, key, { scroller }) {
  const album = lib.albumByKey.get(key);
  if (!album) {
    root.append(pageHead({ title: t('nav.albums') }));
    return {};
  }
  const meta = [album.genre || album.tracks[0]?.genre, album.year].filter(Boolean).join(' · ');
  const artistEntry = album.various ? null : lib.artists.find((a) => a.name === (album.albumArtist || album.artist));
  const items = album.tracks.map((track) => ({ track }));
  const list = createTrackList({ items, layout: 'album', scroller });
  const discs = new Set(album.tracks.map((x) => x.discNo || 1));

  const art = coverEl({ ...album, album: album.title }, { className: 'hero-cover' });
  root.append(
    h('header.hero', {},
      h('div.hero-art', {}, art),
      h('div.hero-text', {},
        h('div.hero-kind', { text: t('album.kind') }),
        h('h1', { text: album.title }),
        album.various
          ? h('div.hero-artist.plain', { text: t('album.various') })
          : h('button.hero-artist.link', { type: 'button', text: album.artist || t('album.unknown'), onclick: () => artistEntry && navigate('artist', artistEntry.key) }),
        h('p.hero-meta', { text: [meta, t('count.tracks', { n: album.tracks.length }), fmtLong(album.duration), discs.size > 1 ? t('album.discs', { n: discs.size }) : null].filter(Boolean).join(' · ') }),
        h('div.hero-actions', {}, ...playButtons(() => album.tracks)))),
    list.el,
    h('div.page-foot'),
  );
  art.addEventListener('contextmenu', (e) => openContextMenu(e, trackMenuItems(album.tracks, { play: () => player.playQueue(album.tracks, 0, { shuffle: false }) })));
  return { destroy: () => list.destroy() };
}

// Vues « Artistes » (grille) et détail d’un artiste.

import { h } from '../dom.js';
import { t } from '../i18n.js';
import { fmtLong } from '../format.js';
import { lib, cmp } from '../data.js';
import { createTrackList } from '../ui/tracklist.js';
import { artistCard } from '../ui/cards.js';
import { coverEl } from '../ui/cover.js';
import { albumGrid } from './albums.js';
import { pageHead, playButtons, section, emptyLibrary } from './shared.js';

export function artistsView(root) {
  if (!lib.artists.length) {
    root.append(pageHead({ title: t('nav.artists') }), emptyLibrary());
    return {};
  }
  const sorted = [...lib.artists].sort((a, b) => cmp(a.name, b.name));
  root.append(
    pageHead({ title: t('nav.artists'), subtitle: t('count.artists', { n: sorted.length }) }),
    h('div.grid.artists', {}, ...sorted.map(artistCard)),
    h('div.page-foot'),
  );
  return {};
}

export function artistView(root, key, { scroller }) {
  const artist = lib.artistByKey.get(key);
  if (!artist) {
    root.append(pageHead({ title: t('nav.artists') }));
    return {};
  }
  const items = artist.tracks.map((track) => ({ track }));
  const list = createTrackList({ items, layout: 'artist', scroller });
  const total = artist.tracks.length;
  root.append(...[
    h('header.hero.round', {},
      h('div.hero-art', {}, coverEl({ cover: artist.cover, title: artist.name, isArtist: true }, { className: 'hero-cover', round: true })),
      h('div.hero-text', {},
        h('div.hero-kind', { text: t('artist.kind') }),
        h('h1', { text: artist.name }),
        h('p.hero-meta', { text: [t('count.albums', { n: artist.albums.length }), t('count.tracks', { n: total }), fmtLong(artist.duration)].join(' · ') }),
        h('div.hero-actions', {}, ...playButtons(() => artist.tracks)))),
    artist.albums.length ? section(t('nav.albums'), albumGrid(artist.albums, { showArtist: false })) : null,
    section(t('artist.songs'), list.el),
    h('div.page-foot'),
  ].filter(Boolean));
  return { destroy: () => list.destroy() };
}

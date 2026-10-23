// Vue « Recherche » : morceaux, albums, artistes, vidéos et playlists.

import { h } from '../dom.js';
import { t } from '../i18n.js';
import { search } from '../data.js';
import { navigate } from '../router.js';
import { createTrackList } from '../ui/tracklist.js';
import { albumCard, artistCard, videoCard, mosaic } from '../ui/cards.js';
import { playlistRows } from '../data.js';
import { pageHead, section, emptyState } from './shared.js';

export function searchView(root, query, { scroller }) {
  const result = search(query || '');
  if (!result) {
    root.append(pageHead({ title: t('search.title') }), emptyState({ icon: 'search', title: t('search.startTitle'), text: t('search.startText') }));
    return {};
  }
  const total = result.tracks.length + result.albums.length + result.artists.length + result.videos.length + result.playlists.length;
  root.append(pageHead({ title: t('search.resultsFor', { q: query }), subtitle: total ? null : null }));
  if (!total) {
    root.append(emptyState({ icon: 'search', title: t('search.noneTitle'), text: t('search.noneText', { q: query }) }));
    return {};
  }
  let list = null;
  if (result.artists.length) root.append(section(t('nav.artists'), h('div.grid.artists.compact', {}, ...result.artists.slice(0, 12).map(artistCard))));
  if (result.albums.length) root.append(section(t('nav.albums'), h('div.grid.albums.compact', {}, ...result.albums.slice(0, 12).map((a) => albumCard(a)))));
  if (result.playlists.length) {
    root.append(section(t('nav.playlists'), h('div.grid.albums.compact', {}, ...result.playlists.map((p) => {
      const tracks = playlistRows(p).map((r) => r.track);
      const card = h('div.card.album-card', { tabindex: '0' },
        h('div.card-art', {}, mosaic(tracks)),
        h('div.card-title', { text: p.name }),
        h('div.card-sub', { text: t('count.tracks', { n: tracks.length }) }));
      card.addEventListener('click', () => navigate('playlist', p.id));
      return card;
    }))));
  }
  if (result.videos.length) root.append(section(t('nav.videos'), h('div.grid.videos.compact', {}, ...result.videos.map((v) => videoCard(v, result.videos)))));
  if (result.tracks.length) {
    list = createTrackList({ items: result.tracks.map((track) => ({ track })), layout: 'search', scroller });
    root.append(section(t('nav.songs'), list.el));
  }
  root.append(h('div.page-foot'));
  return { destroy: () => list?.destroy() };
}

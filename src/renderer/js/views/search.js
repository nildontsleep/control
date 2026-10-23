// Vue « Recherche » : morceaux, albums, artistes, vidéos et playlists.

import { h } from '../dom.js';
import { t } from '../i18n.js';
import { search } from '../data.js';
import { navigate } from '../router.js';
import { createTrackList } from '../ui/tracklist.js';
import { albumCard, artistCard, videoCard, mosaic } from '../ui/cards.js';
import { playlistRows } from '../data.js';
import { pageHead, section, emptyState } from './shared.js';
import { media } from '../media/api.js';
import { posterCard } from '../media/ui.js';
import { button } from '../ui/controls.js';
import { openSettings } from '../ui/settings-sheet.js';

export function searchView(root, query, { scroller }) {
  const result = search(query || '');
  if (!result) {
    root.append(pageHead({ title: t('search.title') }), emptyState({ icon: 'search', title: t('search.startTitle'), text: t('search.startText') }));
    return {};
  }
  const total = result.tracks.length + result.albums.length + result.artists.length + result.videos.length + result.playlists.length;
  let destroyed = false;
  let list = null;
  root.append(pageHead({ title: t('search.resultsFor', { q: query }) }));

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

  // Catalogues en ligne (films, séries, animés) : chargés après les résultats locaux.
  const online = h('div.online');
  const none = !total ? h('div.search-none') : null;
  root.append(online, none, h('div.page-foot'));
  if (String(query).trim().length >= 2) {
    online.append(h('div.dl-hint', {}, h('span.spinner.small', {}, ...Array.from({ length: 12 }, (_, i) => h('i', { style: { '--i': i } }))), h('span', { text: t('search.online') })));
    media.search(query).then((res) => {
      if (destroyed) return;
      const parts = [];
      if (res.tmdb.length) parts.push(section(t('nav.movies'), h('div.grid.posters.compact', {}, ...res.tmdb.map((i) => posterCard(i, { showKind: true })))));
      if (res.anime.length) parts.push(section(t('nav.anime'), h('div.grid.posters.compact', {}, ...res.anime.map((i) => posterCard(i)))));
      if (res.tmdbError === 'nokey') {
        parts.push(h('div.search-hint', {}, h('span', { text: t('search.nokey') }), button(t('search.addKey'), { small: true, kind: 'plain', onClick: () => openSettings('streaming') })));
      }
      online.replaceChildren(...parts);
      if (none && !res.tmdb.length && !res.anime.length) none.replaceChildren(...emptyNone(query).childNodes);
    }).catch(() => {
      if (destroyed) return;
      online.replaceChildren();
      if (none) none.replaceChildren(...emptyNone(query).childNodes);
    });
  } else if (none) {
    none.replaceChildren(...emptyNone(query).childNodes);
  }
  return { destroy: () => { destroyed = true; list?.destroy(); } };
}

const emptyNone = (query) => emptyState({ icon: 'search', title: t('search.noneTitle'), text: t('search.noneText', { q: query }) });

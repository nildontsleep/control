// Cartes (albums, artistes, vidéos, playlists) avec bouton de lecture au survol.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { fmtTime, fmtLong } from '../format.js';
import { player } from '../player.js';
import { navigate } from '../router.js';
import { coverEl } from './cover.js';
import { openContextMenu, trackMenuItems, startTrackDrag } from './actions.js';

function playOverlay(onPlay) {
  return h('button.card-play', {
    type: 'button',
    'data-tip': t('menu.play'),
    html: icon('play', 20, 0),
    onclick: (e) => {
      e.stopPropagation();
      onPlay();
    },
  });
}

export function albumCard(album, { showArtist = true } = {}) {
  const play = () => player.playQueue(album.tracks, 0, { shuffle: false });
  const sub = [showArtist ? (album.various ? t('album.various') : album.artist) : null, album.year].filter(Boolean).join(' · ');
  const card = h('div.card.album-card', { tabindex: '0', draggable: 'true' },
    h('div.card-art', {}, coverEl({ ...album, title: album.title, album: album.title }), playOverlay(play)),
    h('div.card-title', { text: album.title }),
    h('div.card-sub', { text: sub }));
  card.addEventListener('click', () => navigate('album', album.key));
  card.addEventListener('keydown', (e) => { if (e.key === 'Enter') navigate('album', album.key); });
  card.addEventListener('contextmenu', (e) => openContextMenu(e, trackMenuItems(album.tracks, { play })));
  card.addEventListener('dragstart', (e) => startTrackDrag(e, album.tracks));
  return card;
}

export function artistCard(artist) {
  const play = () => player.playQueue(artist.tracks, 0, { shuffle: true });
  const card = h('div.card.artist-card', { tabindex: '0' },
    h('div.card-art.round', {}, coverEl({ cover: artist.cover, title: artist.name, isArtist: true }, { round: true }), playOverlay(play)),
    h('div.card-title.center', { text: artist.name }),
    h('div.card-sub.center', { text: t('artist.count', { n: artist.tracks.length }) }));
  card.addEventListener('click', () => navigate('artist', artist.key));
  card.addEventListener('keydown', (e) => { if (e.key === 'Enter') navigate('artist', artist.key); });
  card.addEventListener('contextmenu', (e) => openContextMenu(e, trackMenuItems(artist.tracks, { play })));
  return card;
}

export function videoCard(track, list) {
  const play = () => player.playQueue(list, list.indexOf(track), { shuffle: false });
  const card = h('div.card.video-card', { tabindex: '0', draggable: 'true' },
    h('div.card-art.wide', {}, coverEl(track, { className: 'wide' }),
      h('span.card-duration', { text: fmtTime(track.duration) }),
      playOverlay(play)),
    h('div.card-title', { text: track.title }),
    h('div.card-sub', { text: [track.height ? `${track.height}p` : null, track.codec].filter(Boolean).join(' · ') }));
  card.addEventListener('click', play);
  card.addEventListener('keydown', (e) => { if (e.key === 'Enter') play(); });
  card.addEventListener('contextmenu', (e) => openContextMenu(e, trackMenuItems([track], { play })));
  card.addEventListener('dragstart', (e) => startTrackDrag(e, [track]));
  return card;
}

/** Mosaïque 2×2 des pochettes d’une playlist. */
export function mosaic(tracks, { size = 'l' } = {}) {
  const seen = new Set();
  const covers = [];
  for (const track of tracks) {
    if (track.cover && !seen.has(track.cover)) {
      seen.add(track.cover);
      covers.push(track);
    }
    if (covers.length === 4) break;
  }
  if (covers.length < 4) {
    const only = covers[0] || tracks[0];
    return coverEl({ cover: only?.cover, title: only?.title || '', isPlaylist: true }, { small: size === 's' });
  }
  return h('div.mosaic', {}, ...covers.map((c) => coverEl(c, { small: true })));
}

export function statLine(tracks) {
  const duration = tracks.reduce((s, x) => s + x.duration, 0);
  return `${t('count.tracks', { n: tracks.length })} · ${fmtLong(duration)}`;
}

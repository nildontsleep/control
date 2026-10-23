// Vue « Morceaux » : toute la musique, triable, virtualisée.

import { h } from '../dom.js';
import { t } from '../i18n.js';
import { lib, cmp } from '../data.js';
import { settings, update } from '../settings.js';
import { createTrackList } from '../ui/tracklist.js';
import { pageHead, playButtons, emptyLibrary } from './shared.js';
import { statLine } from '../ui/cards.js';

export function sortTracks(list, { key, dir }) {
  const sign = dir === 'desc' ? -1 : 1;
  const text = (v) => v || '';
  return [...list].sort((a, b) => {
    let r;
    if (key === 'playCount' || key === 'duration' || key === 'addedAt') r = (a[key] || 0) - (b[key] || 0);
    else r = cmp(text(a[key]), text(b[key])) || cmp(text(a.title), text(b.title));
    return sign * r || cmp(a.title, b.title);
  });
}

export function songsView(root, _param, { scroller }) {
  if (!lib.audio.length) {
    root.append(pageHead({ title: t('nav.songs') }), emptyLibrary());
    return {};
  }
  let list = null;
  const build = () => {
    const sort = settings.sort.songs;
    const sorted = sortTracks(lib.audio, sort);
    const items = sorted.map((track) => ({ track }));
    list?.destroy();
    root.replaceChildren(
      pageHead({ title: t('nav.songs'), subtitle: statLine(sorted), actions: playButtons(() => sorted) }),
      (list = createTrackList({
        items,
        layout: 'songs',
        scroller,
        sort,
        onSort: (key) => {
          const cur = settings.sort.songs;
          update({ sort: { songs: { key, dir: cur.key === key && cur.dir === 'asc' ? 'desc' : 'asc' } } });
          const top = scroller.scrollTop;
          build();
          scroller.scrollTop = top;
        },
      })).el,
      h('div.page-foot'),
    );
  };
  build();
  return { destroy: () => list?.destroy() };
}

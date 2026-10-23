// Vue « Vidéos » : grille de miniatures.

import { h } from '../dom.js';
import { t } from '../i18n.js';
import { lib, cmp } from '../data.js';
import { settings, update } from '../settings.js';
import { videoCard } from '../ui/cards.js';
import { popup } from '../ui/controls.js';
import { fmtLong } from '../format.js';
import { pageHead, playButtons, emptyState } from './shared.js';
import { navigate } from '../router.js';
import { button } from '../ui/controls.js';

const SORTERS = {
  addedAt: (a, b) => b.addedAt - a.addedAt,
  title: (a, b) => cmp(a.title, b.title),
  duration: (a, b) => b.duration - a.duration,
  playCount: (a, b) => b.playCount - a.playCount || cmp(a.title, b.title),
};

export function videosView(root) {
  if (!lib.videos.length) {
    root.append(
      pageHead({ title: t('nav.videos') }),
      emptyState({
        icon: 'film',
        title: t('empty.videosTitle'),
        text: t('empty.videosText'),
        actions: [button(t('empty.download'), { icon: 'cloud-download', kind: 'primary', onClick: () => navigate('downloads') })],
      }),
    );
    return {};
  }
  const build = () => {
    const sorted = [...lib.videos].sort(SORTERS[settings.sort.videos] || SORTERS.addedAt);
    const options = ['addedAt', 'title', 'duration', 'playCount'].map((k) => ({ value: k, label: t(`sort.${k}`) }));
    const sortPopup = popup({ get: () => settings.sort.videos, set: (v) => { update({ sort: { videos: v } }); build(); } }, options);
    root.replaceChildren(
      pageHead({
        title: t('nav.videos'),
        subtitle: `${t('count.videos', { n: sorted.length })} · ${fmtLong(sorted.reduce((sum, v) => sum + v.duration, 0))}`,
        actions: [h('div.toolbar-inline', {}, h('span.toolbar-label', { text: t('sort.by') }), sortPopup), ...playButtons(() => sorted)],
      }),
      h('div.grid.videos', {}, ...sorted.map((v) => videoCard(v, sorted))),
      h('div.page-foot'),
    );
  };
  build();
  return {};
}

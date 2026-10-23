// Éléments communs aux vues : en-tête de page, boutons de lecture, états vides.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { player } from '../player.js';
import { navigate } from '../router.js';
import { button } from '../ui/controls.js';

const library = window.control.library;

export function pageHead({ title, subtitle, actions, className = '' }) {
  return h(`header.page-head${className ? `.${className}` : ''}`, {},
    h('div.page-titles', {}, h('h1', { text: title }), subtitle ? h('p.page-sub', { text: subtitle }) : null),
    actions ? h('div.page-actions', {}, ...[].concat(actions)) : null);
}

export function playButtons(getTracks) {
  const guard = (fn) => () => {
    const tracks = getTracks();
    if (tracks.length) fn(tracks);
  };
  return [
    button(t('common.play'), { icon: 'play', kind: 'primary', onClick: guard((tracks) => player.playQueue(tracks, 0, { shuffle: false })) }),
    button(t('common.shuffle'), { icon: 'shuffle', kind: 'secondary', onClick: guard((tracks) => player.playQueue(tracks, 0, { shuffle: true })) }),
  ];
}

export function section(title, ...children) {
  return h('section.block', {}, title ? h('h2.block-title', { text: title }) : null, ...children);
}

export function emptyState({ icon: ic = 'music', title, text, actions = [] }) {
  return h('div.empty', {},
    h('div.empty-art', { html: icon(ic, 34, 1.5) }),
    h('h2', { text: title }),
    text ? h('p', { text }) : null,
    actions.length ? h('div.empty-actions', {}, ...actions) : null);
}

export function emptyLibrary() {
  return emptyState({
    icon: 'headphones',
    title: t('empty.libraryTitle'),
    text: t('empty.libraryText'),
    actions: [
      button(t('empty.addMusic'), { icon: 'folder-plus', kind: 'primary', onClick: () => library.addMusicFolder() }),
      button(t('empty.addFolder'), { icon: 'folder-open', onClick: () => library.addFolder() }),
      button(t('empty.download'), { icon: 'cloud-download', onClick: () => navigate('downloads') }),
    ],
  });
}

// Profil d'un compte (version web) : présentation, chiffres de l'année, playlists partagées, accès à sa bibliothèque et
// comparaison des récaps. Pour le sien : modification du profil et du partage.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t, currentLanguage } from '../i18n.js';
import { count } from '../format.js';
import { navigate } from '../router.js';
import { button } from '../ui/controls.js';
import { avatarEl, onMyProfile } from '../ui/avatar.js';
import { playlistCover } from '../ui/cards.js';
import { openProfileEditor, openSharingEditor } from '../ui/profile-edit.js';
import { fmtDuration } from '../stats/format.js';
import { accessChips } from './people.js';
import { emptyState } from './shared.js';

const web = window.control.web;

export function profileView(root, param) {
  const id = param || web.user.id;
  const self = id === web.user.id;
  let destroyed = false;
  const body = h('div.pf.loading');
  root.append(body, h('div.page-foot'));
  let offProfile = null;

  async function load() {
    let data;
    try {
      data = await web.social.profile(id);
    } catch (err) {
      body.replaceChildren(emptyState({ icon: 'triangle-alert', title: t('err.title'), text: err.message }));
      body.classList.remove('loading');
      return;
    }
    if (destroyed) return;
    body.classList.remove('loading');
    paint(data);
  }

  function paint({ profile, access, library, playlists, stats }) {
    if (profile.color) body.style.setProperty('--pf', profile.color);
    else body.style.removeProperty('--pf');
    const since = new Date(profile.createdAt).toLocaleDateString(currentLanguage(), { month: 'long', year: 'numeric' });
    const actions = [];
    if (self) {
      actions.push(
        button(t('profile.edit'), { icon: 'square-pen', kind: 'primary', onClick: () => openProfileEditor(profile, load) }),
        button(t('profile.sharing'), { icon: 'users', onClick: () => openSharingEditor(load) }));
    } else {
      if (access.browse) actions.push(button(t('profile.browse'), { icon: 'headphones', kind: 'primary', onClick: () => web.switchLibrary(id) }));
      if (access.compare) actions.push(button(t('profile.compare'), { icon: 'chart-column', kind: access.browse ? 'secondary' : 'primary', onClick: () => navigate('compare', id) }));
    }

    const blocks = [];
    if (stats) {
      blocks.push(h('section.pf-block', {},
        h('h2', { text: t('profile.thisYear', { year: stats.year }) }),
        h('div.pf-kpis', {},
          h('div.pf-kpi', {}, h('strong', { text: fmtDuration(stats.seconds) }), h('span', { text: t('stats.listened') })),
          h('div.pf-kpi', {}, h('strong', { text: count(stats.plays) }), h('span', { text: t('stats.playsLabel') })),
          h('div.pf-kpi', {}, h('strong', { text: t('stats.days', { n: stats.streak.current }) }), h('span', { text: t('stats.streak') }))),
        stats.topArtists.length ? h('p.pf-top', { text: `${t('profile.topArtists')} ${stats.topArtists.join(' · ')}` }) : null));
    }
    if (library) {
      blocks.push(h('section.pf-block', {},
        h('h2', { text: t('people.library') }),
        h('div.pf-kpis', {},
          h('div.pf-kpi', {}, h('strong', { text: count(library.tracks) }), h('span', { text: t('nav.songs') })),
          h('div.pf-kpi', {}, h('strong', { text: count(library.albums) }), h('span', { text: t('nav.albums') })),
          h('div.pf-kpi', {}, h('strong', { text: count(library.artists) }), h('span', { text: t('nav.artists') })))));
      if (playlists.length) {
        blocks.push(h('section.pf-block', {},
          h('h2', { text: t('profile.playlists') }),
          h('div.pf-playlists', {}, ...playlists.map((p) => h('button.pf-pl', { type: 'button', onclick: () => { window.location.assign(`/?lib=${encodeURIComponent(id)}#playlist/${p.id}`); } },
            h('div.pf-pl-cover', {}, playlistCover(p, [], { size: 's' })),
            h('strong', { text: p.name }),
            h('span', { text: p.description || t('count.tracks', { n: p.count }) }))))));
      }
    }
    if (!blocks.length && !self) blocks.push(emptyState({ icon: 'lock', title: t('profile.privateTitle'), text: t('profile.privateText') }));

    body.replaceChildren(
      h('header.pf-hero', {},
        avatarEl(profile, { size: 128, className: 'pf-avatar' }),
        h('div.pf-id', {},
          h('h1', { text: profile.name }),
          h('p.pf-user', { text: `@${profile.username}${profile.role === 'admin' ? ` · ${t('web.roleAdmin')}` : ''} · ${t('profile.since', { date: since })}` }),
          profile.bio ? h('p.pf-bio', { text: profile.bio }) : null,
          self ? null : accessChips(access),
          h('div.pf-actions', {}, ...actions))),
      ...blocks);
  }

  if (self) offProfile = onMyProfile(() => load());
  load();
  return { destroy: () => { destroyed = true; offProfile?.(); } };
}

// Personnes (version web) : les comptes du serveur, ce qu'ils partagent avec moi, et l'accès à leur profil.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { navigate } from '../router.js';
import { avatarEl } from '../ui/avatar.js';
import { pageHead, emptyState } from './shared.js';

const web = window.control.web;

/** Les accès partagés avec moi, en deux pastilles. */
export function accessChips(access) {
  if (!access) return null;
  return h('div.pp-chips', {},
    access.browse ? h('span.pp-chip', { html: `${icon('headphones', 12, 2)}<span>${t('people.library')}</span>` }) : null,
    access.compare ? h('span.pp-chip', { html: `${icon('chart-column', 12, 2)}<span>${t('people.recaps')}</span>` }) : null);
}

export function peopleView(root) {
  let destroyed = false;
  const body = h('div.pp-grid.loading');
  root.append(pageHead({ title: t('people.title'), subtitle: t('people.subtitle') }), body, h('div.page-foot'));

  web.social.others().then((others) => {
    if (destroyed) return;
    body.classList.remove('loading');
    const card = (person, { self = false } = {}) => h('button.pp-card', { type: 'button', style: person.color ? { '--accent-card': person.color } : null, onclick: () => navigate('profile', person.id) },
      avatarEl(person, { size: 72 }),
      h('div.pp-name', {}, h('strong', { text: person.name }), self ? h('span.pp-me', { text: t('people.you') }) : null),
      h('span.pp-user', { text: `@${person.username}` }),
      person.bio ? h('p.pp-bio', { text: person.bio }) : null,
      self ? null : accessChips(person.access));
    const mine = web.user;
    body.replaceChildren(
      card({ ...mine }, { self: true }),
      ...others.map((p) => card(p)));
    if (!others.length) body.append(emptyState({ icon: 'users', title: t('people.noneTitle'), text: t('people.noneText') }));
  }).catch((err) => {
    if (!destroyed) body.replaceChildren(emptyState({ icon: 'triangle-alert', title: t('err.title'), text: err.message }));
  });

  return { destroy: () => { destroyed = true; } };
}

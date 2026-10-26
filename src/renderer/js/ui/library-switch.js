// Version web : passer de sa bibliothèque à celle d'un compte qui la partage (lecture seule), et le bandeau qui le rappelle.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { navigate } from '../router.js';
import { openMenu } from './menu.js';
import { avatarEl, onMyProfile } from './avatar.js';

const web = window.control.web;

export function mountLibrarySwitcher(container) {
  const viewing = web.viewing;
  if (!web.libraries.length && !viewing) {
    container.hidden = true;
    return;
  }
  const who = viewing || web.user;
  let face = avatarEl(who, { size: 22 });
  const btn = h('button.src-btn.lib-btn', { type: 'button' },
    face,
    h('span.src-name', { text: viewing ? t('lib.of', { name: viewing.name }) : t('lib.mine') }),
    h('span.src-chev', { html: icon('chevrons-up-down', 13, 2.2) }));
  btn.addEventListener('click', async () => {
    let libraries = web.libraries;
    try {
      libraries = (await web.social.others()).filter((p) => p.access.browse);
    } catch { /* on garde la liste de la page */ }
    openMenu(btn, [
      { label: t('lib.mine'), icon: 'headphones', checked: !viewing, action: () => viewing && web.switchLibrary(null) },
      ...libraries.map((p) => ({ label: p.name, icon: 'user', checked: viewing?.id === p.id, action: () => viewing?.id !== p.id && web.switchLibrary(p.id) })),
      { separator: true },
      { label: t('people.title'), icon: 'users', action: () => navigate('people') },
    ], { align: 'start', minWidth: 220 });
  });
  container.append(btn);
  if (!viewing) {
    onMyProfile(() => {
      const next = avatarEl(web.user, { size: 22 });
      face.replaceWith(next);
      face = next;
    });
  }
}

/** Bandeau en haut de la page tant qu'on parcourt la bibliothèque d'un autre compte. */
export function mountReadOnlyBanner(topbar) {
  const viewing = web?.viewing;
  if (!viewing) return;
  topbar.append(h('div.ro-banner', {},
    h('span.ro-text', { html: `${icon('lock', 14, 2)}<span>${t('lib.readOnly', { name: viewing.name })}</span>` }),
    h('button.btn.btn-secondary.btn-small', { type: 'button', text: t('lib.back'), onclick: () => web.switchLibrary(null) })));
}

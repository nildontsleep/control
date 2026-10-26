// Modifier son profil (nom affiché, présentation, couleur, photo) et choisir qui voit sa bibliothèque et ses récaps.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { ACCENTS } from '../color.js';
import { openSheet } from './sheet.js';
import { toast } from './feedback.js';
import { button, toggle, withScope, createScope } from './controls.js';
import { avatarEl, announceProfile } from './avatar.js';

const web = window.control.web; // absent dans l'application de bureau : ces fenêtres ne servent que sur le web
const social = web?.social;

export function openProfileEditor(profile, onSaved) {
  let avatar = profile.avatar || null;
  let color = profile.color || null;
  const preview = h('div.pf-avatar-preview');
  const paint = () => preview.replaceChildren(avatarEl({ ...profile, name: name.value || profile.username, avatar, color }, { size: 96 }));
  const name = h('input.text-input', { type: 'text', value: profile.displayName || '', maxlength: '40', placeholder: profile.username, spellcheck: 'false' });
  const bio = h('textarea.text-input', { rows: '3', maxlength: '280', placeholder: t('profile.bioPlaceholder') });
  bio.value = profile.bio || '';
  const status = h('p.pe-status');
  name.addEventListener('input', paint);

  const swatches = h('div.pf-swatches', {},
    h('button.pf-swatch.none', { type: 'button', 'data-tip': t('profile.colorAuto'), html: icon('x', 12, 2.4), onclick: () => { color = null; mark(); paint(); } }),
    ...Object.entries(ACCENTS).map(([key, c]) => h('button.pf-swatch', { type: 'button', 'data-tip': t(`accent.${key}`), style: { '--c': c.dark }, dataset: { color: c.dark }, onclick: () => { color = c.dark; mark(); paint(); } })));
  const mark = () => {
    for (const el of swatches.children) el.classList.toggle('on', el.dataset.color ? el.dataset.color === color : !color);
  };
  mark();

  const choose = async () => {
    status.textContent = '';
    const image = await web.pickImage({ square: true, max: 512 });
    if (!image) return;
    if (image.error) {
      status.textContent = image.error;
      return;
    }
    try {
      const saved = await social.setProfile({ avatar: { data: image.data } });
      avatar = saved.avatar;
      announceProfile(saved);
      paint();
    } catch (err) {
      status.textContent = err.message;
    }
  };
  const remove = async () => {
    const saved = await social.setProfile({ avatar: null });
    avatar = null;
    announceProfile(saved);
    paint();
  };

  paint();
  const save = h('button.btn.btn-primary', { type: 'button', text: t('common.save') });
  const cancel = h('button.btn.btn-secondary', { type: 'button', text: t('common.cancel') });
  const sheet = openSheet({
    title: t('profile.edit'),
    width: 460,
    body: [
      h('div.pe-top', {}, preview, h('div.pe-top-side', {},
        h('span.pe-label', { text: t('profile.photo') }),
        h('div.pe-cover-actions', {}, button(t('profile.photoPick'), { icon: 'image-plus', small: true, onClick: choose }), avatar ? button(t('profile.photoRemove'), { icon: 'trash-2', small: true, onClick: remove }) : null),
        status)),
      h('label.pe-field', {}, h('span.pe-label', { text: t('profile.displayName') }), name),
      h('label.pe-field', {}, h('span.pe-label', { text: t('profile.bio') }), bio),
      h('div.pe-field', {}, h('span.pe-label', { text: t('profile.color') }), swatches),
    ],
    footer: [cancel, save],
  });
  cancel.addEventListener('click', () => sheet.close());
  save.addEventListener('click', async () => {
    save.disabled = true;
    try {
      const saved = await social.setProfile({ displayName: name.value, bio: bio.value, color: color || '' });
      announceProfile(saved);
      sheet.close();
      toast(t('profile.saved'), 'check');
      onSaved?.(saved);
    } catch (err) {
      status.textContent = err.message;
      save.disabled = false;
    }
  });
}

/** Qui peut parcourir ma bibliothèque (browse) et comparer ses récaps avec moi (compare). */
export async function openSharingEditor(onSaved) {
  const [others, mine] = await Promise.all([social.others(), social.me()]);
  const grants = new Map(mine.grants.map((g) => [g.viewerId, { browse: g.browse, compare: g.compare }]));
  const get = (id) => grants.get(id) || { browse: false, compare: false };
  const scope = createScope();

  const line = (person, id) => {
    const g = { ...get(id) };
    const set = (key) => (v) => { g[key] = v; grants.set(id, { ...g }); };
    return h('div.sh-row', {},
      person ? avatarEl(person, { size: 32 }) : h('span.sh-everyone', { html: icon('users', 17, 1.9) }),
      h('div.sh-name', {}, h('strong', { text: person ? person.name : t('share.everyone') }), person ? h('span', { text: `@${person.username}` }) : h('span', { text: t('share.everyoneHint') })),
      withScope(scope, () => h('div.sh-switches', {},
        h('label.sh-switch', {}, h('span', { text: t('share.browse') }), toggle({ get: () => g.browse, set: set('browse') })),
        h('label.sh-switch', {}, h('span', { text: t('share.compare') }), toggle({ get: () => g.compare, set: set('compare') })))));
  };

  const body = h('div.sh-list', {},
    h('p.sheet-text', { text: t('share.intro') }),
    line(null, '*'),
    ...others.map((p) => line(p, p.id)));
  if (!others.length) body.append(h('p.sheet-text', { text: t('share.noOthers') }));

  const save = h('button.btn.btn-primary', { type: 'button', text: t('common.save') });
  const cancel = h('button.btn.btn-secondary', { type: 'button', text: t('common.cancel') });
  const sheet = openSheet({ title: t('share.title'), width: 520, body: [body], footer: [cancel, save] });
  cancel.addEventListener('click', () => sheet.close());
  save.addEventListener('click', async () => {
    save.disabled = true;
    try {
      const list = [...grants].map(([viewerId, g]) => ({ viewerId, browse: g.browse, compare: g.compare })).filter((g) => g.browse || g.compare);
      await social.setGrants(list);
      sheet.close();
      toast(t('share.saved'), 'check');
      onSaved?.();
    } catch (err) {
      toast(err.message, 'triangle-alert');
      save.disabled = false;
    }
  });
}

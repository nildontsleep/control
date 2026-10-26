// Compte (version web) : menu de la barre latérale, changement de mot de passe, déconnexion.

import { h } from '../dom.js';
import { t } from '../i18n.js';
import { openMenu } from './menu.js';
import { openSheet } from './sheet.js';
import { toast } from './feedback.js';
import { navigate } from '../router.js';

const web = window.control?.web;

export function openPasswordSheet() {
  const field = (label, name, autocomplete) => h('label.field', {}, h('span', { text: label }),
    h('input.text-input', { type: 'password', name, autocomplete, required: true }));
  const current = field(t('web.passwordCurrent'), 'current', 'current-password');
  const next = field(t('web.passwordNew'), 'next', 'new-password');
  const again = field(t('web.passwordAgain'), 'again', 'new-password');
  const error = h('p.sheet-text', { style: { color: '#ff6a5f', minHeight: '1.2em' } });
  const save = h('button.btn.btn-primary', { type: 'button', text: t('common.save') });
  const cancel = h('button.btn.btn-secondary', { type: 'button', text: t('common.cancel') });
  const sheet = openSheet({ title: t('web.password'), width: 400, body: [current, next, again, error], footer: [cancel, save] });
  cancel.addEventListener('click', () => sheet.close());
  save.addEventListener('click', async () => {
    const [a, b, c] = [current, next, again].map((f) => f.querySelector('input').value);
    error.textContent = '';
    if (b !== c) {
      error.textContent = t('web.passwordMismatch');
      return;
    }
    save.disabled = true;
    try {
      await web.rpc('srv:me:password', a, b);
      sheet.close();
      toast(t('web.passwordSaved'), 'key-round');
    } catch (err) {
      error.textContent = err.message;
      save.disabled = false;
    }
  });
  setTimeout(() => current.querySelector('input').focus(), 60);
}

export function openAccountMenu(anchor) {
  const items = [
    { header: `${web.user.name || web.user.username} · ${t(web.user.role === 'admin' ? 'web.roleAdmin' : 'web.roleUser')}` },
    { label: t('profile.mine'), icon: 'user', action: () => navigate('profile') },
    web.caps.manageServer ? { label: t('web.admin'), icon: 'settings', action: () => window.open('/admin', '_blank', 'noopener') } : null,
    { label: t('web.password'), icon: 'key-round', action: openPasswordSheet },
    { separator: true },
    { label: t('web.logout'), icon: 'power', action: () => web.logout() },
  ].filter(Boolean);
  openMenu(anchor, items, { align: 'start', placement: 'above', minWidth: 200 });
}



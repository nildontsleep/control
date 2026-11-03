// Application Subsonic (version web) : adresse du serveur, identifiant et mot de passe d'application à saisir dans un client
// compatible (Symfonium, DSub, Feishin, Amperfy…). Le mot de passe d'application est propre à cette API : il ne donne jamais
// accès au site, et le révéler demande de confirmer le mot de passe du compte.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { openSheet, promptSheet, confirmSheet } from './sheet.js';
import { toast } from './feedback.js';

const web = window.control?.web;

async function copy(text) {
  try {
    await navigator.clipboard.writeText(text);
    toast(t('subsonic.copied'), 'check');
  } catch {
    toast(t('subsonic.copyFailed'), 'triangle-alert');
  }
}

function field(label, value, { secret = false } = {}) {
  const input = h('input.text-input.sub-input', { type: secret ? 'password' : 'text', value, readonly: 'true', spellcheck: 'false', autocomplete: 'off' });
  const box = h('div.sub-field', {}, h('label', { text: label }), h('div.sub-line', {}, input,
    h('button.btn.btn-secondary.btn-small', { type: 'button', 'aria-label': t('subsonic.copy'), 'data-tip': t('subsonic.copy'), html: icon('copy', 15, 1.9), onclick: () => copy(input.value) })));
  return { box, input };
}

export function openSubsonicSheet() {
  const body = h('div.sub-body');
  const close = h('button.btn.btn-primary', { type: 'button', text: t('common.close') });
  const sheet = openSheet({ title: t('subsonic.title'), width: 520, body: [body], footer: [close] });
  close.addEventListener('click', () => sheet.close());

  /** Demande le mot de passe du compte, puis exécute l'appel ; null si annulé ou refusé. */
  async function confirmed(channel) {
    const password = await promptSheet({ title: t('subsonic.confirmTitle'), message: t('subsonic.confirmText'), placeholder: t('subsonic.password'), confirmLabel: t('common.ok'), secret: true });
    if (!password) return null;
    try {
      return await web.rpc(channel, password);
    } catch (err) {
      toast(err.message, 'triangle-alert');
      return null;
    }
  }

  async function paint() {
    const info = await web.rpc('srv:me:subsonic');
    if (!info.enabled) {
      body.replaceChildren(h('p.sheet-text', { text: t('subsonic.disabled') }));
      return;
    }
    const server = field(t('subsonic.server'), window.location.origin);
    const user = field(t('subsonic.username'), info.username);
    const pass = field(t('subsonic.secret'), info.hasSecret ? '••••••••••••••••' : '', { secret: true });
    pass.input.placeholder = t('subsonic.none');
    const rows = [h('p.sheet-text', { text: t('subsonic.intro') }), server.box, user.box, pass.box];
    const actions = h('div.sub-actions');

    const shown = (secret) => {
      pass.input.type = 'text';
      pass.input.value = secret;
      pass.input.select();
    };
    if (info.hasSecret) {
      actions.append(
        h('button.btn.btn-secondary', { type: 'button', text: t('subsonic.reveal'), onclick: async () => { const r = await confirmed('srv:me:subsonic-reveal'); if (r) shown(r.secret); } }),
        h('button.btn.btn-secondary', { type: 'button', text: t('subsonic.renew'), onclick: async () => {
          const { ok } = await confirmSheet({ title: t('subsonic.renew'), message: t('subsonic.renewAsk'), confirmLabel: t('subsonic.renew'), destructive: true });
          if (!ok) return;
          const r = await confirmed('srv:me:subsonic-renew');
          if (!r) return;
          await paint();
          // paint() reconstruit le contenu : on ré-affiche la nouvelle valeur dans le champ neuf.
          const fresh = body.querySelectorAll('.sub-input')[2];
          if (fresh) { fresh.type = 'text'; fresh.value = r.secret; fresh.select(); }
          toast(t('subsonic.renewed'), 'check');
        } }),
        h('button.btn.btn-secondary', { type: 'button', text: t('subsonic.revoke'), onclick: async () => {
          const { ok } = await confirmSheet({ title: t('subsonic.revoke'), message: t('subsonic.revokeAsk'), confirmLabel: t('subsonic.revoke'), destructive: true });
          if (!ok) return;
          await web.rpc('srv:me:subsonic-revoke');
          toast(t('subsonic.revoked'), 'check');
          paint();
        } }));
    } else {
      actions.append(h('button.btn.btn-primary', { type: 'button', text: t('subsonic.create'), onclick: async () => {
        const r = await confirmed('srv:me:subsonic-reveal');
        if (!r) return;
        await paint();
        const fresh = body.querySelectorAll('.sub-input')[2];
        if (fresh) { fresh.type = 'text'; fresh.value = r.secret; fresh.select(); }
      } }));
    }
    rows.push(actions, h('p.sheet-text.sub-hint', { text: t('subsonic.hint') }));
    body.replaceChildren(...rows);
  }
  paint().catch((err) => toast(err.message, 'triangle-alert'));
}

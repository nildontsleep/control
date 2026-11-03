// Réglages → Général → Scrobbling : connexion à ListenBrainz (jeton) et à Last.fm (autorisation sur leur site).

import { h } from '../dom.js';
import { t } from '../i18n.js';
import { openSheet, promptSheet } from './sheet.js';
import { toast } from './feedback.js';
import { button, row, group, badge } from './controls.js';
import { isWeb } from '../web.js';

const api = window.control.scrobble;
const B = { purple: '#bf5af2', red: '#ff453a' };

/** Un champ de saisie dans une feuille : renvoie le texte, ou null si annulé. */
const ask = (title, message, placeholder, { secret = false } = {}) => promptSheet({ title, message, placeholder, confirmLabel: t('scr.connect'), secret });

export function scrobbleGroup() {
  const lbLine = h('div.row-buttons');
  const fmLine = h('div.row-buttons');
  const lbText = h('span.row-hint');
  const fmText = h('span.row-hint');
  const pending = h('p.group-footer', { hidden: true });

  const lbRow = row('ListenBrainz', h('div.row-buttons', {}, lbText, lbLine), { hint: t('scr.lbHint'), badge: badge('audio-lines', B.purple) });
  const fmRow = row('Last.fm', h('div.row-buttons', {}, fmText, fmLine), { hint: isWeb ? t('scr.fmHintWeb') : t('scr.fmHint'), badge: badge('audio-lines', B.red) });

  function paint(st) {
    if (!st) return;
    const lb = st.listenbrainz;
    const fm = st.lastfm;
    lbText.textContent = lb.error ? `${t('scr.reconnect')} (${lb.error})` : lb.connected ? t('scr.connectedAs', { user: lb.user || '?' }) : t('scr.notConnected');
    lbText.style.color = lb.error ? B.red : '';
    lbLine.replaceChildren(...(lb.connected
      ? [button(t('scr.disconnect'), { small: true, onClick: async () => paint(await api.disconnect('listenbrainz')) })]
      : [button(t('scr.connect'), { small: true, onClick: connectListenBrainz })]),
    ...(lb.connected && lb.error ? [button(t('scr.connect'), { small: true, onClick: connectListenBrainz })] : []));

    fmText.textContent = fm.error ? `${t('scr.reconnect')} (${fm.error})` : fm.connected ? t('scr.connectedAs', { user: fm.user || '?' }) : fm.configured ? t('scr.notConnected') : t(isWeb ? 'scr.fmNoKeysWeb' : 'scr.fmNoKeys');
    fmText.style.color = fm.error ? B.red : '';
    fmLine.replaceChildren(...[
      fm.connected ? button(t('scr.disconnect'), { small: true, onClick: async () => paint(await api.disconnect('lastfm')) }) : null,
      !fm.connected || fm.error ? button(t('scr.connect'), { small: true, onClick: () => connectLastfm(fm) }) : null,
    ].filter(Boolean));
    pending.hidden = !st.pending;
    pending.textContent = t('scr.pending', { n: st.pending });
  }

  async function connectListenBrainz() {
    const token = await ask('ListenBrainz', t('scr.lbAsk'), t('scr.lbToken'), { secret: true });
    if (!token) return;
    // Serveur compatible (Maloja, ListenBrainz auto-hébergé…) : saisie facultative, en deux temps pour rester simple.
    const res = await api.connectListenBrainz(token, null);
    if (res.ok) {
      toast(t('scr.done'), 'check');
      paint(res.data);
    } else toast(res.message, 'triangle-alert');
  }

  async function connectLastfm(fm) {
    // Bureau : l'application Last.fm est celle de l'utilisateur (https://www.last.fm/api/account/create).
    if (!fm.configured && !isWeb) {
      const key = await ask('Last.fm', t('scr.fmKeyAsk'), t('scr.fmKey'));
      if (!key) return;
      const secret = await ask('Last.fm', t('scr.fmSecretAsk'), t('scr.fmSecret'), { secret: true });
      if (!secret) return;
      await api.setLastfmKeys(key, secret);
    } else if (!fm.configured) {
      toast(t('scr.fmNoKeysWeb'), 'triangle-alert');
      return;
    }
    const start = await api.lastfmStart();
    if (!start.ok) { toast(start.message, 'triangle-alert'); return; }
    window.control.app.openExternal(start.data.url);
    const done = h('button.btn.btn-primary', { type: 'button', text: t('scr.fmDone') });
    const cancel = h('button.btn.btn-secondary', { type: 'button', text: t('common.cancel') });
    const sheet = openSheet({ title: 'Last.fm', width: 420, body: [h('p.sheet-text', { text: t('scr.fmAuthorize') })], footer: [cancel, done] });
    cancel.addEventListener('click', () => sheet.close());
    done.addEventListener('click', async () => {
      done.disabled = true;
      const res = await api.lastfmFinish(start.data.token);
      if (res.ok) {
        sheet.close();
        toast(t('scr.done'), 'check');
        paint(res.data);
      } else {
        toast(res.message, 'triangle-alert');
        done.disabled = false;
      }
    });
  }

  api.status().then(paint).catch(() => {});
  api.onChanged(paint);
  return group(t('scr.title'), [lbRow, fmRow, pending], { footer: t('scr.footer') });
}

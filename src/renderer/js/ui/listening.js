// « En écoute » (version web) : la ligne qui montre ce qu'un compte écoute, avec des barres qui bougent.

import { h } from '../dom.js';
import { t } from '../i18n.js';

/** now : { title, artist } ou null. Renvoie l'élément à placer (masqué quand rien n'est écouté) et une fonction de mise à jour. */
export function listeningLine(now) {
  const text = h('span.lv-text');
  const el = h('div.lv', {}, h('span.lv-bars', {}, h('i'), h('i'), h('i')), text);
  const set = (next) => {
    el.hidden = !next;
    text.textContent = next ? `${next.title}${next.artist ? ` — ${next.artist}` : ''}` : '';
    el.title = next ? t('listen.now') : '';
  };
  set(now);
  return { el, set };
}

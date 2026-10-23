// Feux tricolores façon macOS pour la fenêtre sans bordure.

import { h } from '../dom.js';
import { t } from '../i18n.js';

const GLYPH = {
  close: '<svg viewBox="0 0 12 12" width="12" height="12"><path d="M3.75 3.75l4.5 4.5M8.25 3.75l-4.5 4.5" stroke="currentColor" stroke-width="1.25" stroke-linecap="round"/></svg>',
  min: '<svg viewBox="0 0 12 12" width="12" height="12"><path d="M2.9 6h6.2" stroke="currentColor" stroke-width="1.35" stroke-linecap="round"/></svg>',
  zoom: '<svg viewBox="0 0 12 12" width="12" height="12"><path d="M3.4 4.25v4.35h4.35zM8.6 7.75V3.4H4.25z" fill="currentColor"/></svg>',
};

export function trafficLights() {
  const win = window.control.win;
  const light = (kind, label, onClick) => h(`button.tlight.tlight-${kind}`, { type: 'button', 'aria-label': label, html: GLYPH[kind], onclick: onClick });
  return h('div.traffic', {},
    light('close', t('win.close'), () => win.close()),
    light('min', t('win.minimize'), () => win.minimize()),
    light('zoom', t('win.zoom'), () => win.toggleMaximize()));
}

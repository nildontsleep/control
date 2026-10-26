// Sources : « cet ordinateur » et des serveurs control (la version web hébergée ailleurs).
// Ici : le sélecteur de la barre latérale, la barre de fenêtre affichée au-dessus d'un serveur, et la liste des réglages.
// Le serveur lui-même est une vue web posée par le processus principal sous cette barre (voir src/main/sources.js).

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { openMenu } from './menu.js';
import { trafficLights } from './traffic.js';
import { promptSheet, confirmSheet } from './sheet.js';
import { toast } from './feedback.js';
import { group, row, button, badge } from './controls.js';

const api = window.control.sources || null;
export const hasSources = !!api;

let state = { servers: [], active: 'local', status: { state: 'idle' } };
const listeners = new Set();

export const sourcesState = () => state;

/** Appelé à chaque changement (serveurs, source affichée, état de la connexion) ; renvoie la fonction qui se désabonne. */
export function onSources(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

function apply(next) {
  state = next;
  document.body.classList.toggle('source-remote', state.servers.some((s) => s.id === state.active));
  for (const fn of listeners) fn(state);
}

const spinner = () => h('span.spinner', {}, ...Array.from({ length: 12 }, (_, i) => h('i', { style: { '--i': i } })));

/** Menu des sources, ouvert depuis le sélecteur de la barre latérale ou la barre de fenêtre d'un serveur. */
export function openSourceMenu(anchor, { openSettings, peek = false } = {}) {
  const items = [
    { label: t('src.local'), icon: 'hard-drive', checked: state.active === 'local', action: () => api.switch('local') },
    ...state.servers.map((s) => ({ label: s.name, icon: 'globe', checked: state.active === s.id, action: () => api.switch(s.id) })),
    { separator: true },
    { label: t('src.manage'), icon: 'settings', action: async () => { await api.switch('local'); openSettings?.('servers'); } },
  ];
  openMenu(anchor, items, { align: 'start', onClose: peek ? () => api.unpeek() : undefined });
}

/** Sélecteur sous le logo de la barre latérale : visible dès qu'un serveur est enregistré. */
export function mountSourceSwitcher(container, { openSettings }) {
  if (!api) return;
  const name = h('span.src-name');
  const btn = h('button.src-btn', { type: 'button', onclick: (e) => openSourceMenu(e.currentTarget, { openSettings }) },
    h('span.src-ico', { html: icon('hard-drive', 15, 2) }),
    name,
    h('span.src-chev', { html: icon('chevrons-up-down', 13, 2.2) }));
  container.append(btn);
  const paint = (s) => {
    container.hidden = !s.servers.length;
    name.textContent = t('src.local');
  };
  paint(state);
  onSources(paint);
}

/** Barre de fenêtre et message d'état affichés tant qu'un serveur est la source active (la vue du serveur est dessous). */
export function mountRemoteHost({ openSettings }) {
  if (!api) return;
  const win = window.control.win;
  const name = h('span.rh-name');
  const switcher = h('button.rh-switch', { type: 'button' },
    h('span.rh-ico', { html: icon('globe', 15, 2) }),
    name,
    h('span.rh-chev', { html: icon('chevrons-up-down', 13, 2.2) }));
  switcher.addEventListener('click', async () => {
    // La vue du serveur est une surface native, au-dessus de la page : on la remplace par sa capture le temps du menu.
    const image = await api.peek();
    if (image) host.style.setProperty('--peek', `url("${image}")`);
    host.classList.toggle('peeking', !!image);
    openSourceMenu(switcher, { openSettings, peek: true });
  });
  const reload = h('button.rh-btn', { type: 'button', 'data-tip': t('src.reload'), html: icon('refresh-cw', 15, 2), onclick: () => api.reload() });
  const card = h('div.rh-card');
  const bar = h('div.rh-bar', {}, trafficLights(), switcher, h('span.spacer'), reload);
  bar.addEventListener('dblclick', (e) => { if (e.target === bar) win.toggleMaximize(); });
  const host = h('div.remote-host', { hidden: true }, bar, h('div.rh-body', {}, card));
  document.body.append(host);

  const paint = (s) => {
    const server = s.servers.find((x) => x.id === s.active);
    host.hidden = !server;
    if (!server) {
      host.classList.remove('peeking');
      return;
    }
    name.textContent = server.name;
    if (s.status.state !== 'ready') host.classList.remove('peeking');
    if (s.status.state === 'ready') card.replaceChildren();
    else if (s.status.state === 'error') {
      card.replaceChildren(
        h('div.rh-icon', { html: icon('unplug', 30, 1.6) }),
        h('h2', { text: t('src.errorTitle', { name: server.name }) }),
        h('p', { text: s.status.error || t('src.errorText') }),
        h('div.rh-actions', {},
          button(t('src.retry'), { kind: 'primary', icon: 'refresh-cw', onClick: () => api.reload() }),
          button(t('src.backLocal'), { icon: 'hard-drive', onClick: () => api.switch('local') })));
    } else card.replaceChildren(spinner(), h('p', { text: t('src.connecting', { name: server.name }) }));
  };
  paint(state);
  onSources(paint);
}

/** Contenu de l'onglet « Serveurs » des réglages. */
export function serversTab({ close }) {
  const list = h('div.group-body');
  const input = h('input.text-input.key-input', { type: 'text', placeholder: t('src.urlPlaceholder'), spellcheck: 'false', autocomplete: 'off', autocapitalize: 'none' });
  const status = h('span.key-status');
  const addBtn = button(t('src.add'), { kind: 'primary', small: true });

  const add = async () => {
    const url = input.value.trim();
    if (!url) return;
    addBtn.disabled = true;
    status.className = 'key-status';
    status.textContent = t('src.checking');
    const res = await api.add({ url });
    addBtn.disabled = false;
    if (!res.ok) {
      status.className = 'key-status bad';
      status.textContent = res.error;
      return;
    }
    input.value = '';
    status.className = 'key-status ok';
    status.textContent = t('src.added', { name: res.server.name });
    paint();
  };
  addBtn.addEventListener('click', add);
  input.addEventListener('keydown', (e) => { if (e.key === 'Enter') add(); });

  function paint() {
    const servers = state.servers;
    list.replaceChildren(...(servers.length
      ? servers.map((s) => row(s.name, h('div.row-buttons', {},
        button(t('src.open'), { small: true, icon: 'plug', onClick: async () => { close(); await api.switch(s.id); } }),
        button(null, { small: true, icon: 'square-pen', tip: t('pl.rename'), onClick: async () => {
          const value = await promptSheet({ title: t('src.renameTitle'), value: s.name, confirmLabel: t('common.ok') });
          if (value) await api.rename(s.id, value);
        } }),
        button(null, { small: true, icon: 'trash-2', kind: 'danger', tip: t('common.delete'), onClick: async () => {
          const { ok } = await confirmSheet({ title: t('src.removeTitle', { name: s.name }), message: t('src.removeText'), confirmLabel: t('common.delete'), destructive: true });
          if (!ok) return;
          await api.remove(s.id);
          toast(t('src.removed', { name: s.name }), 'trash-2');
        } })), { hint: s.url, badge: badge('globe', '#40c8e0') }))
      : [h('div.row', {}, h('div.row-label', {}, h('span.row-hint', { text: t('src.none') })))]));
  }

  // La liste suit les changements (ajout, renommage, suppression) tant que l'onglet est affiché.
  const off = onSources(() => { if (list.isConnected) paint(); else off(); });
  paint();

  return [
    h('section.group', {}, h('header.group-head', {}, h('h3', { text: t('src.title') })), list, h('p.group-footer', { text: t('src.intro') })),
    group(t('src.addTitle'), [
      h('div.row.row-block', {}, h('div.key-form', {}, input, addBtn), h('div.key-meta', {}, status)),
    ], { footer: t('src.addHint') }),
  ];
}

export async function initSources({ openSettings, pause }) {
  if (!api) return;
  apply(await api.state());
  api.onChange(apply);
  api.onPauseRequest(pause);
  mountRemoteHost({ openSettings });
}

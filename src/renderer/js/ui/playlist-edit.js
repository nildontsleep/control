// Personnaliser une playlist : nom, description, pochette (image, pochette d'un morceau ou mosaïque) et visibilité.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { playlistRows } from '../data.js';
import { openSheet } from './sheet.js';
import { toast } from './feedback.js';
import { playlistCover } from './cards.js';
import { coverEl } from './cover.js';
import { toggle, withScope, createScope, button } from './controls.js';
import { isWeb } from '../web.js';

const api = window.control.playlists;

/** Grille des pochettes des morceaux de la playlist : on en choisit une. */
function pickTrackCover(tracks) {
  return new Promise((resolve) => {
    const seen = new Map();
    for (const track of tracks) if (track.cover && !seen.has(track.cover)) seen.set(track.cover, track);
    let done = false;
    const finish = (value) => {
      if (done) return;
      done = true;
      sheet.close();
      resolve(value);
    };
    const body = seen.size
      ? h('div.pe-grid', {}, ...[...seen.values()].slice(0, 120).map((track) => {
        const btn = h('button.pe-pick', { type: 'button', title: `${track.title} — ${track.artist || ''}`, onclick: () => finish(track) }, coverEl(track, { small: true }));
        return btn;
      }))
      : h('p.sheet-text', { text: t('pl.coverNone') });
    const sheet = openSheet({ title: t('pl.coverTrackTitle'), width: 520, body: [body], onClose: () => finish(null) });
  });
}

export function openPlaylistEditor(playlist) {
  const rows = playlistRows(playlist);
  const tracks = rows.map((r) => r.track);
  let cover = playlist.cover || null;
  let shared = !!playlist.shared;

  const preview = h('div.pe-preview');
  const paint = () => preview.replaceChildren(playlistCover({ ...playlist, cover }, tracks, { size: 'l' }));
  paint();

  const status = h('p.pe-status');
  const apply = async (promise) => {
    status.textContent = '';
    const res = await promise;
    if (res === null || res === undefined) return;
    if (!res.ok) {
      status.textContent = res.message || t('pl.coverError');
      return;
    }
    cover = res.cover || null;
    paint();
  };

  const name = h('input.text-input', { type: 'text', value: playlist.name, maxlength: '120', spellcheck: 'false' });
  const description = h('textarea.text-input', { rows: '3', maxlength: '500', placeholder: t('pl.descPlaceholder') });
  description.value = playlist.description || '';

  const coverButtons = h('div.pe-cover-actions', {},
    button(t('pl.coverPick'), { icon: 'image-plus', small: true, onClick: () => apply(api.pickCover(playlist.id)) }),
    button(t('pl.coverTrack'), { icon: 'images', small: true, onClick: async () => {
      const track = await pickTrackCover(tracks);
      if (track) await apply(api.setCover(playlist.id, { trackId: track.id }));
    } }),
    button(t('pl.coverAuto'), { icon: 'layout-grid', small: true, onClick: () => apply(api.setCover(playlist.id, null)) }));

  const visibility = isWeb
    ? h('div.pe-row', {},
      h('div.pe-row-label', {}, h('strong', { text: t('pl.shared') }), h('span', { text: t('pl.sharedHint') })),
      withScope(createScope(), () => toggle({ get: () => shared, set: (v) => { shared = v; } })))
    : null;

  const save = h('button.btn.btn-primary', { type: 'button', text: t('common.save') });
  const cancel = h('button.btn.btn-secondary', { type: 'button', text: t('common.cancel') });
  const sheet = openSheet({
    title: t('pl.editTitle'),
    width: 480,
    body: [
      h('div.pe-top', {}, preview, h('div.pe-top-side', {}, h('span.pe-label', { text: t('pl.cover') }), coverButtons, status)),
      h('label.pe-field', {}, h('span.pe-label', { text: t('pl.name') }), name),
      h('label.pe-field', {}, h('span.pe-label', { text: t('pl.description') }), description),
      visibility,
    ],
    footer: [cancel, save],
  });
  cancel.addEventListener('click', () => sheet.close());
  save.addEventListener('click', async () => {
    save.disabled = true;
    const patch = { name: name.value.trim() || playlist.name, description: description.value.trim() };
    if (isWeb) patch.shared = shared;
    await api.update(playlist.id, patch);
    sheet.close();
    toast(t('pl.saved'), 'check');
  });
  setTimeout(() => name.focus(), 60);
}

// Vue d’une playlist (classique ou intelligente) : lecture, tri manuel par glisser-déposer, renommage, export.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { fmtLong } from '../format.js';
import { playlistById, playlistRows } from '../data.js';
import { player } from '../player.js';
import { navigate } from '../router.js';
import { createTrackList } from '../ui/tracklist.js';
import { mosaic } from '../ui/cards.js';
import { openMenu } from '../ui/menu.js';
import { promptSheet, confirmSheet } from '../ui/sheet.js';
import { toast } from '../ui/feedback.js';
import { openSmartEditor } from '../ui/smart.js';
import { playButtons, emptyState } from './shared.js';

const api = window.control.playlists;

export async function renamePlaylist(playlist) {
  const name = await promptSheet({ title: t('pl.rename'), value: playlist.name, confirmLabel: t('common.rename') });
  if (name && name !== playlist.name) await api.rename(playlist.id, name);
}

export async function deletePlaylist(playlist) {
  const { ok } = await confirmSheet({
    title: t('pl.deleteTitle', { name: playlist.name }),
    message: t('pl.deleteText'),
    confirmLabel: t('common.delete'),
    destructive: true,
  });
  if (!ok) return false;
  await api.delete(playlist.id);
  toast(t('toast.playlistDeleted'), 'trash-2');
  return true;
}

export async function exportPlaylist(playlist) {
  const rows = playlistRows(playlist);
  const result = await api.export(playlist.name, rows.map((r) => r.track.id));
  if (result) toast(t('toast.exported', { n: result.count }), 'check');
}

export function playlistView(root, id, { scroller }) {
  const playlist = playlistById(id);
  if (!playlist) {
    navigate('songs', null, { replace: true });
    return {};
  }
  const smart = playlist.kind === 'smart';
  const rows = playlistRows(playlist);
  const tracks = rows.map((r) => r.track);
  let order = rows.map((r) => r);

  const list = createTrackList({
    items: order,
    layout: 'playlist',
    scroller,
    context: smart ? {} : {
      playlistId: playlist.id,
      removeFromPlaylist: async (selected) => {
        await api.remove(selected.map((r) => r.entryId));
      },
    },
    onMove: smart ? null : async (from, to) => {
      const moving = from.map((i) => order[i]);
      const rest = order.filter((_, i) => !from.includes(i));
      const insertAt = to - from.filter((i) => i < to).length;
      rest.splice(insertAt, 0, ...moving);
      order = rest;
      list.setItems(order);
      await api.reorder(playlist.id, order.map((r) => r.entryId));
    },
  });

  const more = h('button.btn.btn-secondary.btn-icon-only', { type: 'button', 'data-tip': t('common.more'), html: icon('ellipsis', 18, 2) });
  more.addEventListener('click', () => {
    openMenu(more, [
      { label: t('pl.rename'), icon: 'square-pen', action: () => renamePlaylist(playlist) },
      smart ? { label: t('pl.editRules'), icon: 'sliders-horizontal', action: () => openSmartEditor(playlist) } : null,
      { label: t('pl.export'), icon: 'upload', action: () => exportPlaylist(playlist) },
      { separator: true },
      { label: t('common.delete'), icon: 'trash-2', action: () => deletePlaylist(playlist).then((ok) => ok && navigate('songs')) },
    ].filter(Boolean), { align: 'end' });
  });

  const duration = tracks.reduce((s, x) => s + x.duration, 0);
  root.append(
    h('header.hero', {},
      h('div.hero-art', {}, h('div.hero-cover.mosaic-wrap', {}, mosaic(tracks))),
      h('div.hero-text', {},
        h('div.hero-kind', {}, smart ? h('span.badge-smart', { html: `${icon('sparkles', 12, 2)}<span>${t('pl.smart')}</span>` }) : t('pl.kind')),
        h('h1.editable', { text: playlist.name, title: t('pl.rename'), onclick: () => renamePlaylist(playlist) }),
        h('p.hero-meta', { text: tracks.length ? `${t('count.tracks', { n: tracks.length })} · ${fmtLong(duration)}` : t('pl.empty') }),
        h('div.hero-actions', {}, ...playButtons(() => tracks), more))),
    tracks.length
      ? list.el
      : emptyState({ icon: 'list-music', title: t('pl.emptyTitle'), text: smart ? t('pl.emptySmart') : t('pl.emptyText') }),
    h('div.page-foot'),
  );
  void player;
  return { destroy: () => list.destroy() };
}

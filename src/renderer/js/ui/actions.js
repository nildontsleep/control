// Actions partagées : menus contextuels, ajout aux playlists, suppression, glisser-déposer de morceaux.

import { h } from '../dom.js';
import { t } from '../i18n.js';
import { player } from '../player.js';
import { lib, loadPlaylists } from '../data.js';
import { navigate } from '../router.js';
import { toast } from './feedback.js';
import { openMenu } from './menu.js';
import { promptSheet, confirmSheet } from './sheet.js';
import { openTagEditor } from './tag-editor.js';
import { isWeb, can } from '../web.js';

const library = window.control.library;
const playlists = window.control.playlists;

export const DRAG_TRACKS = 'application/x-control-tracks';

function virtualAnchor(x, y) {
  return {
    getBoundingClientRect: () => ({ left: x, top: y, right: x, bottom: y, width: 0, height: 0 }),
    classList: { add() {}, remove() {} },
    contains: () => false,
  };
}

export function openContextMenu(event, items) {
  event.preventDefault();
  openMenu(virtualAnchor(event.clientX, event.clientY), items.filter(Boolean), { align: 'start' });
}

export async function createPlaylistWith(tracks = []) {
  const name = await promptSheet({
    title: t('pl.new'),
    placeholder: t('pl.namePlaceholder'),
    confirmLabel: t('common.create'),
  });
  if (!name) return null;
  const id = await playlists.create(name, 'manual', null, tracks.map((x) => x.id));
  await loadPlaylists();
  toast(tracks.length ? t('toast.playlistCreatedWith', { name, n: tracks.length }) : t('toast.playlistCreated', { name }), 'list-music');
  return id;
}

export async function addToPlaylist(playlist, tracks) {
  await playlists.add(playlist.id, tracks.map((x) => x.id));
  toast(t('toast.added', { n: tracks.length, name: playlist.name }), 'list-plus');
}

export async function removeFromLibrary(tracks) {
  const { ok, checked } = await confirmSheet({
    title: tracks.length > 1 ? t('del.titleMany', { n: tracks.length }) : t('del.title'),
    message: t('del.message'),
    confirmLabel: t('del.confirm'),
    destructive: true,
    checkbox: can('editFiles') ? t('del.alsoFiles') : null,
  });
  if (!ok) return false;
  await library.removeTracks(tracks.map((x) => x.id), checked);
  toast(checked ? t('toast.trashed', { n: tracks.length }) : t('toast.removed', { n: tracks.length }), 'trash-2');
  return true;
}

export function artistKey(track) {
  const name = track.albumArtist || track.artist;
  if (!name) return null;
  for (const a of lib.artists) if (a.name === name) return a.key;
  return null;
}

/** Éléments de menu pour un ou plusieurs morceaux. ctx : { playlistId, removeFromPlaylist, play } */
export function trackMenuItems(tracks, ctx = {}) {
  const n = tracks.length;
  const first = tracks[0];
  const manual = lib.playlists.filter((p) => p.kind === 'manual');
  const items = [
    { label: n > 1 ? t('menu.playSelection', { n }) : t('menu.play'), icon: 'play', action: ctx.play || (() => player.playQueue(tracks, 0)) },
    { label: t('menu.playNext'), icon: 'list-start', action: () => { player.playNext(tracks); toast(t('toast.playNext'), 'list-start'); } },
    { label: t('menu.addQueue'), icon: 'list-end', action: () => { player.enqueue(tracks); toast(t('toast.queued'), 'list-end'); } },
    { separator: true },
    { header: t('menu.addToPlaylist') },
    ...manual.slice(0, 12).map((p) => ({ label: p.name, icon: 'list-music', action: () => addToPlaylist(p, tracks) })),
    { label: t('menu.newPlaylist'), icon: 'plus', action: () => createPlaylistWith(tracks).then((id) => id && navigate('playlist', id)) },
    { separator: true },
  ];
  if (ctx.removeFromPlaylist) items.push({ label: ctx.removeLabel || t('menu.removeFromPlaylist'), icon: 'x', action: ctx.removeFromPlaylist });
  if (n === 1) {
    const key = first.kind === 'audio' ? artistKey(first) : null;
    if (key) items.push({ label: t('menu.goArtist'), icon: 'user', action: () => navigate('artist', key) });
    if (first.albumKey) items.push({ label: t('menu.goAlbum'), icon: 'disc-3', action: () => navigate('album', first.albumKey) });
    if (first.sourceUrl) items.push({ label: t('menu.openSource'), icon: 'link', action: () => window.control.app.openExternal(first.sourceUrl) });
  }
  if (!isWeb) items.push({ label: t('menu.reveal'), icon: 'folder-open', action: () => library.reveal(first.id) });
  if (can('editFiles')) items.push({ label: t('tag.menu'), icon: 'tags', shortcut: 'Ctrl+I', action: () => openTagEditor(tracks) });
  items.push({ separator: true });
  items.push({ label: t('menu.removeLibrary'), icon: 'trash-2', action: () => removeFromLibrary(tracks) });
  return items;
}

/** Prépare un glisser-déposer de morceaux (vers une playlist de la barre latérale). */
export function startTrackDrag(event, tracks) {
  event.dataTransfer.setData(DRAG_TRACKS, JSON.stringify(tracks.map((x) => x.id)));
  event.dataTransfer.effectAllowed = 'copyMove';
  const ghost = h('div.drag-ghost', {}, h('strong', { text: tracks[0].title }), tracks.length > 1 ? h('span', { text: `+${tracks.length - 1}` }) : null);
  document.body.append(ghost);
  event.dataTransfer.setDragImage(ghost, 16, 16);
  setTimeout(() => ghost.remove());
}

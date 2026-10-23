// Liste de morceaux virtualisée : sélection multiple, lecture, menu contextuel, glisser-déposer, tri.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { fmtTime } from '../format.js';
import { player, on as onPlayer } from '../player.js';
import { settings } from '../settings.js';
import { coverEl } from './cover.js';
import { openContextMenu, trackMenuItems, startTrackDrag, removeFromLibrary } from './actions.js';

const OVERSCAN = 6;
const REORDER = 'application/x-control-reorder';

const LAYOUTS = {
  songs: { cols: '44px minmax(0,2.2fr) minmax(0,1.3fr) minmax(0,1.3fr) 56px 64px', cells: ['title', 'artist', 'album', 'plays', 'time'], cover: true, sortable: true },
  album: { cols: '44px minmax(0,3fr) 56px 64px', cells: ['title', 'plays', 'time'], cover: false, sortable: false },
  playlist: { cols: '44px minmax(0,2.2fr) minmax(0,1.3fr) minmax(0,1.3fr) 64px', cells: ['title', 'artist', 'album', 'time'], cover: true, sortable: false },
  artist: { cols: '44px minmax(0,2.4fr) minmax(0,1.5fr) 56px 64px', cells: ['title', 'album', 'plays', 'time'], cover: true, sortable: false },
  search: { cols: '44px minmax(0,2.2fr) minmax(0,1.3fr) minmax(0,1.3fr) 64px', cells: ['title', 'artist', 'album', 'time'], cover: true, sortable: false },
  queue: { cols: '28px minmax(0,1fr) 52px', cells: ['title', 'time'], cover: true, sortable: false, compact: true },
};

const SORT_KEYS = { title: 'title', artist: 'artist', album: 'album', plays: 'playCount', time: 'duration' };

/**
 * options : { items: [{ track, entryId? }], layout, scroller, context: { playlistId, removeFromPlaylist },
 *             sort: { key, dir }, onSort(key), onMove(fromIndices, toIndex), onPlay(index) }
 */
export function createTrackList(options) {
  const { layout: layoutName = 'songs', scroller, sort = null, onSort = null, onMove = null } = options;
  const layout = LAYOUTS[layoutName];
  let items = options.items;
  const rowHeight = layoutName === 'queue' ? 56 : settings.compact ? 44 : 54;
  const selected = new Set();
  let anchor = -1;
  const mounted = new Map();
  let raf = 0;
  let dropAt = -1;

  const el = h(`div.tl.tl-${layoutName}`, { tabindex: '0', style: { '--cols': layout.cols } });
  const body = h('div.tl-body');
  const indicator = h('div.tl-drop');
  body.append(indicator);

  if (layout.sortable && sort) {
    const labels = { title: t('col.title'), artist: t('col.artist'), album: t('col.album'), plays: t('col.plays'), time: t('col.time') };
    const head = h('div.tl-head', {}, h('div.tl-cell.idx', { text: '#' }));
    for (const cell of layout.cells) {
      const key = SORT_KEYS[cell];
      const active = sort.key === key;
      const col = h(`div.tl-cell.${cell}.sortable${active ? '.active' : ''}`, { onclick: () => onSort?.(key) },
        h('span', { text: labels[cell] }),
        active ? h('span.sort-arrow', { html: icon(sort.dir === 'asc' ? 'chevron-up' : 'chevron-down', 12, 2.4) }) : null);
      if (cell === 'plays' || cell === 'time') col.classList.add('right');
      head.append(col);
    }
    el.append(head);
  }
  el.append(body);

  const tracksAt = (indices) => indices.map((i) => items[i]?.track).filter(Boolean);
  const selectedIndices = () => [...selected].sort((a, b) => a - b);

  function play(index) {
    if (options.onPlay) return options.onPlay(index);
    player.playQueue(items.map((x) => x.track), index);
  }

  function cellsFor(item, i) {
    const { track } = item;
    const out = [];
    const num = layoutName === 'album' ? (track.trackNo || i + 1) : i + 1;
    out.push(h('div.tl-cell.idx', {},
      h('span.idx-num', { text: layoutName === 'queue' ? '' : String(num) }),
      h('span.idx-eq', { html: '<i></i><i></i><i></i><i></i>' }),
      h('button.idx-play', { type: 'button', tabindex: '-1', html: icon('play', 15, 0), onclick: (e) => { e.stopPropagation(); play(i); } })));
    for (const cell of layout.cells) {
      if (cell === 'title') {
        out.push(h('div.tl-cell.title', {},
          layout.cover ? coverEl(track, { small: true, className: 'row-cover' }) : null,
          h('div.title-text', {}, h('span.t', { text: track.title }), layoutName === 'queue' ? h('span.s', { text: track.artist || '' }) : null)));
      } else if (cell === 'artist') {
        out.push(h('div.tl-cell.artist', {}, h('span', { text: track.artist || '—' })));
      } else if (cell === 'album') {
        out.push(h('div.tl-cell.album', {}, h('span', { text: track.album || '—' })));
      } else if (cell === 'plays') {
        out.push(h('div.tl-cell.plays.right', {}, h('span', { text: track.playCount ? String(track.playCount) : '' })));
      } else if (cell === 'time') {
        out.push(h('div.tl-cell.time.right', {}, h('span', { text: fmtTime(track.duration) })));
      }
    }
    return out;
  }

  function buildRow(i) {
    const row = h('div.tl-row', { role: 'row', draggable: 'true', dataset: { i } }, ...cellsFor(items[i], i));
    row.style.transform = `translateY(${i * rowHeight}px)`;
    row.style.height = `${rowHeight}px`;
    if (selected.has(i)) row.classList.add('selected');
    paintPlaying(row, i);
    return row;
  }

  function paintPlaying(row, i) {
    const current = player.track && items[i]?.track.id === player.track.id;
    row.classList.toggle('current', !!current);
    row.classList.toggle('playing', !!current && player.playing);
  }

  function refreshPlaying() {
    for (const [i, row] of mounted) paintPlaying(row, i);
  }

  function paintSelection() {
    for (const [i, row] of mounted) row.classList.toggle('selected', selected.has(i));
  }

  function update() {
    raf = 0;
    if (!el.isConnected) return;
    const sc = scroller.getBoundingClientRect();
    const bd = body.getBoundingClientRect();
    const offset = sc.top - bd.top;
    const first = Math.max(0, Math.floor(offset / rowHeight) - OVERSCAN);
    const last = Math.min(items.length, Math.ceil((offset + sc.height) / rowHeight) + OVERSCAN);
    for (const [i, row] of mounted) {
      if (i < first || i >= last) {
        row.remove();
        mounted.delete(i);
      }
    }
    for (let i = first; i < last; i++) {
      if (mounted.has(i)) continue;
      const row = buildRow(i);
      body.append(row);
      mounted.set(i, row);
    }
  }

  const schedule = () => {
    if (!raf) raf = requestAnimationFrame(update);
  };

  function relayout() {
    body.style.height = `${items.length * rowHeight}px`;
    for (const row of mounted.values()) row.remove();
    mounted.clear();
    update();
  }

  // ─── Sélection ─────────────────────────────────────────────────────────────
  function select(i, { additive = false, range = false } = {}) {
    if (range && anchor >= 0) {
      if (!additive) selected.clear();
      const [a, b] = [Math.min(anchor, i), Math.max(anchor, i)];
      for (let k = a; k <= b; k++) selected.add(k);
    } else if (additive) {
      if (selected.has(i)) selected.delete(i);
      else selected.add(i);
      anchor = i;
    } else {
      selected.clear();
      selected.add(i);
      anchor = i;
    }
    paintSelection();
  }

  function scrollTo(i) {
    const sc = scroller.getBoundingClientRect();
    const top = body.getBoundingClientRect().top - sc.top + i * rowHeight;
    if (top < 0) scroller.scrollTop += top - 8;
    else if (top + rowHeight > sc.height - 110) scroller.scrollTop += top + rowHeight - sc.height + 110;
  }

  body.addEventListener('click', (e) => {
    const row = e.target.closest('.tl-row');
    if (!row) return;
    select(Number(row.dataset.i), { additive: e.ctrlKey || e.metaKey, range: e.shiftKey });
  });
  body.addEventListener('dblclick', (e) => {
    const row = e.target.closest('.tl-row');
    if (row && !e.target.closest('.idx-play')) play(Number(row.dataset.i));
  });
  body.addEventListener('contextmenu', (e) => {
    const row = e.target.closest('.tl-row');
    if (!row) return;
    const i = Number(row.dataset.i);
    if (!selected.has(i)) select(i);
    const idx = selectedIndices();
    const tracks = tracksAt(idx);
    const ctx = { ...(options.context || {}) };
    if (ctx.removeFromPlaylist) {
      const remove = ctx.removeFromPlaylist;
      ctx.removeFromPlaylist = () => remove(idx.map((k) => items[k]).filter(Boolean));
    }
    ctx.play = () => (tracks.length === 1 ? play(idx[0]) : player.playQueue(tracks, 0));
    if (layoutName === 'queue') ctx.removeLabel = ctx.removeLabel || t('queue.remove');
    openContextMenu(e, trackMenuItems(tracks, ctx));
  });

  el.addEventListener('keydown', (e) => {
    const last = items.length - 1;
    const cur = selected.size ? Math.max(...selected) : -1;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const next = Math.min(last, Math.max(0, (e.key === 'ArrowDown' ? cur + 1 : (selected.size ? Math.min(...selected) : 1) - 1)));
      select(next, { range: e.shiftKey, additive: e.shiftKey });
      scrollTo(next);
    } else if (e.key === 'Enter' && selected.size) {
      e.preventDefault();
      play(selectedIndices()[0]);
    } else if ((e.key === 'a' || e.key === 'A') && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      items.forEach((_, k) => selected.add(k));
      paintSelection();
    } else if ((e.key === 'Delete' || e.key === 'Backspace') && selected.size) {
      e.preventDefault();
      const idx = selectedIndices();
      if (options.context?.removeFromPlaylist) options.context.removeFromPlaylist(idx.map((k) => items[k]));
      else removeFromLibrary(tracksAt(idx));
    } else if (e.key === 'Escape' && selected.size) {
      selected.clear();
      paintSelection();
    }
  });

  // ─── Glisser-déposer ───────────────────────────────────────────────────────
  body.addEventListener('dragstart', (e) => {
    const row = e.target.closest('.tl-row');
    if (!row) return;
    const i = Number(row.dataset.i);
    if (!selected.has(i)) select(i);
    const idx = selectedIndices();
    startTrackDrag(e, tracksAt(idx));
    if (onMove) e.dataTransfer.setData(REORDER, JSON.stringify(idx));
  });

  const targetFromEvent = (e) => {
    const y = e.clientY - body.getBoundingClientRect().top;
    return Math.max(0, Math.min(items.length, Math.round(y / rowHeight)));
  };
  const clearDrop = () => {
    dropAt = -1;
    indicator.style.opacity = '0';
  };
  body.addEventListener('dragover', (e) => {
    if (!onMove || !e.dataTransfer.types.includes(REORDER)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
    dropAt = targetFromEvent(e);
    indicator.style.opacity = '1';
    indicator.style.transform = `translateY(${dropAt * rowHeight - 1}px)`;
  });
  body.addEventListener('dragleave', (e) => {
    if (!body.contains(e.relatedTarget)) clearDrop();
  });
  body.addEventListener('drop', (e) => {
    if (!onMove || !e.dataTransfer.types.includes(REORDER)) return;
    e.preventDefault();
    const from = JSON.parse(e.dataTransfer.getData(REORDER) || '[]');
    const to = dropAt >= 0 ? dropAt : targetFromEvent(e);
    clearDrop();
    selected.clear();
    onMove(from, to);
  });
  body.addEventListener('dragend', clearDrop);

  // ─── Cycle de vie ──────────────────────────────────────────────────────────
  scroller.addEventListener('scroll', schedule, { passive: true });
  const resize = new ResizeObserver(schedule);
  resize.observe(scroller);
  const offs = [onPlayer('track', refreshPlaying), onPlayer('state', refreshPlaying)];
  relayout();
  requestAnimationFrame(update);

  return {
    el,
    setItems(next) {
      items = next;
      selected.clear();
      relayout();
    },
    selectedTracks: () => tracksAt(selectedIndices()),
    refreshPlaying,
    destroy() {
      scroller.removeEventListener('scroll', schedule);
      resize.disconnect();
      offs.forEach((off) => off());
      cancelAnimationFrame(raf);
    },
  };
}

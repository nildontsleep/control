// Mode immersif : plein écran, rien que la pochette et les paroles animées, sur un fond aux couleurs de la pochette.
// Aucun contrôle n'est affiché : un clic sur la pochette lance ou met en pause, et une petite barre apparaît quand la
// souris bouge (paroles, quitter), puis se retire avec le curseur.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { settings, update } from '../settings.js';
import { player } from '../player.js';
import { parseLrc, lineAt } from '../lyrics/lrc.js';
import { coverEl } from './cover.js';
import { paletteOf, hsl } from './palette.js';

const api = window.control.lyrics;
const AWAKE_MS = 2800;
const ASIAN = /[\u3040-\u30ff\u3400-\u9fff\uac00-\ud7af]/;

const clamp = (v, a, b) => Math.min(b, Math.max(a, v));

export function createImmersive(host, { onExit }) {
  const art = h('div.im-art');
  const title = h('div.im-title');
  const artist = h('div.im-artist');
  const stage = h('div.im-stage', {}, art, h('div.im-caption', {}, title, artist));
  const lines = h('div.im-lines');
  const lyrics = h('div.im-lyrics', {}, lines);
  const lyricsBtn = h('button.im-btn', { type: 'button', 'data-tip': t('im.lyrics'), 'data-kbd': 'L', html: icon('mic-vocal', 19, 2), onclick: () => api_.toggleLyrics() });
  const exitBtn = h('button.im-btn', { type: 'button', 'data-tip': t('im.exit'), 'data-kbd': 'Échap', html: icon('minimize', 19, 2), onclick: () => onExit() });
  const root = h('div.im', { 'aria-hidden': 'true' },
    h('div.im-bg', {}, h('i.im-blob.b1'), h('i.im-blob.b2'), h('i.im-blob.b3'), h('i.im-blob.b4'), h('div.im-veil')),
    h('div.im-body', {}, stage, lyrics),
    h('div.im-hud', {}, lyricsBtn, exitBtn),
    h('div.im-hint', { text: t(matchMedia('(hover: none) and (pointer: coarse)').matches ? 'im.hintTouch' : 'im.hint') }));
  host.append(root);

  let on = false;
  let track = null;
  let token = 0;
  let raf = 0;
  let awakeTimer = 0;
  let manualUntil = 0;
  let offset = 0; // décalage vertical appliqué à toutes les lignes (équivaut à un défilement)
  let lastPlain = 0;
  let ly = { has: false, synced: false, lines: [], nodes: [], words: [], current: -2, lit: -1 };

  // ─── Pochette, titre, couleurs ─────────────────────────────────────────────
  function paintPalette(colors) {
    colors.forEach((c, i) => root.style.setProperty(`--c${i + 1}`, hsl(c)));
  }

  function setTrack(next) {
    track = next;
    const mine = ++token;
    art.replaceChildren(track ? h('div.im-swap', {}, coverEl(track, { className: 'im-cover' })) : '');
    title.textContent = track?.title || '';
    artist.textContent = [track?.artist, track?.album].filter(Boolean).join(' · ');
    paletteOf(track).then((colors) => { if (mine === token && colors) paintPalette(colors); });
    loadLyrics(track, mine);
    sync();
  }

  // ─── Paroles ───────────────────────────────────────────────────────────────
  function showLyrics() {
    return ly.has && settings.immersiveLyrics !== false;
  }

  function paintLayout() {
    root.classList.toggle('no-lyrics', !showLyrics());
    root.classList.toggle('lyrics-missing', !ly.has);
    lyricsBtn.classList.toggle('on', showLyrics());
    lyricsBtn.disabled = !ly.has;
  }

  function reset() {
    ly = { has: false, synced: false, lines: [], nodes: [], words: [], current: -2, lit: -1 };
    lines.replaceChildren();
    lines.className = 'im-lines';
    lines.style.width = '';
    setOffset(0);
    paintLayout();
  }

  async function loadLyrics(tr, mine) {
    reset();
    if (!tr || tr.kind !== 'audio') return;
    const res = await api.get(tr.id, false).catch(() => null);
    if (mine !== token) return;
    render(res?.ok ? res.data : null);
  }

  /** Découpe une ligne en mots (ou en caractères pour les écritures sans espaces) pour pouvoir les allumer un à un. */
  function split(text) {
    const parts = text.split(/\s+/).filter(Boolean);
    if (parts.length === 1 && ASIAN.test(text) && text.length > 4) return { parts: Array.from(text), spaced: false };
    return { parts, spaced: true };
  }

  function render(data) {
    const raw = data && (data.synced || data.plain);
    const parsed = raw ? parseLrc(raw) : null;
    if (!parsed || !parsed.lines.some((l) => l.text)) {
      paintLayout();
      return;
    }
    ly.has = true;
    ly.synced = parsed.synced;
    ly.lines = parsed.lines;
    lines.className = `im-lines${parsed.synced ? '' : ' plain-mode'}`;
    parsed.lines.forEach((line, i) => {
      let el;
      let words = null;
      if (!line.text) {
        el = h('div.im-line.gap', { html: '<i></i><i></i><i></i>' });
      } else {
        const { parts, spaced } = split(line.text);
        const spans = parts.map((p) => h('span.w', { text: p }));
        el = h(`div.im-line${parsed.synced ? '' : '.plain'}`);
        spans.forEach((s, k) => { if (k && spaced) el.append(' '); el.append(s); });
        const total = parts.reduce((n, p) => n + p.length, 0) || 1;
        let acc = 0;
        words = { spans, starts: parts.map((p) => { const s = acc / total; acc += p.length; return s; }) };
        if (parsed.synced) el.addEventListener('click', () => { player.seek(line.t); player.play(); });
      }
      el.style.setProperty('--d', String(parsed.synced ? 99 : 0));
      ly.nodes.push(el);
      ly.words.push(words);
    });
    lines.replaceChildren(...ly.nodes);
    paintLayout();
    // La mise en page change (colonne de paroles) : on attend qu'elle ait pris place pour mesurer les lignes.
    requestAnimationFrame(() => requestAnimationFrame(() => reposition(true)));
  }

  function setOffset(px) {
    offset = px;
    lines.style.setProperty('--off', `${px}px`);
  }

  /** Remet la ligne courante (ou la progression, pour des paroles sans minutage) à sa place. */
  function reposition(instant = false) {
    if (!ly.has) return;
    if (showLyrics() && lyrics.clientWidth > 120) lines.style.width = `${lyrics.clientWidth}px`;
    if (instant) {
      lines.classList.add('jump');
      requestAnimationFrame(() => requestAnimationFrame(() => lines.classList.remove('jump')));
    }
    lines.classList.remove('manual');
    if (!ly.synced) {
      lastPlain = 0;
      plainFrame(player.time, true);
      return;
    }
    const i = Math.max(0, ly.current);
    const el = ly.nodes[i];
    if (!el) return;
    setOffset(lyrics.clientHeight * 0.36 - (el.offsetTop + el.offsetHeight / 2));
  }

  function activate(i) {
    const prev = ly.current;
    ly.current = i;
    ly.lit = -1;
    const far = prev === -2 || Math.abs(i - prev) > 6;
    if (far) {
      lines.classList.add('jump');
      requestAnimationFrame(() => requestAnimationFrame(() => lines.classList.remove('jump')));
    }
    for (let k = 0; k < ly.nodes.length; k++) {
      const el = ly.nodes[k];
      const d = k - i;
      el.style.setProperty('--d', String(d));
      el.classList.toggle('active', d === 0);
      el.classList.toggle('past', d < 0);
    }
    const old = ly.words[prev];
    if (old) old.spans.forEach((s) => s.classList.remove('on'));
    if (performance.now() < manualUntil) return;
    if (i < 0) {
      setOffset(0);
      return;
    }
    const el = ly.nodes[i];
    setOffset(lyrics.clientHeight * 0.36 - (el.offsetTop + el.offsetHeight / 2));
  }

  /** Allume les mots de la ligne courante au rythme de son minutage. */
  function lightWords(i, time) {
    const w = ly.words[i];
    if (!w) return;
    const line = ly.lines[i];
    let next = null;
    for (let k = i + 1; k < ly.lines.length; k++) if (ly.lines[k].t > line.t) { next = ly.lines[k].t; break; }
    const room = (next ?? line.t + 5) - line.t - 0.15;
    const dur = Math.max(0.5, Math.min(room, Math.max(1.1, line.text.length * 0.07)));
    const p = clamp((time - line.t) / dur, 0, 1);
    let n = 0;
    while (n < w.starts.length && w.starts[n] <= p + 1e-6) n++;
    if (n === ly.lit) return;
    ly.lit = n;
    for (let k = 0; k < w.spans.length; k++) w.spans[k].classList.toggle('on', k < n);
  }

  function plainFrame(time, force = false) {
    const now = performance.now();
    if (!force && now - lastPlain < 300) return;
    lastPlain = now;
    if (now < manualUntil) return;
    const room = Math.max(0, lines.scrollHeight - lyrics.clientHeight * 0.75);
    const p = clamp(time / Math.max(1, player.duration), 0, 1);
    setOffset(lyrics.clientHeight * 0.3 - p * room);
  }

  function frame() {
    raf = requestAnimationFrame(frame);
    if (!ly.has) return;
    const time = player.time;
    if (!ly.synced) {
      plainFrame(time);
    } else {
      const i = lineAt(ly.lines, time + 0.2);
      if (i !== ly.current) activate(i);
      if (i >= 0) lightWords(i, time);
    }
    if (manualUntil && performance.now() > manualUntil) {
      manualUntil = 0;
      if (ly.synced) reposition();
      else lines.classList.remove('manual');
    }
  }

  // Défiler à la main : les lignes suivent la molette quelques secondes, puis reviennent sur la lecture.
  lyrics.addEventListener('wheel', (e) => {
    if (!ly.has) return;
    e.preventDefault();
    manualUntil = performance.now() + 3200;
    lines.classList.add('manual');
    const low = lyrics.clientHeight * 0.4 - lines.scrollHeight;
    setOffset(clamp(offset - e.deltaY, low, lyrics.clientHeight * 0.4));
  }, { passive: false });
  // La largeur des lignes est figée tant que la colonne est repliée : le texte ne se recompose pas pendant l'animation.
  new ResizeObserver(() => {
    if (on && showLyrics() && lyrics.clientWidth > 120) reposition(true);
  }).observe(lyrics);

  // ─── Éveil : la barre et le curseur disparaissent quand rien ne bouge ──────
  function wake() {
    root.classList.add('awake');
    host.classList.remove('no-cursor');
    clearTimeout(awakeTimer);
    awakeTimer = setTimeout(() => {
      root.classList.remove('awake');
      if (on) host.classList.add('no-cursor');
    }, AWAKE_MS);
  }
  root.addEventListener('pointermove', wake);
  root.addEventListener('pointerdown', wake);
  art.addEventListener('click', () => player.toggle());

  function sync() {
    root.classList.toggle('playing', player.playing);
    root.classList.toggle('paused', !player.playing);
  }

  const api_ = {
    get on() {
      return on;
    },
    get hasLyrics() {
      return ly.has;
    },
    show() {
      on = true;
      root.classList.add('on');
      root.setAttribute('aria-hidden', 'false');
      setTrack(player.track);
      wake();
      root.classList.remove('hint');
      void root.offsetWidth;
      root.classList.add('hint');
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(frame);
    },
    hide() {
      on = false;
      cancelAnimationFrame(raf);
      clearTimeout(awakeTimer);
      root.classList.remove('on', 'awake', 'hint');
      root.setAttribute('aria-hidden', 'true');
      host.classList.remove('no-cursor');
      token += 1; // abandonne un chargement de paroles en cours
    },
    setTrack,
    sync,
    toggleLyrics() {
      if (!ly.has) return;
      update({ immersiveLyrics: settings.immersiveLyrics === false });
      paintLayout();
      wake();
    },
    refresh: paintLayout,
  };
  return api_;
}

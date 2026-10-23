// Page de lecture : iframe vidrift (sans sandbox), progression et reprise par postMessage,
// plein écran géré par la page (fs=parent), épisode précédent / suivant.
// Doc : https://vidrift.net/docs — l'origine de chaque message est vérifiée.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { settings, update } from '../settings.js';
import { navigate, back, canBack, setParam } from '../router.js';
import { player } from '../player.js';
import {
  media, watch, parseWatchParam, watchParam, neighbour,
} from '../media/api.js';
import { embedUrl, streamOrigin } from '../media/vidrift.js';
import { button } from '../ui/controls.js';

const epKey = (season, episode) => `${season ?? ''}:${episode ?? ''}`;

function accentHex() {
  const raw = getComputedStyle(document.documentElement).getPropertyValue('--accent-rgb').trim();
  const [r, g, b] = raw.split(/\s+/).map(Number);
  if ([r, g, b].some((n) => !Number.isFinite(n))) return null;
  return [r, g, b].map((n) => n.toString(16).padStart(2, '0')).join('');
}

export function watchingView(root, param) {
  const req = parseWatchParam(param);
  const state = { type: req.type, id: String(req.id), season: req.season, episode: req.episode };
  let destroyed = false;
  let details = null;
  let iframe = null;
  let box = null;
  let seasonData = null;
  let lastPosition = 0;
  let lastDuration = 0;
  let gotProgress = false;
  let queue = Promise.resolve();
  const saved = new Map();
  const origin = streamOrigin();
  const app = document.getElementById('app');

  // Pas de musique en fond sous un film.
  player.pause();

  const page = h('div.watch-page', {}, h('div.theater.loading', {}, h('span.spinner', {}, ...Array.from({ length: 12 }, (_, i) => h('i', { style: { '--i': i } })))));
  root.append(page);

  // ─── Textes de l'en-tête ───────────────────────────────────────────────────
  const isMovie = () => state.type === 'movie' || (state.type === 'anime' && details?.isMovie);
  const titleEl = h('h1.wi-title');
  const subEl = h('p.wi-sub');
  const prevBtn = h('button.btn.btn-secondary', { type: 'button' }, h('span.btn-icon', { html: icon('skip-back', 15, 2) }), h('span', { text: t('watch.prev') }));
  const nextBtn = h('button.btn.btn-secondary', { type: 'button' }, h('span', { text: t('watch.next') }), h('span.btn-icon', { html: icon('skip-forward', 15, 2) }));
  const dubBox = h('div.seg-mini');

  const epLabel = () => (state.season != null ? `S${state.season} · E${state.episode}` : state.episode != null ? `${t('ep.short')} ${state.episode}` : '');

  function episodeName() {
    if (state.type === 'anime') return details?.episodes?.[state.episode - 1]?.name || '';
    return seasonData?.number === state.season ? seasonData.episodes.find((e) => e.number === state.episode)?.name || '' : '';
  }

  function paintInfo() {
    titleEl.textContent = details?.title || t('watch.untitled');
    subEl.textContent = isMovie() ? [details?.year, details?.runtime ? `${details.runtime} min` : null].filter(Boolean).join(' · ') : [epLabel(), episodeName()].filter(Boolean).join(' — ');
    const series = !isMovie() && !!details;
    const prev = series ? neighbour(details, state.season, state.episode, -1) : null;
    const next = series ? neighbour(details, state.season, state.episode, 1) : null;
    prevBtn.hidden = nextBtn.hidden = !series;
    prevBtn.disabled = !prev;
    nextBtn.disabled = !next;
    prevBtn.onclick = () => prev && playEpisode(prev.season, prev.episode);
    nextBtn.onclick = () => next && playEpisode(next.season, next.episode);
    if (state.type === 'anime') {
      const dub = !!settings.media?.dub;
      dubBox.replaceChildren(
        h('button', { type: 'button', class: dub ? '' : 'on', text: t('dub.sub'), onclick: () => setDub(false) }),
        h('button', { type: 'button', class: dub ? 'on' : '', text: t('dub.en'), onclick: () => setDub(true) }));
    } else {
      dubBox.replaceChildren();
    }
  }

  async function loadSeasonName() {
    if (state.type !== 'tv' || state.season == null || seasonData?.number === state.season) return;
    try {
      const data = await media.season(state.id, state.season);
      if (!destroyed && state.season === data.number) {
        seasonData = data;
        paintInfo();
      }
    } catch { /* le nom de l'épisode est facultatif */ }
  }

  // ─── Lecteur ───────────────────────────────────────────────────────────────
  const playerUrl = () => embedUrl(
    { type: state.type, id: state.id, season: state.season, episode: state.episode },
    { title: details?.title, color: accentHex(), poster: details?.backdrop, dub: !!settings.media?.dub },
  );

  const post = (message) => {
    try {
      iframe?.contentWindow?.postMessage(message, origin);
    } catch { /* iframe en cours de remplacement */ }
  };

  function sendFullscreenState() {
    post({ type: 'vidrift:fullscreen-state', on: document.fullscreenElement === box });
  }

  function resumeIfNeeded() {
    const row = saved.get(epKey(state.season, state.episode));
    if (!row || row.finished || row.position < 30) return;
    gotProgress = false;
    post({ type: 'vidrift:resume', currentTime: row.position });
    // Le lecteur applique la reprise une fois la vidéo prête ; un second envoi couvre un écouteur tardif.
    setTimeout(() => { if (!destroyed && !gotProgress) post({ type: 'vidrift:resume', currentTime: row.position }); }, 1800);
  }

  function mountPlayer() {
    iframe = h('iframe.player-frame', {
      src: playerUrl(),
      allow: 'autoplay; fullscreen; picture-in-picture; encrypted-media',
      title: details?.title || 'Lecteur',
    });
    iframe.addEventListener('load', () => {
      sendFullscreenState();
      resumeIfNeeded();
    });
    box.replaceChildren(iframe);
    box.classList.remove('loading');
  }

  function playEpisode(season, episode) {
    state.season = season;
    state.episode = episode;
    lastPosition = 0;
    setParam(watchParam(state.type, state.id, season, episode));
    paintInfo();
    loadSeasonName();
    iframe.src = playerUrl();
  }

  function setDub(on) {
    update({ media: { dub: on } });
    if (lastPosition > 0) saved.set(epKey(state.season, state.episode), { position: lastPosition, finished: false });
    paintInfo();
    iframe.src = playerUrl();
  }

  // ─── Messages du lecteur ───────────────────────────────────────────────────
  /** Clé du titre que le lecteur joue réellement (pour un animé : l'entrée AniList en cours). */
  function keyFrom(d) {
    if (state.type === 'anime') {
      const a = d.anilist;
      return { id: a?.id != null ? String(a.id) : state.id, season: null, episode: a?.episode ?? state.episode };
    }
    if (state.type === 'movie') return { id: state.id, season: null, episode: null };
    return {
      id: state.id,
      season: Number.isInteger(d.season) ? d.season : state.season,
      episode: Number.isInteger(d.episode) ? d.episode : state.episode,
    };
  }

  async function adopt(key) {
    const idChanged = key.id !== state.id;
    if (!idChanged && key.season === state.season && key.episode === state.episode) return;
    state.id = key.id;
    state.season = key.season;
    state.episode = key.episode;
    setParam(watchParam(state.type, state.id, state.season, state.episode));
    if (idChanged) {
      // Le lecteur est passé à la suite (autre saison AniList) : on récupère son titre.
      try {
        details = await media.details(state.type, state.id);
      } catch { /* on garde l'ancien titre */ }
    }
    if (destroyed) return;
    paintInfo();
    loadSeasonName();
  }

  async function record(d, finished) {
    await adopt(keyFrom(d));
    if (destroyed) return;
    const position = Number(d.currentTime) || 0;
    const duration = Number(d.duration) || lastDuration || 0;
    lastPosition = position;
    if (duration) lastDuration = duration;
    const row = { position, duration, finished: finished || (duration > 0 && position / duration >= 0.92) };
    saved.set(epKey(state.season, state.episode), row);
    await watch.save({
      type: state.type,
      id: state.id,
      season: state.season,
      episode: state.episode,
      position,
      duration,
      finished,
      title: details?.title || '',
      poster: details?.poster || null,
      backdrop: details?.backdrop || null,
      year: details?.year || null,
      genres: details?.genres || [],
    }).catch(() => {});
  }

  function onMessage(e) {
    if (!iframe || e.source !== iframe.contentWindow || e.origin !== origin) return;
    const d = e.data;
    if (!d || typeof d.type !== 'string') return;
    const enqueue = (fn) => { queue = queue.then(fn).catch(() => {}); };
    switch (d.type) {
      case 'vidrift:progress':
        gotProgress = true;
        enqueue(() => record(d, false));
        break;
      case 'vidrift:paused':
        enqueue(() => record(d, false));
        break;
      case 'vidrift:ended':
        enqueue(() => record(d, true));
        break;
      case 'vidrift:episode':
        // Le lecteur change d'épisode lui-même (liste intégrée ou « suivant ») : on suit.
        enqueue(() => adopt(keyFrom(d)));
        break;
      case 'vidrift:exit':
        goBack();
        break;
      case 'vidrift:fullscreen':
        if (d.on) box.requestFullscreen?.().catch(() => {});
        else if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
        break;
      default:
    }
  }

  function goBack() {
    if (canBack()) back();
    else navigate('title', `${state.type}:${state.id}`);
  }

  function onFullscreenChange() {
    app.classList.toggle('watch-fs', document.fullscreenElement === box);
    sendFullscreenState();
  }

  // ─── Page ──────────────────────────────────────────────────────────────────
  async function start() {
    const [d, rows] = await Promise.all([
      media.details(state.type, state.id).catch(() => null),
      watch.show(state.type, state.id).catch(() => []),
    ]);
    if (destroyed) return;
    details = d;
    for (const r of rows) saved.set(epKey(r.season, r.episode), r);
    if (state.type === 'anime' && details?.isMovie) state.episode = null;
    else if (state.type === 'anime' && state.episode == null) state.episode = 1;
    if (state.type === 'tv') {
      state.season ??= details?.seasons?.[0]?.number ?? 1;
      state.episode ??= 1;
    }

    box = h('div.theater');
    const backBtn = h('button.btn.btn-secondary', { type: 'button', onclick: goBack }, h('span.btn-icon', { html: icon('arrow-left', 15, 2) }), h('span', { text: t('watch.back') }));
    page.replaceChildren(
      box,
      h('div.watch-info', {},
        h('div.wi-main', {}, titleEl, subEl),
        h('div.wi-actions', {},
          backBtn,
          prevBtn,
          nextBtn,
          button(t('watch.sheet'), { icon: 'info', onClick: () => navigate('title', `${state.type}:${state.id}`) }),
          dubBox)),
      h('p.watch-note', { text: t('watch.note') }));
    paintInfo();
    loadSeasonName();
    mountPlayer();
  }

  window.addEventListener('message', onMessage);
  document.addEventListener('fullscreenchange', onFullscreenChange);
  start();

  return {
    destroy() {
      destroyed = true;
      window.removeEventListener('message', onMessage);
      document.removeEventListener('fullscreenchange', onFullscreenChange);
      app.classList.remove('watch-fs');
      if (document.fullscreenElement === box) document.exitFullscreen().catch(() => {});
      iframe?.remove();
    },
  };
}

// Fiche d'un film, d'une série ou d'un animé : lecture / reprise, épisodes, casting, titres liés.

import { h, clamp } from '../dom.js';
import { icon } from '../icons.js';
import { t, currentLanguage } from '../i18n.js';
import { fmtLong, fmtTime } from '../format.js';
import { settings, update } from '../settings.js';
import { navigate } from '../router.js';
import {
  media, watch, inList, toggleList, onList, watchParam, neighbour, loadList, MediaFail,
} from '../media/api.js';
import {
  posterShelf, shelf, posterCard, errorState, kindLabel, animeGenreLabel,
} from '../media/ui.js';
import { toast } from '../ui/feedback.js';
import { button } from '../ui/controls.js';

const EP_GROUP = 50;
const RELATION_KEYS = {
  PREQUEL: 'rel.prequel', SEQUEL: 'rel.sequel', PARENT: 'rel.parent', SUMMARY: 'rel.summary',
  SIDE_STORY: 'rel.side', SPIN_OFF: 'rel.spinoff', ALTERNATIVE: 'rel.alt',
};

const epKey = (season, episode) => `${season ?? ''}:${episode ?? ''}`;

function skeleton() {
  return h('div.td-skeleton', {}, h('div.td-sk-hero.sk'), h('div.td-sk-line.sk'), h('div.td-sk-line.short.sk'));
}

export function titleView(root, param) {
  const [type, ...rest] = String(param || '').split(':');
  const id = rest.join(':');
  let destroyed = false;
  let details = null;
  let progress = [];
  let seasonCache = new Map();
  let offList = null;
  let episodesBox = null;
  let heroActions = null;
  let currentSeason = null;
  let group = 0;

  const page = h('div.title-page');
  root.append(page);

  // ─── Progression ───────────────────────────────────────────────────────────
  const byEpisode = () => new Map(progress.map((r) => [epKey(r.season, r.episode), r]));

  /** Où reprendre : épisode en cours, épisode suivant, ou début. */
  function target() {
    const last = progress[0];
    const isMovie = type === 'movie' || details.isMovie;
    if (isMovie) {
      const resume = last && !last.finished && last.position >= 30 ? last : null;
      return { season: null, episode: null, resume, label: resume ? t('title.resume') : t('common.play'), sub: resume ? t('resume.left', { t: fmtLong(resume.duration - resume.position) }) : null };
    }
    const firstSeason = type === 'tv' ? (details.seasons[0]?.number ?? 1) : null;
    const tag = (season, episode) => (season != null ? `S${season} · E${episode}` : `${t('ep.short')} ${episode}`);
    if (!last) return { season: firstSeason, episode: 1, resume: null, label: t('common.play'), sub: tag(firstSeason, 1) };
    if (!last.finished) {
      // Épisode entamé : on y retourne, avec reprise seulement s'il a vraiment avancé.
      const far = last.position >= 30;
      return {
        season: last.season,
        episode: last.episode,
        resume: far ? last : null,
        label: far ? t('title.resume') : t('common.play'),
        sub: far ? `${tag(last.season, last.episode)} · ${t('resume.left', { t: fmtLong(last.duration - last.position) })}` : tag(last.season, last.episode),
      };
    }
    const next = neighbour(details, last.season, last.episode, 1);
    if (next) return { season: next.season, episode: next.episode, resume: null, label: t('title.next'), sub: tag(next.season, next.episode) };
    return { season: firstSeason, episode: 1, resume: null, label: t('title.again'), sub: tag(firstSeason, 1) };
  }

  const play = (season = null, episode = null) => navigate('watching', watchParam(type, id, season, episode));

  // ─── Fiche ─────────────────────────────────────────────────────────────────
  function metaChips() {
    const d = details;
    const chips = [];
    if (d.year) chips.push(h('span', { text: String(d.year) }));
    if (d.runtime) chips.push(h('span', { text: fmtLong(d.runtime * 60) }));
    if (d.certification) chips.push(h('span.badge', { text: d.certification }));
    if (d.rating) chips.push(h('span.rating', { html: `${icon('star', 13, 0)}<span>${d.rating.toFixed(1)}</span>` }));
    if (type === 'tv' && d.numberOfSeasons) chips.push(h('span', { text: t('title.seasons', { n: d.numberOfSeasons }) }));
    if (type === 'anime' && !d.isMovie && d.episodes?.length) chips.push(h('span', { text: t('title.episodes', { n: d.episodes.length }) }));
    if (type === 'anime' && d.format && d.format !== 'TV') chips.push(h('span', { text: d.format.replace('_', ' ') }));
    return chips;
  }

  function creditsLine() {
    const d = details;
    const lines = [];
    if (d.creators?.length) lines.push([type === 'movie' ? t('title.directedBy') : t('title.createdBy'), d.creators.join(', ')]);
    if (d.studios?.length) lines.push([t('title.studio'), d.studios.join(', ')]);
    if (type === 'anime' && d.nextAiring) {
      const date = new Date(d.nextAiring.at).toLocaleDateString(currentLanguage(), { weekday: 'long', day: 'numeric', month: 'long' });
      lines.push([t('title.nextEpisode'), `${t('ep.short')} ${d.nextAiring.episode} · ${date}`]);
    }
    return lines.map(([k, v]) => h('div.td-credit', {}, h('span', { text: k }), h('strong', { text: v })));
  }

  function paintActions() {
    if (!heroActions || !details) return;
    const goal = target();
    const listed = inList(type, id);
    const main = h('button.btn.btn-primary.btn-large', { type: 'button', onclick: () => play(goal.season, goal.episode) },
      h('span.btn-icon', { html: icon('play', 18, 0) }),
      h('span.btn-stack', {}, h('span', { text: goal.label }), goal.sub ? h('small', { text: goal.sub }) : null));
    const listBtn = button(listed ? t('list.inList') : t('list.add'), {
      icon: listed ? 'bookmark-check' : 'bookmark-plus',
      onClick: async () => {
        const added = await toggleList(details);
        toast(added ? t('toast.listAdded', { title: details.title }) : t('toast.listRemoved', { title: details.title }), added ? 'bookmark-check' : 'bookmark');
      },
    });
    const items = [main, listBtn];
    if (details.trailer) items.push(button(t('title.trailer'), { icon: 'clapperboard', onClick: () => window.control.app.openExternal(`https://www.youtube.com/watch?v=${details.trailer}`) }));
    heroActions.replaceChildren(...items);
    if (type === 'anime') {
      const dub = !!settings.media?.dub;
      const seg = h('div.seg-mini', { role: 'radiogroup' },
        h('button', { type: 'button', class: dub ? '' : 'on', text: t('dub.sub'), onclick: () => { update({ media: { dub: false } }); paintActions(); } }),
        h('button', { type: 'button', class: dub ? 'on' : '', text: t('dub.en'), onclick: () => { update({ media: { dub: true } }); paintActions(); } }));
      heroActions.append(seg);
    }
  }

  function hero() {
    const d = details;
    const genres = (d.genres || []).map((g) => (type === 'anime' ? animeGenreLabel(g, currentLanguage()) : g));
    heroActions = h('div.td-actions');
    const bg = d.backdrop || d.poster;
    const el = h('section.td-hero', {},
      h('div.td-bg', {}, bg ? h('img', { src: bg, alt: '', referrerpolicy: 'no-referrer' }) : null),
      h('div.td-shade'),
      h('div.td-body', {},
        h('div.td-poster', {}, d.poster ? h('img', { src: d.poster, alt: '', referrerpolicy: 'no-referrer' }) : h('div.pcard-blank', { text: d.title })),
        h('div.td-text', {},
          h('div.td-kind', { text: kindLabel(type) }),
          h('h1', { text: d.title }),
          d.original && d.original !== d.title && type === 'anime' ? h('div.td-original', { text: d.original }) : null,
          h('div.td-meta', {}, ...metaChips()),
          genres.length ? h('div.td-genres', {}, ...genres.map((g) => h('span', { text: g }))) : null,
          d.tagline ? h('div.td-tagline', { text: d.tagline }) : null,
          d.overview ? h('p.td-overview', { text: d.overview }) : null,
          heroActions,
          h('div.td-credits', {}, ...creditsLine()))));
    paintActions();
    return el;
  }

  // ─── Épisodes ──────────────────────────────────────────────────────────────
  function episodeRow(ep, season) {
    const done = byEpisode().get(epKey(season, ep.number));
    const pct = done && done.duration ? clamp(done.position / done.duration, 0, 1) : 0;
    const art = h('div.ep-art', {}, ep.still ? h('img', { src: ep.still, alt: '', loading: 'lazy', referrerpolicy: 'no-referrer' }) : h('span.ep-num-big', { text: String(ep.number) }));
    art.append(h('span.ep-play', { html: icon('play', 18, 0) }));
    if (done && !done.finished && pct > 0) art.append(h('div.pcard-progress', {}, h('span', { style: { width: `${Math.round(pct * 100)}%` } })));
    const meta = [ep.runtime ? `${ep.runtime} min` : null, ep.airDate ? new Date(ep.airDate).toLocaleDateString(currentLanguage(), { day: 'numeric', month: 'short', year: 'numeric' }) : null].filter(Boolean).join(' · ');
    const row = h('button.ep-row', { type: 'button' },
      art,
      h('div.ep-text', {},
        h('div.ep-title', {}, h('span.ep-num', { text: `${ep.number}.` }), h('span', { text: ep.name || `${t('ep.word')} ${ep.number}` }),
          done?.finished ? h('span.ep-done', { html: icon('circle-check', 15, 2) }) : null),
        meta ? h('div.ep-meta', { text: meta }) : null,
        ep.overview ? h('p.ep-overview', { text: ep.overview }) : null,
        done && !done.finished && done.position >= 30 ? h('div.ep-resume', { text: t('ep.resumeAt', { t: fmtTime(done.position) }) }) : null));
    row.addEventListener('click', () => play(season, ep.number));
    return row;
  }

  async function paintTvEpisodes() {
    const seasons = details.seasons;
    if (!seasons.length) return;
    const tabs = h('div.season-tabs', {}, ...seasons.map((s) => h(`button.chip${s.number === currentSeason ? '.on' : ''}`, {
      type: 'button',
      text: seasons.length > 12 ? `S${s.number}` : s.name || `${t('season.word')} ${s.number}`,
      onclick: () => { currentSeason = s.number; paintTvEpisodes(); },
    })));
    const list = h('div.ep-list', {}, ...Array.from({ length: 4 }, () => h('div.ep-row.sk-row', {}, h('div.ep-art.sk'), h('div.ep-text', {}, h('div.sk', { text: ' '.repeat(30) })))));
    episodesBox.replaceChildren(h('h2.block-title', { text: t('title.episodesHeading') }), tabs, list);
    try {
      let data = seasonCache.get(currentSeason);
      if (!data) {
        data = await media.season(id, currentSeason);
        seasonCache.set(currentSeason, data);
      }
      if (destroyed) return;
      list.replaceChildren(...data.episodes.map((e) => episodeRow(e, currentSeason)));
    } catch (err) {
      if (!destroyed) list.replaceChildren(errorState(err, paintTvEpisodes));
    }
  }

  function paintAnimeEpisodes() {
    const eps = details.episodes;
    if (!eps.length) return;
    const groups = Math.ceil(eps.length / EP_GROUP);
    const nodes = [h('h2.block-title', { text: t('title.episodesHeading') })];
    if (groups > 1) {
      nodes.push(h('div.season-tabs', {}, ...Array.from({ length: groups }, (_, i) => h(`button.chip${i === group ? '.on' : ''}`, {
        type: 'button',
        text: `${i * EP_GROUP + 1}–${Math.min(eps.length, (i + 1) * EP_GROUP)}`,
        onclick: () => { group = i; paintAnimeEpisodes(); },
      }))));
    }
    nodes.push(h('div.ep-list', {}, ...eps.slice(group * EP_GROUP, (group + 1) * EP_GROUP).map((e) => episodeRow(e, null))));
    episodesBox.replaceChildren(...nodes);
  }

  function castRow() {
    const cast = (details.cast || []).filter((c) => c.name);
    if (!cast.length) return null;
    return shelf({
      title: t('title.cast'),
      items: cast,
      build: (c) => h('div.cast-card', {},
        h('div.cast-photo', {}, c.profile ? h('img', { src: c.profile, alt: '', loading: 'lazy', referrerpolicy: 'no-referrer' }) : h('span', { text: c.name.slice(0, 1) })),
        h('div.cast-name', { text: c.name }),
        h('div.cast-role', { text: c.role || '' })),
    });
  }

  function relatedRow() {
    if (!details.related?.length) return null;
    return shelf({
      title: t('title.related'),
      items: details.related,
      build: (item) => {
        const card = posterCard(item);
        card.querySelector('.pcard-sub').textContent = [t(RELATION_KEYS[item.relation] || 'rel.alt'), item.year].filter(Boolean).join(' · ');
        return card;
      },
    });
  }

  // ─── Chargement ────────────────────────────────────────────────────────────
  function render() {
    const parts = [hero()];
    const isSeries = type === 'tv' || (type === 'anime' && !details.isMovie);
    if (isSeries) {
      episodesBox = h('section.block.episodes');
      parts.push(episodesBox);
    }
    parts.push(relatedRow(), castRow());
    if (details.recommendations?.length) parts.push(posterShelf({ title: t('title.recommended'), items: details.recommendations }));
    if (details.similar?.length) parts.push(posterShelf({ title: t('title.similar'), items: details.similar }));
    parts.push(h('div.page-foot'));
    page.replaceChildren(...parts.filter(Boolean));

    if (type === 'tv') {
      const goal = target();
      currentSeason = goal.season ?? details.seasons[0]?.number ?? 1;
      paintTvEpisodes();
    } else if (type === 'anime' && !details.isMovie) {
      const goal = target();
      group = Math.max(0, Math.floor(((goal.episode || 1) - 1) / EP_GROUP));
      paintAnimeEpisodes();
    }
  }

  async function load() {
    page.replaceChildren(skeleton());
    try {
      const [d, rows] = await Promise.all([media.details(type, id), watch.show(type, id).catch(() => [])]);
      if (destroyed) return;
      details = d;
      progress = rows;
      render();
    } catch (err) {
      if (destroyed) return;
      const empty = errorState(err, load);
      if (err instanceof MediaFail && err.code === 'notfound') empty.querySelector('p').textContent = t('err.notfound');
      page.replaceChildren(empty);
    }
  }

  offList = onList(() => paintActions());
  loadList().catch(() => {});
  load();
  return { destroy: () => { destroyed = true; offList?.(); } };
}

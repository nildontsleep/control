// Accueil « Films & séries » (TMDB) et « Animés » (AniList) : bannière, reprise, ma liste, genres, rangées.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t, currentLanguage } from '../i18n.js';
import { update } from '../settings.js';
import { navigate } from '../router.js';
import { media, watch, listItems, onList, watchParam, loadList, MediaFail } from '../media/api.js';
import {
  posterShelf, shelf, resumeCard, heroBanner, genreChips, skeletonShelf, errorState, animeGenreLabel,
} from '../media/ui.js';
import { quickPlay } from '../media/play.js';
import { toast } from '../ui/feedback.js';
import { button } from '../ui/controls.js';
import { pageHead } from './shared.js';

const TABS = ['all', 'movie', 'tv'];

/** Carte de saisie de la clé TMDB (affichée tant qu'aucune clé n'est enregistrée). */
export function keyCard(onSaved) {
  const input = h('input.text-input', { type: 'text', placeholder: t('key.placeholder'), spellcheck: 'false', autocomplete: 'off' });
  const status = h('div.key-status');
  const save = async () => {
    const key = input.value.trim();
    if (!key) return;
    status.textContent = t('key.testing');
    status.className = 'key-status';
    try {
      await media.testKey(key);
      update({ media: { tmdbKey: key } });
      toast(t('key.saved'), 'circle-check');
      onSaved?.();
    } catch (err) {
      status.textContent = err instanceof MediaFail && err.code === 'auth' ? t('key.invalid') : t('err.network');
      status.className = 'key-status bad';
    }
  };
  input.addEventListener('keydown', (e) => { if (e.key === 'Enter') save(); });
  return h('div.key-card', {},
    h('div.key-icon', { html: icon('key-round', 26, 1.6) }),
    h('div.key-text', {},
      h('h2', { text: t('key.title') }),
      h('p', { text: t('key.text') }),
      h('div.key-form', {}, input, button(t('common.save'), { kind: 'primary', onClick: save })),
      status,
      h('div.key-links', {},
        h('button.link-btn', { type: 'button', text: t('key.get'), onclick: () => window.control.app.openExternal('https://www.themoviedb.org/settings/api') }),
        h('button.link-btn', { type: 'button', text: t('key.anime'), onclick: () => navigate('anime') }))));
}

function homeView(root, { tab, provider }) {
  let destroyed = false;
  let banner = null;
  const content = h('div.watch-content');
  const isAnime = provider === 'anime';
  const lang = currentLanguage();

  const tabs = isAnime ? null : h('div.tabs', {}, ...TABS.map((id) => h(`button.tab${id === tab ? '.on' : ''}`, {
    type: 'button',
    text: t(`tab.${id}`),
    onclick: () => navigate('watch', id, { replace: true }),
  })));

  root.append(
    pageHead({ title: isAnime ? t('nav.anime') : t('nav.movies'), subtitle: isAnime ? t('anime.subtitle') : t('movies.subtitle'), actions: tabs }),
    content,
    h('div.page-foot'));

  const typeFilter = (type) => (isAnime ? type === 'anime' : tab === 'all' ? type !== 'anime' : type === tab);

  function shelfTitle(id) {
    return t(`shelf.${id}`);
  }

  async function removeEntry(entry) {
    await watch.remove(entry.type, entry.id);
    load();
  }

  function localRows(continuing) {
    const rows = [];
    const resume = continuing.filter((e) => typeFilter(e.type));
    if (resume.length) {
      rows.push(shelf({
        title: t('shelf.continue'),
        items: resume,
        build: (entry) => resumeCard(entry, {
          onPlay: () => navigate('watching', watchParam(entry.type, entry.id, entry.season, entry.episode)),
          onRemove: () => removeEntry(entry),
        }),
      }));
    }
    const saved = listItems().filter((e) => typeFilter(e.type));
    if (saved.length) {
      rows.push(posterShelf({ title: t('shelf.myList'), items: saved.slice(0, 24).map((e) => ({ ...e, key: `${e.type}:${e.id}` })), onMore: () => navigate('mylist'), card: { showKind: !isAnime && tab === 'all' } }));
    }
    return rows;
  }

  function render(home, continuing) {
    banner?.destroy?.();
    const first = home.shelves.find((s) => s.items.some((i) => i.backdrop));
    banner = first ? heroBanner(first.items.filter((i) => i.backdrop), { onPlay: quickPlay }) : null;

    const parts = [banner];
    if (home.genres?.length) {
      const chips = home.genres.map((g) => ({
        ...g,
        label: isAnime ? animeGenreLabel(g.name, lang) : g.name,
      }));
      parts.push(genreChips(chips, {
        onPick: (g) => navigate('shelf', isAnime ? `anime:genre:${g.id}|${g.label}` : `tmdb:discover/${tab}/${g.id}|${g.label}`),
      }));
    }
    parts.push(...localRows(continuing));
    for (const s of home.shelves) {
      parts.push(posterShelf({
        title: shelfTitle(s.id),
        items: s.items,
        onMore: () => navigate('shelf', `${s.source}|${shelfTitle(s.id)}`),
        card: { showKind: !isAnime && tab === 'all' },
      }));
    }
    content.replaceChildren(...parts.filter(Boolean));
  }

  async function load() {
    content.replaceChildren(skeletonShelf(), skeletonShelf(), skeletonShelf());
    try {
      const [home, continuing] = await Promise.all([media.home(isAnime ? 'anime' : tab), watch.continue().catch(() => [])]);
      if (!destroyed) render(home, continuing);
    } catch (err) {
      if (destroyed) return;
      content.replaceChildren(err.code === 'nokey' ? h('div.watch-setup', {}, keyCard(load)) : errorState(err, load));
    }
  }

  const offList = onList(() => {
    // « Ma liste » a changé (depuis la bannière, un menu…) : on recalcule les rangées locales seulement.
    if (!destroyed && content.querySelector('.shelf')) load();
  });
  loadList().catch(() => {}).finally(() => { if (!destroyed) load(); });
  return { destroy: () => { destroyed = true; banner?.destroy?.(); offList(); } };
}

export const moviesView = (root, param) => homeView(root, { tab: TABS.includes(param) ? param : 'all', provider: 'tmdb' });
export const animeView = (root) => homeView(root, { tab: 'anime', provider: 'anime' });

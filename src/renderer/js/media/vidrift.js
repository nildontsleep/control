// Lecteur vidrift : construction des URL d'intégration et vérification de l'origine des messages.
// Voir https://vidrift.net/docs — une iframe sans attribut sandbox, une origine vérifiée à chaque message.

import { settings } from '../settings.js';

const OFFICIAL = ['https://embed.vidrift.net', 'https://embed.vidrift.in'];

export function streamBase() {
  const override = window.control.env?.streamOverride;
  if (override) return override;
  return OFFICIAL.includes(settings.media?.streamBase) ? settings.media.streamBase : OFFICIAL[0];
}

export const streamOrigin = () => new URL(streamBase()).origin;

/**
 * item : { type: 'movie'|'tv'|'anime', id, season?, episode? }
 * opts : { title, color, poster, dub }
 */
export function embedUrl({ type, id, season, episode }, { title, color, poster, dub } = {}) {
  const base = streamBase();
  let path;
  if (type === 'movie') path = `/embed/movie/${id}`;
  else if (type === 'tv') path = `/embed/tv/${id}/${season ?? 1}/${episode ?? 1}`;
  else path = episode == null ? `/embed/anilist/${id}` : `/embed/anilist/${id}/${episode}`; // un film d'animation n'a pas d'épisode

  const params = new URLSearchParams();
  if (title) params.set('title', title);
  params.set('brand', 'control');
  if (/^#?[0-9a-f]{6}$/i.test(color || '')) params.set('brandColor', color.replace('#', ''));
  if (poster && /^https:\/\//.test(poster)) params.set('poster', poster);
  params.set('exit', '1');
  params.set('fs', 'parent');
  params.set('font', 'Inter');
  params.set('hide', 'cast');
  if (type === 'anime' && dub) params.set('dub', '1');
  return `${base}${path}?${params}`;
}

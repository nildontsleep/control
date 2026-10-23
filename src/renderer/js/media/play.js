// Lancer la lecture d'un titre depuis une affiche ou une bannière.

import { navigate } from '../router.js';
import { watch, watchParam } from './api.js';
import { titleRoute } from './ui.js';

/**
 * Film, ou épisode en cours / premier épisode : on lance directement.
 * Si le dernier épisode est terminé, la fiche sait calculer le suivant : on l'ouvre.
 */
export async function quickPlay(item) {
  if (item.type === 'movie') return navigate('watching', watchParam('movie', item.id));
  let rows = [];
  try {
    rows = await watch.show(item.type, item.id);
  } catch { /* hors ligne : on retombe sur la fiche */ }
  const last = rows[0];
  if (!last) {
    if (item.type === 'anime' && item.format === 'MOVIE') return navigate('watching', watchParam('anime', item.id));
    return navigate('watching', watchParam(item.type, item.id, item.type === 'tv' ? 1 : null, 1));
  }
  if (!last.finished) return navigate('watching', watchParam(item.type, item.id, last.season, last.episode));
  return titleRoute(item);
}

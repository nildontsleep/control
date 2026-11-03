// Normalisation du volume : le gain (en dB) à appliquer à un morceau pour qu'il sonne au niveau voulu.
// Le niveau d'un morceau (LUFS) et sa crête vraie (dBTP) sont mesurés par le noyau ; un morceau pas encore mesuré reste inchangé.

export const MIN_GAIN = -18; // dB : on n'atténue pas davantage (un morceau très fort reste fort)
export const MAX_GAIN = 9; // dB : on ne relève pas davantage (un morceau très faible reste faible)
const HEADROOM = -1; // dBTP : une relève ne doit jamais pousser la crête au-delà

/**
 * track : { loudness, truePeak } ; target : niveau voulu en LUFS ; boost : relever est possible (graphe Web Audio),
 * sinon seul l'atténuation l'est (volume de l'élément média).
 */
export function gainDb(track, { enabled, target = -16, boost = true } = {}) {
  if (!enabled || !track || !Number.isFinite(track.loudness)) return 0;
  let db = target - track.loudness;
  db = Math.max(MIN_GAIN, Math.min(MAX_GAIN, db));
  if (db > 0) {
    if (!boost) return 0;
    // La relève s'arrête avant d'écrêter : sans crête connue, on reste prudent.
    const room = Number.isFinite(track.truePeak) ? HEADROOM - track.truePeak : 0;
    db = Math.max(0, Math.min(db, room));
  }
  return db;
}

export const toLinear = (db) => 10 ** (db / 20);

const asked = new Set();

/** Demande au noyau le niveau des morceaux que le lecteur ne connaît pas encore (mesurés depuis le chargement de la page). */
export async function fillLevels(tracks, api = window.control?.loudness) {
  const missing = tracks.filter((t) => t && t.kind !== 'video' && !Number.isFinite(t.loudness) && !asked.has(t.id));
  if (!missing.length || !api) return false;
  for (const t of missing) asked.add(t.id);
  try {
    const levels = await api.levels(missing.map((t) => t.id));
    let any = false;
    for (const t of missing) {
      const l = levels?.[t.id];
      if (l && Number.isFinite(l.loudness)) {
        t.loudness = l.loudness;
        t.truePeak = l.truePeak;
        any = true;
      } else {
        asked.delete(t.id); // pas encore mesuré : on redemandera plus tard
      }
    }
    return any;
  } catch {
    return false;
  }
}

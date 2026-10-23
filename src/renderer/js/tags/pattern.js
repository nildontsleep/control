// Motifs façon mp3tag (%artist% - %title%) : composer un nom de fichier, relire des tags depuis un nom,
// et quelques transformations de texte. Fonctions pures, sans accès à l'interface.

export const PLACEHOLDERS = {
  title: 'title',
  artist: 'artist',
  album: 'album',
  albumartist: 'albumArtist',
  genre: 'genre',
  composer: 'composer',
  year: 'year',
  track: 'trackNo',
  tracktotal: 'trackTotal',
  disc: 'discNo',
  disctotal: 'discTotal',
};

const NUMERIC = new Set(['year', 'trackNo', 'trackTotal', 'discNo', 'discTotal']);
const TOKEN = /%([a-z]+)%/gi;

export const PATTERN_PRESETS = [
  '%track% - %title%',
  '%artist% - %title%',
  '%track% - %artist% - %title%',
  '%track%. %title%',
  '%disc%-%track% %title%',
  '%artist% - %album% - %track% - %title%',
];

const escapeRegex = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** Texte d'un champ tel qu'il apparaît dans un nom de fichier (pistes sur au moins 2 chiffres). */
function fieldText(fields, key, name) {
  const value = fields[key];
  if (value === undefined || value === null || value === '' || value === 0) return '';
  if (name === 'track' || name === 'tracktotal') return String(value).padStart(2, '0');
  return String(value);
}

/** Compose un nom (sans extension) à partir d'un motif et des champs d'un morceau. */
export function renderPattern(pattern, fields) {
  const raw = String(pattern || '').replace(TOKEN, (match, name) => {
    const key = PLACEHOLDERS[name.toLowerCase()];
    return key ? fieldText(fields, key, name.toLowerCase()) : match;
  });
  return raw
    .replace(/\s+/g, ' ')
    .replace(/(?:\s*[-–—._]\s*){2,}/g, ' - ')
    .replace(/^[\s\-–—._]+|[\s\-–—._]+$/g, '')
    .trim();
}

/** Transforme un motif en expression régulière + liste des champs capturés (ou null si le motif est vide). */
export function compilePattern(pattern) {
  const keys = [];
  let source = '';
  let last = 0;
  const text = String(pattern || '');
  for (const m of text.matchAll(TOKEN)) {
    const name = m[1].toLowerCase();
    const key = PLACEHOLDERS[name];
    if (!key) continue;
    source += escapeRegex(text.slice(last, m.index)).replace(/\\? /g, '\\s*');
    source += NUMERIC.has(key) ? '(\\d+)' : '(.+?)';
    keys.push(key);
    last = m.index + m[0].length;
  }
  if (!keys.length) return null;
  source += escapeRegex(text.slice(last)).replace(/\\? /g, '\\s*');
  return { regex: new RegExp(`^\\s*${source}\\s*$`, 'i'), keys };
}

/** Lit des champs depuis un nom de fichier (sans extension). Retourne null si le nom ne correspond pas. */
export function parsePattern(compiled, basename) {
  if (!compiled) return null;
  const hit = compiled.regex.exec(basename);
  if (!hit) return null;
  const out = {};
  compiled.keys.forEach((key, i) => {
    const value = hit[i + 1].trim();
    out[key] = NUMERIC.has(key) ? Number(value) : value;
  });
  return out;
}

// ─── Transformations de texte ────────────────────────────────────────────────
const SMALL = new Set(['a', 'an', 'and', 'as', 'at', 'but', 'by', 'for', 'in', 'of', 'on', 'or', 'the', 'to', 'vs', 'de', 'du', 'des', 'la', 'le', 'les', 'un', 'une', 'et', 'ou', 'à', 'au', 'aux', 'en', 'sur', 'sous']);

const upperFirst = (word) => word.charAt(0).toLocaleUpperCase() + word.slice(1);

export const CASES = {
  title: (s) => s.toLocaleLowerCase().replace(/(^|[\s([{"'«\-–—/])(\p{L}[\p{L}\p{M}'’]*)/gu, (m, lead, word, offset) => {
    const bare = word.toLocaleLowerCase();
    const first = offset === 0 && lead === '';
    return lead + (!first && SMALL.has(bare) ? bare : upperFirst(word));
  }),
  sentence: (s) => upperFirst(s.toLocaleLowerCase()),
  lower: (s) => s.toLocaleLowerCase(),
  upper: (s) => s.toLocaleUpperCase(),
};

/** Retire les espaces en trop et les parasites de titres (« (Official Video) », « [Lyrics] »…). */
export function tidy(s) {
  return String(s)
    .replace(/\s*[([{]\s*(?:official\s*(?:music\s*)?(?:video|audio|lyric\s*video)|lyrics?|lyric\s*video|audio|hd|hq|4k|clip\s*officiel|visuali[sz]er)\s*[)\]}]/gi, '')
    .replace(/\s+/g, ' ')
    .trim();
}

export function replaceText(value, { find, replace = '', regex = false, matchCase = false }) {
  if (!find) return value;
  if (regex) {
    let re;
    try {
      re = new RegExp(find, matchCase ? 'gu' : 'giu');
    } catch {
      return value;
    }
    return String(value).replace(re, replace);
  }
  return String(value).replace(new RegExp(escapeRegex(find), matchCase ? 'gu' : 'giu'), () => replace);
}

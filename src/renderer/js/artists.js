// Noms d'artistes : une étiquette « A, B feat. C » désigne plusieurs artistes. On la découpe, on retire les pseudos de
// réseaux sociaux (« @nom ») et les doublons, pour que chaque artiste n'ait qu'une fiche et que les listes restent lisibles.
// Module sans dépendance : testé directement par la suite du serveur.

const fold = (s) => String(s || '').normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase();

// « COEURCO @1coeurco », « Bi (t3rmina) - (@t3rminadoo) », « COEURCO ig @1coeurco » : on enlève le pseudo et son habillage.
const HANDLE = /\s*[-–—]?\s*\(?\s*(?:ig|insta|instagram|tw|twitter)?\s*(?<![\w@])@[\w.]+\s*\)?/gi;
// « (feat. X) » ou « [ft. X] » dans une étiquette d'artiste : devient « , X ».
const FEAT_PAREN = /[[(]\s*(?:feat\.?|ft\.?|featuring)\s+([^\])]*)[\])]/gi;
// Séparateurs : virgule (aussi pleine chasse « ， » et « 、 »), point-virgule, « feat. », « ft. », « featuring ».
const SEPARATORS = /\s*[,，;；、]\s*|\s+(?:feat\.?|ft\.?|featuring)\s+/i;

/** Clé de regroupement d'un nom : sans accents, sans casse, sans point ni espace final. */
export const artistKey = (name) => fold(name).replace(/[\s.]+$/, '').replace(/\s+/g, ' ').trim();

/** « COEURCO, Bi2l, COEURCO @1coeurco » → ['COEURCO', 'Bi2l'] (sans doublon, dans l'ordre). */
export function splitArtists(label) {
  const text = String(label || '').replace(HANDLE, '').replace(FEAT_PAREN, ', $1');
  const seen = new Set();
  const names = [];
  for (const raw of text.split(SEPARATORS)) {
    const name = raw.trim().replace(/\s+/g, ' ');
    const key = artistKey(name);
    if (!name || !key || seen.has(key)) continue;
    seen.add(key);
    names.push(name);
  }
  return names;
}

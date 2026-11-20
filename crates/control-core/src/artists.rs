//! Noms d'artistes : une étiquette « A, B feat. C » désigne plusieurs artistes. On la découpe, on retire les pseudos de
//! réseaux sociaux (« @nom ») et les doublons, pour que chaque artiste n'ait qu'une fiche.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

use crate::util::fold;

// « COEURCO @1coeurco », « Bi (t3rmina) - (@t3rminadoo) », « COEURCO ig @1coeurco » : le pseudo et son habillage.
// L'original exige que « @ » ne suive ni lettre ni « @ » (lookbehind) : vérifié à la main dans `strip_handles`.
static HANDLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s*[-–—]?\s*\(?\s*(?:ig|insta|instagram|tw|twitter)?\s*@[A-Za-z0-9_.]+\s*\)?").expect("regex")
});
// « (feat. X) » ou « [ft. X] » : devient « , X ».
static FEAT_PAREN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[\[(]\s*(?:feat\.?|ft\.?|featuring)\s+([^\])]*)[\])]").expect("regex"));
// Séparateurs : virgules (aussi « ， » et « 、 »), points-virgules, « feat. », « ft. », « featuring ».
static SEPARATORS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\s*[,，;；、]\s*|\s+(?:feat\.?|ft\.?|featuring)\s+").expect("regex"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("regex"));
static TRAILING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\s.]+$").expect("regex"));

/// Clé de regroupement d'un nom : sans accents, sans casse, sans point ni espace final.
pub fn artist_key(name: &str) -> String {
    let f = fold(name);
    let f = TRAILING.replace(&f, "");
    SPACES.replace_all(&f, " ").trim().to_string()
}

fn strip_handles(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for m in HANDLE.find_iter(text) {
        // Position du « @ » dans la correspondance : il ne doit suivre ni un caractère de mot ni un autre « @ ».
        let at = m.start() + m.as_str().find('@').unwrap_or(0);
        let prev = text[..at].chars().next_back();
        if prev.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '@') {
            continue;
        }
        out.push_str(&text[last..m.start()]);
        last = m.end();
    }
    out.push_str(&text[last..]);
    out
}

/// « COEURCO, Bi2l, COEURCO @1coeurco » → `["COEURCO", "Bi2l"]` (sans doublon, dans l'ordre).
pub fn split_artists(label: &str) -> Vec<String> {
    let text = strip_handles(label);
    let text = FEAT_PAREN.replace_all(&text, ", $1");
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for raw in SEPARATORS.split(&text) {
        let name = SPACES.replace_all(raw.trim(), " ").to_string();
        let key = artist_key(&name);
        if name.is_empty() || key.is_empty() || !seen.insert(key) {
            continue;
        }
        names.push(name);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn splits_and_dedupes() {
        assert_eq!(split_artists("COEURCO, Bi2l, COEURCO @1coeurco"), s(&["COEURCO", "Bi2l"]));
        assert_eq!(split_artists("A feat. B"), s(&["A", "B"]));
        assert_eq!(split_artists("A ft B"), s(&["A", "B"]));
        assert_eq!(split_artists("A (feat. B & C)"), s(&["A", "B & C"]));
        assert_eq!(split_artists("A [ft. B]"), s(&["A", "B"]));
        assert_eq!(split_artists("A；B、C，D"), s(&["A", "B", "C", "D"]));
        assert_eq!(split_artists("Bi (t3rmina) - (@t3rminadoo)"), s(&["Bi (t3rmina)"]));
        assert_eq!(split_artists("COEURCO ig @1coeurco"), s(&["COEURCO"]));
        assert_eq!(split_artists("Beyoncé, BEYONCE"), s(&["Beyoncé"]));
        assert_eq!(split_artists(""), Vec::<String>::new());
    }

    #[test]
    fn emails_are_not_handles() {
        assert_eq!(split_artists("mail me@x.com"), s(&["mail me@x.com"]));
    }

    #[test]
    fn keys_fold() {
        assert_eq!(artist_key("  Mr.  Oizo. "), "mr. oizo");
        assert_eq!(artist_key("Björk"), "bjork");
    }
}

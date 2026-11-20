//! Motifs façon mp3tag (`%artist% - %title%`) : composer un nom de fichier, relire des tags depuis un nom, et
//! quelques transformations de texte (casse, nettoyage, remplacement). Fonctions pures, sans accès aux fichiers :
//! l'éditeur de tags prépare ses modifications avec elles, puis les écrit avec [`crate::tags`].

use std::sync::LazyLock;

use regex::{Captures, Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::tags::TagFields;

/// Noms des marqueurs (`%nom%`) et champ qu'ils désignent (noms JSON de [`TagFields`]).
pub const PLACEHOLDERS: &[(&str, &str)] = &[
    ("title", "title"),
    ("artist", "artist"),
    ("album", "album"),
    ("albumartist", "albumArtist"),
    ("genre", "genre"),
    ("composer", "composer"),
    ("year", "year"),
    ("track", "trackNo"),
    ("tracktotal", "trackTotal"),
    ("disc", "discNo"),
    ("disctotal", "discTotal"),
];

/// Motifs proposés dans le menu de l'éditeur.
pub const PATTERN_PRESETS: &[&str] = &[
    "%track% - %title%",
    "%artist% - %title%",
    "%track% - %artist% - %title%",
    "%track%. %title%",
    "%disc%-%track% %title%",
    "%artist% - %album% - %track% - %title%",
];

static TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)%([a-z]+)%").expect("regex constante"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("regex constante"));
static SEPARATORS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:\s*[-–—._]\s*){2,}").expect("regex constante"));
static EDGES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\s\-–—._]+|[\s\-–—._]+$").expect("regex constante"));

fn placeholder(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    PLACEHOLDERS.iter().find(|(n, _)| *n == lower).map(|(_, key)| *key)
}

fn is_numeric(key: &str) -> bool {
    matches!(key, "year" | "trackNo" | "trackTotal" | "discNo" | "discTotal")
}

/// Texte d'un champ tel qu'il apparaît dans un nom de fichier (pistes sur au moins 2 chiffres, rien pour zéro).
fn field_text(fields: &TagFields, key: &str, name: &str) -> String {
    let number = |n: u32| -> String {
        if n == 0 {
            String::new()
        } else if name == "track" || name == "tracktotal" {
            format!("{n:02}")
        } else {
            n.to_string()
        }
    };
    match key {
        "title" => fields.title.clone(),
        "artist" => fields.artist.clone(),
        "album" => fields.album.clone(),
        "albumArtist" => fields.album_artist.clone(),
        "genre" => fields.genre.clone(),
        "composer" => fields.composer.clone(),
        "year" => number(fields.year),
        "trackNo" => number(fields.track_no),
        "trackTotal" => number(fields.track_total),
        "discNo" => number(fields.disc_no),
        "discTotal" => number(fields.disc_total),
        _ => String::new(),
    }
}

/// Compose un nom (sans extension) à partir d'un motif et des champs d'un morceau. Les séparateurs orphelins
/// (champ vide entre deux tirets) sont fusionnés, et ceux des bords retirés.
pub fn render_pattern(pattern: &str, fields: &TagFields) -> String {
    let raw = TOKEN.replace_all(pattern, |c: &Captures<'_>| {
        let name = c[1].to_lowercase();
        match placeholder(&name) {
            Some(key) => field_text(fields, key, &name),
            None => c[0].to_string(),
        }
    });
    let raw = SPACES.replace_all(&raw, " ");
    let raw = SEPARATORS.replace_all(&raw, " - ");
    EDGES.replace_all(&raw, "").trim().to_string()
}

/// Motif prêt à relire des noms de fichiers.
#[derive(Debug, Clone)]
pub struct CompiledPattern {
    regex: Regex,
    keys: Vec<&'static str>,
}

/// Échappe un morceau littéral du motif ; les espaces deviennent « des espaces ou rien ».
fn literal(text: &str) -> String {
    regex::escape(text).replace(' ', r"\s*")
}

/// Transforme un motif en expression régulière (`None` si le motif ne contient aucun marqueur connu).
pub fn compile_pattern(pattern: &str) -> Option<CompiledPattern> {
    let mut keys = Vec::new();
    let mut source = String::new();
    let mut last = 0;
    for m in TOKEN.captures_iter(pattern) {
        let whole = m.get(0).expect("groupe 0 toujours présent");
        let Some(key) = placeholder(&m[1]) else { continue };
        source.push_str(&literal(&pattern[last..whole.start()]));
        source.push_str(if is_numeric(key) { r"(\d+)" } else { "(.+?)" });
        keys.push(key);
        last = whole.end();
    }
    if keys.is_empty() {
        return None;
    }
    source.push_str(&literal(&pattern[last..]));
    let regex = RegexBuilder::new(&format!(r"^\s*{source}\s*$")).case_insensitive(true).build().ok()?;
    Some(CompiledPattern { regex, keys })
}

/// Champs relus dans un nom de fichier : seuls ceux du motif sont présents.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialFields {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album_artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub composer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_no: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_total: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disc_no: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disc_total: Option<u32>,
}

impl PartialFields {
    fn put(&mut self, key: &str, value: &str) {
        // Un nombre trop grand pour un u32 reste « très grand » : l'écriture le bornera de toute façon.
        let number = || Some(value.parse::<u32>().unwrap_or(u32::MAX));
        let text = || Some(value.to_string());
        match key {
            "title" => self.title = text(),
            "artist" => self.artist = text(),
            "album" => self.album = text(),
            "albumArtist" => self.album_artist = text(),
            "genre" => self.genre = text(),
            "composer" => self.composer = text(),
            "year" => self.year = number(),
            "trackNo" => self.track_no = number(),
            "trackTotal" => self.track_total = number(),
            "discNo" => self.disc_no = number(),
            "discTotal" => self.disc_total = number(),
            _ => {}
        }
    }
}

impl CompiledPattern {
    /// Champs lus dans un nom de fichier (sans extension), ou `None` s'il ne correspond pas au motif.
    pub fn parse(&self, basename: &str) -> Option<PartialFields> {
        let hit = self.regex.captures(basename)?;
        let mut out = PartialFields::default();
        for (i, key) in self.keys.iter().enumerate() {
            let value = hit.get(i + 1).map(|m| m.as_str().trim()).unwrap_or_default();
            out.put(key, value);
        }
        Some(out)
    }

    /// Champs du motif, dans l'ordre (noms JSON).
    pub fn keys(&self) -> &[&'static str] {
        &self.keys
    }
}

/// Raccourci : compile le motif et lit un nom (`None` si le motif est vide ou si le nom ne correspond pas).
pub fn parse_pattern(pattern: &str, basename: &str) -> Option<PartialFields> {
    compile_pattern(pattern)?.parse(basename)
}

// ─── Numérotation ────────────────────────────────────────────────────────────

/// Numéro de piste proposé par l'outil « Numéroter ».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Numbering {
    pub track_no: u32,
    /// Nombre de pistes, si demandé : le dernier numéro attribué.
    pub track_total: Option<u32>,
}

/// Numérote `count` morceaux à partir de `start` (borné à 0..=999, comme le champ de l'interface).
pub fn numbering(count: usize, start: u32, with_total: bool) -> Vec<Numbering> {
    let start = start.min(999);
    let last = (start as usize + count).saturating_sub(1) as u32;
    (0..count).map(|i| Numbering { track_no: start + i as u32, track_total: with_total.then_some(last) }).collect()
}

/// Pour chaque nom, `true` si un autre nom de la liste est identique à la casse près (renommage en double).
pub fn duplicate_names(names: &[String]) -> Vec<bool> {
    let lower: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
    lower.iter().map(|n| !n.is_empty() && lower.iter().filter(|o| *o == n).count() > 1).collect()
}

// ─── Transformations de texte ────────────────────────────────────────────────

/// Petits mots laissés en minuscules au milieu d'un titre (anglais et français).
const SMALL: &[&str] = &[
    "a", "an", "and", "as", "at", "but", "by", "for", "in", "of", "on", "or", "the", "to", "vs", "de", "du", "des",
    "la", "le", "les", "un", "une", "et", "ou", "à", "au", "aux", "en", "sur", "sous",
];

static WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(^|[\s(\[{"'«\-–—/])(\p{L}[\p{L}\p{M}'’]*)"#).expect("regex constante"));

fn upper_first(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Changements de casse proposés par l'outil « Casse ».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaseMode {
    /// Chaque Mot En Capitale, sauf les petits mots au milieu.
    Title,
    /// Première lettre en capitale.
    Sentence,
    Lower,
    Upper,
}

pub fn change_case(s: &str, mode: CaseMode) -> String {
    match mode {
        CaseMode::Title => {
            let lower = s.to_lowercase();
            WORD.replace_all(&lower, |c: &Captures<'_>| {
                let lead = &c[1];
                let word = &c[2];
                let first = c.get(0).is_some_and(|m| m.start() == 0) && lead.is_empty();
                if !first && SMALL.contains(&word) {
                    format!("{lead}{word}")
                } else {
                    format!("{lead}{}", upper_first(word))
                }
            })
            .into_owned()
        }
        CaseMode::Sentence => upper_first(&s.to_lowercase()),
        CaseMode::Lower => s.to_lowercase(),
        CaseMode::Upper => s.to_uppercase(),
    }
}

static NOISE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\s*[(\[{]\s*(?:official\s*(?:music\s*)?(?:video|audio|lyric\s*video)|lyrics?|lyric\s*video|audio|hd|hq|4k|clip\s*officiel|visuali[sz]er)\s*[)\]}]",
    )
    .expect("regex constante")
});

/// Retire les espaces en trop et les parasites de titres (« (Official Video) », « [Lyrics] »…).
pub fn tidy(s: &str) -> String {
    let s = NOISE.replace_all(s, "");
    SPACES.replace_all(&s, " ").trim().to_string()
}

/// Options de l'outil « Remplacer ».
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReplaceOptions {
    pub find: String,
    pub replace: String,
    /// `find` est une expression régulière, et `replace` peut citer ses groupes (`$1`, `$<nom>`, `$&`).
    pub regex: bool,
    pub match_case: bool,
}

/// Remplace toutes les occurrences. Une expression régulière invalide laisse le texte tel quel.
///
/// Les expressions sont celles du moteur `regex` (sans références arrière ni assertions avant/arrière) ; le texte
/// de remplacement suit la syntaxe de JavaScript, comme dans la version d'origine.
pub fn replace_text(value: &str, o: &ReplaceOptions) -> String {
    if o.find.is_empty() {
        return value.to_string();
    }
    if !o.regex {
        let Ok(re) = RegexBuilder::new(&regex::escape(&o.find)).case_insensitive(!o.match_case).build() else {
            return value.to_string();
        };
        return re.replace_all(value, regex::NoExpand(&o.replace)).into_owned();
    }
    let Ok(re) = RegexBuilder::new(&o.find).case_insensitive(!o.match_case).build() else {
        return value.to_string();
    };
    // En JavaScript, `$<x>` n'est remplacé que si l'expression a des groupes nommés.
    let named = re.capture_names().any(|n| n.is_some());
    re.replace_all(value, |c: &Captures<'_>| expand_js(&o.replace, c, value, named)).into_owned()
}

/// Remplacement à la manière de `String.prototype.replace` : `$$`, `$&`, `` $` ``, `$'`, `$1`…`$99`, `$<nom>`.
fn expand_js(template: &str, c: &Captures<'_>, haystack: &str, named: bool) -> String {
    let whole = c.get(0).expect("groupe 0 toujours présent");
    let groups = c.len() - 1;
    let bytes = template.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    let mut plain = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' || i + 1 >= bytes.len() {
            i += 1;
            continue;
        }
        let next = bytes[i + 1];
        let (text, used): (Option<&str>, usize) = match next {
            b'$' => (Some("$"), 2),
            b'&' => (Some(whole.as_str()), 2),
            b'`' => (Some(&haystack[..whole.start()]), 2),
            b'\'' => (Some(&haystack[whole.end()..]), 2),
            b'0'..=b'9' => {
                let one = (next - b'0') as usize;
                let two = bytes.get(i + 2).filter(|b| b.is_ascii_digit()).map(|b| one * 10 + (b - b'0') as usize);
                match two {
                    Some(n) if n >= 1 && n <= groups => (Some(c.get(n).map_or("", |m| m.as_str())), 3),
                    _ if one >= 1 && one <= groups => (Some(c.get(one).map_or("", |m| m.as_str())), 2),
                    _ => (None, 0),
                }
            }
            b'<' if named => match template[i + 2..].find('>') {
                Some(end) => {
                    let name = &template[i + 2..i + 2 + end];
                    (Some(c.name(name).map_or("", |m| m.as_str())), end + 3)
                }
                _ => (None, 0),
            },
            _ => (None, 0),
        };
        match text {
            Some(t) => {
                out.push_str(&template[plain..i]);
                out.push_str(t);
                i += used;
                plain = i;
            }
            None => i += 1,
        }
    }
    out.push_str(&template[plain..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> TagFields {
        TagFields {
            title: "Song".into(),
            artist: "Band".into(),
            album: "Record".into(),
            year: 2001,
            track_no: 3,
            track_total: 12,
            ..TagFields::default()
        }
    }

    #[test]
    fn renders_patterns() {
        assert_eq!(render_pattern("%track% - %title%", &fields()), "03 - Song");
        assert_eq!(render_pattern("%ARTIST% - %album% - %title%", &fields()), "Band - Record - Song");
        // Champ vide : séparateurs fusionnés, bords nettoyés.
        assert_eq!(render_pattern("%artist% - %genre% - %title%", &fields()), "Band - Song");
        assert_eq!(render_pattern("%disc%-%track% %title%", &fields()), "03 Song");
        assert_eq!(render_pattern("%genre% - %title%", &fields()), "Song");
        // Marqueur inconnu laissé tel quel.
        assert_eq!(render_pattern("%foo% %title%", &fields()), "%foo% Song");
        assert_eq!(render_pattern("%tracktotal%", &fields()), "12");
    }

    #[test]
    fn parses_file_names() {
        let p = compile_pattern("%track% - %artist% - %title%").unwrap();
        assert_eq!(p.keys(), ["trackNo", "artist", "title"]);
        let got = p.parse("07 - Daft Punk - One More Time").unwrap();
        assert_eq!(got.track_no, Some(7));
        assert_eq!(got.artist.as_deref(), Some("Daft Punk"));
        assert_eq!(got.title.as_deref(), Some("One More Time"));
        // Espaces facultatifs autour des séparateurs.
        let got = p.parse("07-Daft Punk-One").unwrap();
        assert_eq!(got.artist.as_deref(), Some("Daft Punk"));
        assert!(p.parse("Daft Punk - One").is_none());
        assert!(compile_pattern("plain text").is_none());
        assert!(compile_pattern("%nope%").is_none());
        // Caractères spéciaux du motif pris littéralement.
        let got = parse_pattern("%track%. %title% (%year%)", "01. Intro (1999)").unwrap();
        assert_eq!((got.track_no, got.title.as_deref(), got.year), (Some(1), Some("Intro"), Some(1999)));
        let json = serde_json::to_value(&got).unwrap();
        assert_eq!(json, serde_json::json!({ "title": "Intro", "year": 1999, "trackNo": 1 }));
    }

    #[test]
    fn numbers_tracks() {
        let n = numbering(3, 5, true);
        assert_eq!(n.iter().map(|x| x.track_no).collect::<Vec<_>>(), [5, 6, 7]);
        assert!(n.iter().all(|x| x.track_total == Some(7)));
        assert_eq!(numbering(2, 1, false)[1], Numbering { track_no: 2, track_total: None });
        assert!(numbering(0, 1, true).is_empty());
        assert_eq!(duplicate_names(&["A".into(), "b".into(), "a".into(), String::new()]), [true, false, true, false]);
    }

    #[test]
    fn changes_case() {
        assert_eq!(change_case("the lord OF the rings", CaseMode::Title), "The Lord of the Rings");
        assert_eq!(change_case("la vie en rose", CaseMode::Title), "La Vie en Rose");
        assert_eq!(change_case("rock-and-roll (live at the bbc)", CaseMode::Title), "Rock-and-Roll (Live at the Bbc)");
        assert_eq!(change_case("l'été indien", CaseMode::Title), "L'été Indien");
        assert_eq!(change_case("HELLO world", CaseMode::Sentence), "Hello world");
        assert_eq!(change_case("ÉtÉ", CaseMode::Lower), "été");
        assert_eq!(change_case("straße", CaseMode::Upper), "STRASSE");
    }

    #[test]
    fn tidies_titles() {
        assert_eq!(tidy("Song (Official Video)"), "Song");
        assert_eq!(tidy("Song [Official Music Video]  "), "Song");
        assert_eq!(tidy("Song  (Lyrics) [HD]"), "Song");
        assert_eq!(tidy("Song {clip officiel}"), "Song");
        assert_eq!(tidy("Song (Live)"), "Song (Live)");
        assert_eq!(tidy("  A   B "), "A B");
    }

    #[test]
    fn replaces_text() {
        let o = |find: &str, replace: &str, regex: bool, match_case: bool| ReplaceOptions {
            find: find.into(),
            replace: replace.into(),
            regex,
            match_case,
        };
        assert_eq!(replace_text("Feat. X feat. Y", &o("feat.", "ft.", false, false)), "ft. X ft. Y");
        assert_eq!(replace_text("Feat. X feat. Y", &o("feat.", "ft.", false, true)), "Feat. X ft. Y");
        // Littéral : ni métacaractères ni `$` interprétés.
        assert_eq!(replace_text("a.b", &o(".", "$1", false, false)), "a$1b");
        assert_eq!(replace_text("Artist - Title", &o(r"(.+) - (.+)", "$2 - $1", true, false)), "Title - Artist");
        assert_eq!(replace_text("abc", &o("b", "[$&]", true, false)), "a[b]c");
        assert_eq!(replace_text("abc", &o("b", "$$", true, false)), "a$c");
        assert_eq!(replace_text("abc", &o("b", "<$`|$'>", true, false)), "a<a|c>c");
        // `$3` sans troisième groupe : laissé tel quel ; `$1x` : groupe 1 puis « x ».
        assert_eq!(replace_text("ab", &o("(a)", "$3", true, false)), "$3b");
        assert_eq!(replace_text("ab", &o("(a)", "$1x", true, false)), "axb");
        assert_eq!(replace_text("ab", &o("(?<n>a)", "<$<n>>", true, false)), "<a>b");
        assert_eq!(replace_text("ab", &o("(a)", "$<n>", true, false)), "$<n>b");
        // Expression invalide : rien ne change.
        assert_eq!(replace_text("abc", &o("(", "x", true, false)), "abc");
        assert_eq!(replace_text("abc", &o("", "x", true, false)), "abc");
        assert_eq!(replace_text("ÉTÉ", &o("été", "x", false, false)), "x");
    }
}

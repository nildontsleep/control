//! Petits outils partagés : repli des accents, horloge, comparaison « naturelle », écriture atomique.

use std::cmp::Ordering;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Millisecondes depuis l'époque Unix (comme `Date.now()`).
pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Texte sans accents ni casse, pour la recherche et les regroupements (`fold` de l'interface d'origine).
pub fn fold(s: &str) -> String {
    s.nfd().filter(|c| !is_combining_mark(*c)).flat_map(char::to_lowercase).collect()
}

/// Comparaison insensible à la casse et aux accents, avec les nombres dans l'ordre naturel
/// (« Piste 2 » avant « Piste 10 »), comme `Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (fa, fb) = (fold(a), fold(b));
    let mut ia = fa.chars().peekable();
    let mut ib = fb.chars().peekable();
    loop {
        match (ia.peek().copied(), ib.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let na = take_number(&mut ia);
                let nb = take_number(&mut ib);
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(ca), Some(cb)) => {
                let (ka, kb) = (sort_key(ca), sort_key(cb));
                if ka != kb {
                    return ka.cmp(&kb);
                }
                ia.next();
                ib.next();
            }
        }
    }
}

fn take_number(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut s = String::new();
    while let Some(c) = it.peek().copied() {
        if !c.is_ascii_digit() {
            break;
        }
        s.push(c);
        it.next();
    }
    s
}

/// Ponctuation et espaces avant les chiffres, eux-mêmes avant les lettres (ordre d'ICU).
fn sort_key(c: char) -> (u8, char) {
    if c.is_alphabetic() {
        (2, c)
    } else if c.is_numeric() {
        (1, c)
    } else {
        (0, c)
    }
}

/// Écrit un fichier en passant par un fichier temporaire renommé : jamais de fichier à moitié écrit.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// Clé de comparaison d'un chemin : insensible à la casse sous Windows seulement.
pub fn path_key(p: &str) -> String {
    if cfg!(windows) { p.to_lowercase() } else { p.to_string() }
}

/// Coupe une chaîne à `max` caractères (pas octets).
pub fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Date locale « AAAA-MM-JJ » d'un instant en millisecondes.
pub fn local_day(ms: i64) -> String {
    let dt = local_datetime(ms);
    format!("{:04}-{:02}-{:02}", dt.year(), u8::from(dt.month()), dt.day())
}

/// Instant en millisecondes vu dans le fuseau local (UTC si le fuseau est inconnu).
pub fn local_datetime(ms: i64) -> time::OffsetDateTime {
    let utc = time::OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    match time::UtcOffset::local_offset_at(utc) {
        Ok(off) => utc.to_offset(off),
        Err(_) => utc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_strips_accents_and_case() {
        assert_eq!(fold("Éléonore À"), "eleonore a");
        assert_eq!(fold("Beyoncé"), "beyonce");
    }

    #[test]
    fn natural_order() {
        assert_eq!(natural_cmp("Piste 2", "Piste 10"), Ordering::Less);
        assert_eq!(natural_cmp("abc", "ABC"), Ordering::Equal);
        assert_eq!(natural_cmp("été", "ete"), Ordering::Equal);
        assert_eq!(natural_cmp("a", "b"), Ordering::Less);
        assert_eq!(natural_cmp("a01", "a1"), Ordering::Equal);
        assert_eq!(natural_cmp("", "a"), Ordering::Less);
    }

    #[test]
    fn atomic_write_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x/y.json");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
    }
}

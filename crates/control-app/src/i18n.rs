//! Textes de l'interface : français par défaut, anglais en option. Les dictionnaires sont ceux de la version
//! d'origine (`i18n/fr.json`, `i18n/en.json`), avec les mêmes clés et les mêmes formes du pluriel.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

type Dict = HashMap<String, String>;

static FR: LazyLock<Dict> = LazyLock::new(|| serde_json::from_str(include_str!("../i18n/fr.json")).unwrap_or_default());
static EN: LazyLock<Dict> = LazyLock::new(|| serde_json::from_str(include_str!("../i18n/en.json")).unwrap_or_default());

static ENGLISH: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Fr,
    En,
}

/// Langue d'un réglage (`fr`, `en` ou `auto`, qui suit la langue du système).
pub fn resolve(pref: &str) -> Lang {
    match pref {
        "fr" => Lang::Fr,
        "en" => Lang::En,
        _ => {
            let sys = std::env::var("LC_ALL")
                .or_else(|_| std::env::var("LC_MESSAGES"))
                .or_else(|_| std::env::var("LANG"))
                .unwrap_or_default()
                .to_lowercase();
            if sys.is_empty() || sys.starts_with("fr") || sys == "c" || sys == "posix" { Lang::Fr } else { Lang::En }
        }
    }
}

pub fn set_language(pref: &str) -> Lang {
    let lang = resolve(pref);
    ENGLISH.store(lang == Lang::En, Ordering::Relaxed);
    lang
}

pub fn current() -> Lang {
    if ENGLISH.load(Ordering::Relaxed) { Lang::En } else { Lang::Fr }
}

fn dict() -> &'static Dict {
    if current() == Lang::En { &EN } else { &FR }
}

/// Forme plurielle (règles CLDR du français et de l'anglais).
fn plural_form(n: f64) -> &'static str {
    match current() {
        Lang::Fr => {
            if (0.0..2.0).contains(&n) {
                "one"
            } else if n != 0.0 && n.fract() == 0.0 && (n as i64) % 1_000_000 == 0 {
                "many"
            } else {
                "other"
            }
        }
        Lang::En => {
            if n == 1.0 {
                "one"
            } else {
                "other"
            }
        }
    }
}

fn lookup(key: &str) -> String {
    dict().get(key).or_else(|| FR.get(key)).cloned().unwrap_or_else(|| key.to_string())
}

/// Texte d'une clé.
pub fn t(key: &str) -> String {
    lookup(key)
}

/// Texte d'une clé avec des variables `{nom}` ; `n` choisit aussi la forme du pluriel.
pub fn tv(key: &str, vars: &[(&str, &dyn std::fmt::Display)]) -> String {
    let n = vars.iter().find(|(k, _)| *k == "n").and_then(|(_, v)| v.to_string().parse::<f64>().ok());
    let mut s = None;
    if let Some(n) = n {
        let form = plural_form(n);
        let d = dict();
        s = d
            .get(&format!("{key}_{form}"))
            .or_else(|| d.get(&format!("{key}_other")))
            .or_else(|| FR.get(&format!("{key}_{form}")))
            .or_else(|| FR.get(&format!("{key}_other")))
            .cloned();
    }
    let mut s = s.unwrap_or_else(|| lookup(key));
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), &v.to_string());
    }
    s
}

/// Raccourci pour un seul nombre : `tn("count.tracks", 3)` → « 3 morceaux ».
pub fn tn(key: &str, n: impl std::fmt::Display) -> String {
    tv(key, &[("n", &n)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionaries_load_and_plurals_work() {
        assert!(FR.len() > 1000 && EN.len() == FR.len());
        set_language("fr");
        assert_eq!(tn("count.tracks", 1), "1 morceau");
        assert_eq!(tn("count.tracks", 0), "0 morceau");
        assert_eq!(tn("count.tracks", 12), "12 morceaux");
        assert_eq!(t("nav.songs"), "Morceaux");
        assert_eq!(t("no.such.key"), "no.such.key");
        set_language("en");
        assert_eq!(tn("count.tracks", 0), "0 tracks");
        assert_eq!(tn("count.tracks", 1), "1 track");
        set_language("fr");
    }
}

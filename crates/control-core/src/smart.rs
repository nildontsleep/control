//! Playlists intelligentes : champs, opérateurs, modèles prêts à l'emploi et test d'une règle.

use std::cmp::Ordering;

use serde_json::{Value, json};

use crate::model::{SmartRule, SmartSort, SmartSpec, Track};
use crate::util::{fold, natural_cmp};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Number,
    Text,
    Date,
}

/// Champs utilisables dans une règle, dans l'ordre du menu.
pub const FIELDS: &[(&str, FieldType)] = &[
    ("playCount", FieldType::Number),
    ("lastPlayed", FieldType::Date),
    ("addedAt", FieldType::Date),
    ("genre", FieldType::Text),
    ("artist", FieldType::Text),
    ("album", FieldType::Text),
    ("title", FieldType::Text),
    ("year", FieldType::Number),
    ("duration", FieldType::Number),
];

/// Opérateurs par type de champ (le premier est celui par défaut).
pub fn ops(t: FieldType) -> &'static [&'static str] {
    match t {
        FieldType::Number => &["is", "isNot", "gt", "lt"],
        FieldType::Text => &["contains", "notContains", "is", "isNot"],
        FieldType::Date => &["within", "notWithin"],
    }
}

/// Clés de tri proposées.
pub const SORT_KEYS: &[&str] = &["title", "artist", "playCount", "addedAt", "lastPlayed", "year", "random"];

pub fn field_type(field: &str) -> Option<FieldType> {
    FIELDS.iter().find(|(f, _)| *f == field).map(|(_, t)| *t)
}

/// Valeur par défaut d'une règle quand on change de champ.
pub fn default_value(t: FieldType) -> Value {
    match t {
        FieldType::Text => json!(""),
        FieldType::Date => json!(30),
        FieldType::Number => json!(0),
    }
}

/// Modèles : (identifiant, règles).
pub fn presets() -> Vec<(&'static str, SmartSpec)> {
    let rule = |field: &str, op: &str, value: Value| SmartRule { field: field.into(), op: op.into(), value };
    let sort = |key: &str, dir: &str| SmartSort { key: key.into(), dir: dir.into() };
    let spec = |rules, sort, limit| SmartSpec { match_: "all".into(), rules, sort, limit };
    vec![
        ("mostPlayed", spec(vec![rule("playCount", "gt", json!(0))], sort("playCount", "desc"), 50)),
        ("recent", spec(vec![rule("addedAt", "within", json!(30))], sort("addedAt", "desc"), 0)),
        ("neverPlayed", spec(vec![rule("playCount", "is", json!(0))], sort("addedAt", "desc"), 0)),
        ("recentlyPlayed", spec(vec![rule("lastPlayed", "within", json!(14))], sort("lastPlayed", "desc"), 0)),
        ("genre", spec(vec![rule("genre", "contains", json!(""))], sort("title", "asc"), 0)),
    ]
}

/// Nombre lu comme `Number(x || 0)` en JavaScript (texte numérique accepté, le reste vaut 0).
fn number(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s.trim().parse::<f64>().unwrap_or(if s.trim().is_empty() { 0.0 } else { f64::NAN }),
        Value::Bool(b) => f64::from(u8::from(*b)),
        _ => 0.0,
    }
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

fn track_text<'a>(t: &'a Track, field: &str) -> &'a str {
    match field {
        "genre" => &t.genre,
        "artist" => &t.artist,
        "album" => &t.album,
        "title" => &t.title,
        _ => "",
    }
}

fn track_number(t: &Track, field: &str) -> f64 {
    match field {
        "playCount" => t.play_count as f64,
        "year" => t.year.unwrap_or(0) as f64,
        // La durée se compare en minutes.
        "duration" => t.duration / 60.0,
        _ => 0.0,
    }
}

fn track_date(t: &Track, field: &str) -> Option<i64> {
    match field {
        "lastPlayed" => t.last_played,
        "addedAt" => Some(t.added_at).filter(|v| *v != 0),
        _ => None,
    }
}

/// Le morceau satisfait-il la règle ? Un champ inconnu ne filtre rien.
pub fn test_rule(t: &Track, r: &SmartRule, now_ms: i64) -> bool {
    let Some(ft) = field_type(&r.field) else { return true };
    match ft {
        FieldType::Text => {
            let a = fold(track_text(t, &r.field));
            let b = fold(&text(&r.value));
            match r.op.as_str() {
                "contains" => a.contains(&b),
                "notContains" => !a.contains(&b),
                "is" => a == b,
                _ => a != b,
            }
        }
        FieldType::Date => {
            let days = number(&r.value);
            let within = track_date(t, &r.field).is_some_and(|d| ((now_ms - d) as f64) <= days * 86_400_000.0);
            if r.op == "within" { within } else { !within }
        }
        FieldType::Number => {
            let v = track_number(t, &r.field);
            let n = number(&r.value);
            match r.op.as_str() {
                "is" => v.floor() == n,
                "isNot" => v.floor() != n,
                "gt" => v > n,
                _ => v < n,
            }
        }
    }
}

/// Ordre d'une clé de tri de playlist intelligente (croissant).
pub fn compare_by(a: &Track, b: &Track, key: &str) -> Ordering {
    let num = |x: f64, y: f64| x.partial_cmp(&y).unwrap_or(Ordering::Equal);
    match key {
        "playCount" => a.play_count.cmp(&b.play_count),
        "addedAt" => a.added_at.cmp(&b.added_at),
        "lastPlayed" => a.last_played.unwrap_or(0).cmp(&b.last_played.unwrap_or(0)),
        "year" => a.year.unwrap_or(0).cmp(&b.year.unwrap_or(0)),
        "duration" => num(a.duration, b.duration),
        "artist" => natural_cmp(&a.artist, &b.artist),
        "album" => natural_cmp(&a.album, &b.album),
        "genre" => natural_cmp(&a.genre, &b.genre),
        _ => natural_cmp(&a.title, &b.title),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(play_count: i64, genre: &str, duration: f64, added_at: i64) -> Track {
        Track { play_count, genre: genre.into(), duration, added_at, title: "x".into(), ..Track::default() }
    }

    fn r(field: &str, op: &str, value: Value) -> SmartRule {
        SmartRule { field: field.into(), op: op.into(), value }
    }

    #[test]
    fn rules() {
        let now = 100 * 86_400_000;
        let a = t(3, "Électro, Pop", 185.0, now - 2 * 86_400_000);
        assert!(test_rule(&a, &r("genre", "contains", json!("electro")), now));
        assert!(!test_rule(&a, &r("genre", "notContains", json!("POP")), now));
        assert!(test_rule(&a, &r("playCount", "gt", json!(2)), now));
        assert!(test_rule(&a, &r("playCount", "is", json!("3")), now));
        assert!(test_rule(&a, &r("duration", "is", json!(3)), now), "3 min 05 → 3");
        assert!(test_rule(&a, &r("duration", "lt", json!(4)), now));
        assert!(test_rule(&a, &r("addedAt", "within", json!(7)), now));
        assert!(!test_rule(&a, &r("addedAt", "within", json!(1)), now));
        assert!(test_rule(&a, &r("lastPlayed", "notWithin", json!(30)), now), "jamais écouté");
        assert!(test_rule(&a, &r("nope", "is", json!(1)), now));
    }

    #[test]
    fn presets_are_valid() {
        for (_, spec) in presets() {
            for rule in &spec.rules {
                let ft = field_type(&rule.field).unwrap();
                assert!(ops(ft).contains(&rule.op.as_str()));
            }
            assert!(SORT_KEYS.contains(&spec.sort.key.as_str()));
        }
    }

    #[test]
    fn spec_json_roundtrip_matches_original_shape() {
        let (_, spec) = presets().remove(0);
        let v = serde_json::to_value(&spec).unwrap();
        assert_eq!(
            v,
            json!({"match": "all", "rules": [{"field": "playCount", "op": "gt", "value": 0}],
                   "sort": {"key": "playCount", "dir": "desc"}, "limit": 50})
        );
    }
}

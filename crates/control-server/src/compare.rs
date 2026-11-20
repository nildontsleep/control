//! Comparaison de deux récaps (le résultat de `stats::get` de chaque compte, sur la même période) : totaux côte à
//! côte, tops, goûts communs et un indice de ressemblance. Ne renvoie que des agrégats, jamais l'historique.

use std::collections::HashMap;

use serde_json::{Value, json};
use unicode_normalization::UnicodeNormalization;

/// Texte sans accents ni casse, espaces réduits.
pub fn fold(s: &str) -> String {
    let stripped: String = s.nfd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).collect();
    stripped.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Similarité cosinus de deux « sacs de comptes » : 0 (rien en commun) à 1 (mêmes proportions).
pub fn cosine(a: &HashMap<String, f64>, b: &HashMap<String, f64>) -> f64 {
    let na: f64 = a.values().map(|v| v * v).sum();
    let nb: f64 = b.values().map(|v| v * v).sum();
    let dot: f64 = a.iter().filter_map(|(k, v)| b.get(k).map(|w| v * w)).sum();
    if na > 0.0 && nb > 0.0 { dot / (na * nb).sqrt() } else { 0.0 }
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn n(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn list(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn bag(rows: &[Value], key: impl Fn(&Value) -> String) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    for r in rows {
        let k = key(r);
        if !k.is_empty() {
            *out.entry(k).or_insert(0.0) += n(&r["plays"]);
        }
    }
    out
}

fn round(v: &Value) -> i64 {
    n(v).round() as i64
}

/// Ce qu'on garde d'un récap pour l'affichage.
pub fn summary(stats: &Value, top: usize) -> Value {
    let m = &stats["music"];
    let take = |v: &Value, k: usize| list(v).iter().take(k).cloned().collect::<Vec<_>>();
    json!({
        "range": stats["range"],
        "plays": n(&m["plays"]) as i64,
        "seconds": round(&m["seconds"]),
        "tracks": n(&m["tracks"]) as i64,
        "artists": n(&m["artists"]) as i64,
        "albums": n(&m["albums"]) as i64,
        "completed": n(&m["completed"]) as i64,
        "streak": if stats["streak"].is_object() { stats["streak"].clone() } else { json!({ "current": 0, "longest": 0 }) },
        "video": {
            "seconds": round(&stats["video"]["seconds"]),
            "titles": n(&stats["video"]["titles"]) as i64,
            "finished": n(&stats["video"]["finished"]) as i64,
        },
        "topArtists": take(&m["topArtists"], top).iter().map(|a| json!({
            "name": a["name"], "plays": a["plays"], "seconds": round(&a["seconds"]), "cover": a["cover"],
        })).collect::<Vec<_>>(),
        "topTracks": take(&m["topTracks"], top).iter().map(|t| json!({
            "title": t["title"], "artist": t["artist"], "album": t["album"], "cover": t["cover"], "plays": t["plays"],
        })).collect::<Vec<_>>(),
        "topAlbums": take(&m["topAlbums"], 6).iter().map(|a| json!({
            "album": a["album"], "artist": a["artist"], "cover": a["cover"], "plays": a["plays"],
        })).collect::<Vec<_>>(),
        "genres": take(&m["genres"], 6).iter().map(|g| json!({
            "name": g["name"], "plays": g["plays"], "seconds": round(&g["seconds"]),
        })).collect::<Vec<_>>(),
        "byWeekday": if m["byWeekday"].is_array() { m["byWeekday"].clone() } else { json!([0, 0, 0, 0, 0, 0, 0]) },
        "byHour": if m["byHour"].is_array() { m["byHour"].clone() } else { json!(vec![0; 24]) },
    })
}

/// `a`, `b` : récaps (idéalement avec un `limit` large). Renvoie `{ me, other, shared, score }` (score de 0 à 100).
pub fn compare(a: &Value, b: &Value) -> Value {
    let (ma, mb) = (&a["music"], &b["music"]);
    let artist_key = |r: &Value| fold(&s(&r["name"]));
    let track_key = |r: &Value| {
        let title = s(&r["title"]);
        if title.is_empty() { String::new() } else { format!("{}|{}", fold(&s(&r["artist"])), fold(&title)) }
    };
    let (artists_a, artists_b) = (bag(list(&ma["topArtists"]), artist_key), bag(list(&mb["topArtists"]), artist_key));
    let (tracks_a, tracks_b) = (bag(list(&ma["topTracks"]), track_key), bag(list(&mb["topTracks"]), track_key));
    let (genres_a, genres_b) = (bag(list(&ma["genres"]), artist_key), bag(list(&mb["genres"]), artist_key));

    let shared = |rows: &[Value],
                  other: &HashMap<String, f64>,
                  key: &dyn Fn(&Value) -> String,
                  limit: usize,
                  f: &dyn Fn(&Value, f64) -> Value| {
        let mut out: Vec<(f64, Value)> = rows
            .iter()
            .filter_map(|r| other.get(&key(r)).map(|theirs| (n(&r["plays"]) + theirs, f(r, *theirs))))
            .collect();
        out.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(std::cmp::Ordering::Equal));
        out.into_iter().take(limit).map(|(_, v)| v).collect::<Vec<_>>()
    };
    let shared_artists = shared(
        list(&ma["topArtists"]),
        &artists_b,
        &artist_key,
        12,
        &|r, theirs| json!({ "name": r["name"], "cover": r["cover"], "mine": r["plays"], "theirs": theirs }),
    );
    let shared_tracks = shared(
        list(&ma["topTracks"]),
        &tracks_b,
        &track_key,
        8,
        &|r, theirs| json!({ "title": r["title"], "artist": r["artist"], "cover": r["cover"], "mine": r["plays"], "theirs": theirs }),
    );
    let shared_genres = shared(
        list(&ma["genres"]),
        &genres_b,
        &artist_key,
        6,
        &|r, theirs| json!({ "name": r["name"], "mine": r["plays"], "theirs": theirs }),
    );

    // Pas d'écoute d'un côté : rien à comparer, l'indice reste à 0.
    let enough = n(&ma["plays"]) > 0.0 && n(&mb["plays"]) > 0.0;
    let score = if enough {
        (100.0
            * (0.55 * cosine(&artists_a, &artists_b)
                + 0.3 * cosine(&genres_a, &genres_b)
                + 0.15 * cosine(&tracks_a, &tracks_b)))
        .round() as i64
    } else {
        0
    };
    json!({
        "me": summary(a, 10),
        "other": summary(b, 10),
        "shared": { "artists": shared_artists, "tracks": shared_tracks, "genres": shared_genres },
        "score": score,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(artists: Value, tracks: Value, genres: Value, plays: i64) -> Value {
        json!({
            "range": { "from": "2026-01-01", "to": "2026-12-31" }, "years": [2026], "streak": { "current": 1, "longest": 3 },
            "music": { "plays": plays, "seconds": plays * 200, "tracks": 1, "artists": 1, "albums": 1, "completed": plays,
                "topArtists": artists, "topTracks": tracks, "topAlbums": [], "genres": genres,
                "byWeekday": [0, 0, 0, 0, 0, 0, 0], "byHour": vec![0; 24] },
            "video": { "seconds": 0, "titles": 0, "finished": 0 },
        })
    }

    fn artist(name: &str, plays: i64) -> Value {
        json!({ "name": name, "plays": plays, "seconds": plays * 200, "cover": null })
    }

    fn track(title: &str, artist: &str, plays: i64) -> Value {
        json!({ "title": title, "artist": artist, "album": "", "cover": null, "plays": plays })
    }

    #[test]
    fn shared_tastes_and_score() {
        let a = stats(
            json!([artist("Daft Punk", 10), artist("Björk", 4)]),
            json!([track("One More Time", "Daft Punk", 6)]),
            json!([{ "name": "Électro", "plays": 10, "seconds": 1 }]),
            14,
        );
        let b = stats(
            json!([artist("daft punk", 8), artist("Mono", 2)]),
            json!([track("one more time", "DAFT PUNK", 2)]),
            json!([{ "name": "electro", "plays": 8, "seconds": 1 }]),
            10,
        );
        let same = compare(&a, &b);
        assert_eq!(same["shared"]["artists"][0]["name"], "Daft Punk");
        assert_eq!(same["shared"]["tracks"].as_array().unwrap().len(), 1);
        assert_eq!(same["shared"]["genres"].as_array().unwrap().len(), 1);
        let score = same["score"].as_i64().unwrap();
        assert!(score > 40 && score < 100, "score {score}");

        let mono = stats(
            json!([artist("Mono", 5)]),
            json!([track("Ghost", "Mono", 5)]),
            json!([{ "name": "Classique", "plays": 5, "seconds": 1 }]),
            5,
        );
        let alone = compare(&a, &mono);
        assert_eq!(alone["shared"]["artists"], json!([]));
        assert_eq!(alone["score"], 0);
        assert_eq!(compare(&a, &stats(json!([]), json!([]), json!([]), 0))["score"], 0);
        assert_eq!(compare(&a, &a)["score"], 100);
    }
}

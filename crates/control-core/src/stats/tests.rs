use super::*;
use crate::util::now_ms;

fn db() -> Connection {
    let db = crate::db::open_in_memory().unwrap();
    for (i, (title, artist, album, genre, kind)) in [
        ("A", "Alpha", "One", "Rock, Pop", "audio"),
        ("B", "Alpha", "One", "Pop", "audio"),
        ("C", "Beta", "", "", "audio"),
        ("V", "", "", "", "video"),
    ]
    .iter()
    .enumerate()
    {
        db.execute(
            "INSERT INTO tracks (id, path, kind, title, artist, album, genre, added_at, duration) VALUES (?, ?, ?, ?, ?, ?, ?, 0, 200)",
            params![i as i64 + 1, format!("/m/{title}"), kind, title, artist, album, genre],
        )
        .unwrap();
    }
    db
}

fn play(db: &Connection, track: i64, at: i64, seconds: f64) {
    db.execute(
        "INSERT INTO plays (track_id, played_at, seconds, duration, completed) VALUES (?, ?, ?, 200, 1)",
        params![track, at, seconds],
    )
    .unwrap();
}

#[test]
fn totals_tops_and_genres() {
    let db = db();
    let now = now_ms();
    play(&db, 1, now - 1000, 100.0);
    play(&db, 1, now - 2000, 0.0); // compte 30 s
    play(&db, 2, now - 3000, 50.0);
    play(&db, 3, now - 4000, 10.0);
    play(&db, 4, now - 5000, 60.0);
    let s = get(&db, "", "", 10, now).unwrap();
    assert_eq!(s.music.plays, 4);
    assert!((s.music.seconds - 190.0).abs() < 1e-9);
    assert_eq!(s.music.tracks, 3);
    assert_eq!(s.music.artists, 2);
    assert_eq!(s.music.albums, 1);
    assert_eq!(s.music.top_tracks[0].id, 1);
    assert_eq!(s.music.top_artists[0].name, "Alpha");
    assert_eq!(s.music.top_artists[0].plays, 3);
    assert_eq!(s.music.genres[0].name, "Pop");
    assert_eq!(s.music.genres[0].plays, 3);
    assert_eq!(s.video.titles, 1);
    assert!((s.video.seconds - 60.0).abs() < 1e-9);
    assert_eq!(s.video.top[0].kind, "local");
    assert_eq!(s.music.by_hour.iter().sum::<f64>(), 190.0);
    assert_eq!(s.streak.current, 1);
    assert_eq!(s.days.len(), 1);
}

#[test]
fn range_excludes_outside_plays() {
    let db = db();
    let now = now_ms();
    play(&db, 1, now - 40 * DAY_MS, 100.0);
    let from = local_day(now - 7 * DAY_MS);
    let s = get(&db, &from, "", 10, now).unwrap();
    assert_eq!(s.music.plays, 0);
    assert_eq!(s.range.from, from);
    assert_eq!(s.range.to, local_day(now));
    assert_eq!(s.years.len(), 1, "les années couvrent tout l'historique");
}

#[test]
fn streak_counting() {
    let now = now_ms();
    let d = |n: i64| local_day(now - n * DAY_MS);
    let days = vec![d(0), d(1), d(2), d(5), d(6), d(7), d(8)];
    assert_eq!(streaks(&days, now), Streak { current: 3, longest: 4 });
    assert_eq!(streaks(&[d(2), d(3)], now), Streak { current: 0, longest: 2 });
    assert_eq!(streaks(&[d(1)], now), Streak { current: 1, longest: 1 });
    assert_eq!(streaks(&[], now), Streak::default());
}

#[test]
fn json_shape() {
    let db = db();
    let v = serde_json::to_value(get(&db, "2024-01-01", "2024-01-31", 5, now_ms()).unwrap()).unwrap();
    assert_eq!(v["range"], serde_json::json!({"from": "2024-01-01", "to": "2024-01-31"}));
    assert!(v["music"]["topTracks"].is_array());
    assert!(v["music"]["byWeekday"].as_array().unwrap().len() == 7);
    assert!(v["video"]["byDay"].is_array());
}

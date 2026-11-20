use std::sync::Arc;

use super::*;
use crate::model::Event;

fn setup() -> (Watch, Arc<Mutex<Vec<Value>>>, SharedDb) {
    let db: SharedDb = Arc::new(Mutex::new(crate::db::open_in_memory().unwrap()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let emit: Emit = Arc::new(move |e| {
        if let Event::Json("watch:changed", v) = e {
            sink.lock().push(v);
        }
    });
    (Watch::new(db.clone(), emit), events, db)
}

fn rec(v: Value) -> WatchRecord {
    WatchRecord::from_json(&v).unwrap()
}

fn watched_seconds(db: &SharedDb) -> f64 {
    db.lock().query_row("SELECT COALESCE(SUM(seconds), 0) FROM watch_time", [], |r| r.get(0)).unwrap()
}

#[test]
fn records_are_read_like_the_original() {
    assert_eq!(WatchRecord::from_json(&json!({"type": "music", "id": 1})), None);
    assert_eq!(WatchRecord::from_json(&json!({"type": "tv", "id": ""})), None);
    assert_eq!(WatchRecord::from_json(&json!({"type": "tv", "id": 0})), None);
    let r = rec(json!({"type": "tv", "id": 1399, "season": 1, "episode": 2.5, "position": "12.5", "duration": null,
                       "year": 2011.0, "genres": ["Drame"], "poster": "", "finished": 0}));
    assert_eq!(r.id, "1399");
    assert_eq!((r.season, r.episode, r.year), (Some(1), None, Some(2011)));
    assert_eq!((r.position, r.duration, r.finished), (12.5, 0.0, false));
    assert_eq!(r.poster, None);
    let l = ListEntry::from_json(&json!({"type": "movie", "id": "603", "rating": "8"})).unwrap();
    assert_eq!(l.rating, None);
}

#[test]
fn progress_finish_and_merge() {
    let (w, _, _) = setup();
    let base = json!({"type": "tv", "id": "1", "season": 1, "episode": 1, "duration": 1000, "title": "Série",
                      "poster": "https://p", "genres": ["A", "B", "C", "D", "E", "F", "G", "H", "I"]});
    let mut v = base.clone();
    v["position"] = json!(500);
    assert_eq!(w.save_at(&rec(v), 1000).unwrap(), Saved { finished: false });
    let mut v = base.clone();
    v["position"] = json!(950);
    assert_eq!(w.save_at(&rec(v), 2000).unwrap(), Saved { finished: true });
    // Revenir au début ne « dé-termine » pas l'épisode ; le titre vide ne remplace pas l'ancien.
    let r = rec(json!({"type": "tv", "id": "1", "season": 1, "episode": 1, "position": 5, "duration": 0}));
    w.save_at(&r, 3000).unwrap();
    let rows = w.show("tv", "1").unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert!(row.finished);
    assert_eq!((row.position, row.duration, row.title.as_str()), (5.0, 1000.0, "Série"));
    assert_eq!(row.poster.as_deref(), Some("https://p"));
    assert_eq!(row.genres.as_array().unwrap().len(), 8);
    assert_eq!((row.first_at, row.updated_at), (1000, 3000));

    let v = serde_json::to_value(row).unwrap();
    for field in [
        "type",
        "id",
        "season",
        "episode",
        "position",
        "duration",
        "finished",
        "title",
        "poster",
        "backdrop",
        "year",
        "genres",
        "firstAt",
        "updatedAt",
    ] {
        assert!(v.get(field).is_some(), "{field}");
    }
}

#[test]
fn watch_time_counts_small_steps_only() {
    let (w, _, db) = setup();
    let at = |pos: f64, now: i64| {
        w.save_at(&rec(json!({"type": "movie", "id": "603", "position": pos, "duration": 8000})), now).unwrap();
    };
    at(100.0, 0);
    at(105.0, 5_000); // +5
    at(110.0, 10_000); // +5
    at(900.0, 15_000); // saut
    at(905.0, 90_000); // trop tard après le message précédent
    at(910.0, 95_000); // +5
    at(905.0, 100_000); // recul
    assert_eq!(watched_seconds(&db), 15.0);
    let season: i64 = db.lock().query_row("SELECT season FROM watch_time LIMIT 1", [], |r| r.get(0)).unwrap();
    assert_eq!(season, -1);
}

#[test]
fn continue_history_remove_clear() {
    let (w, events, db) = setup();
    let save = |id: &str, ep: i64, pos: f64, now: i64| {
        w.save_at(&rec(json!({"type": "anime", "id": id, "episode": ep, "position": pos, "duration": 1400})), now)
            .unwrap();
    };
    save("21", 1, 1390.0, 1); // fini
    save("21", 2, 300.0, 2); // en cours : c'est lui qu'on reprend
    save("30", 1, 5.0, 3); // à peine commencé : ni « continuer » ni historique
    save("40", 1, 1395.0, 4); // dernier épisode fini : pas dans « continuer »
    let cont = w.continue_list(24).unwrap();
    assert_eq!(cont.iter().map(|r| (r.id.as_str(), r.episode)).collect::<Vec<_>>(), [("21", Some(2))]);
    assert_eq!(cont[0].season, None);
    let hist = w.history(80).unwrap();
    assert_eq!(hist.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["40", "21", "21"]);

    w.remove_show("anime", "21").unwrap();
    assert!(w.show("anime", "21").unwrap().is_empty());
    save("50", 1, 100.0, 10);
    save("50", 1, 105.0, 11);
    assert!(watched_seconds(&db) > 0.0);
    w.clear_history().unwrap();
    assert!(w.history(80).unwrap().is_empty());
    assert_eq!(watched_seconds(&db), 0.0);
    assert_eq!(*events.lock(), vec![json!({"what": "progress"}), json!({"what": "progress"})]);
}

#[test]
fn watchlist_toggle() {
    let (w, events, _) = setup();
    let item =
        json!({"type": "movie", "id": 603, "title": "Matrix", "poster": "https://p", "year": 1999, "rating": 8.2});
    assert!(w.toggle_json(&item).unwrap());
    let list = w.list().unwrap();
    assert_eq!(list.len(), 1);
    let v = serde_json::to_value(&list[0]).unwrap();
    assert_eq!(v["type"], "movie");
    assert_eq!(v["id"], "603");
    assert_eq!(v["rating"], 8.2);
    assert!(v["addedAt"].as_i64().unwrap() > 0);
    assert!(!w.toggle_json(&item).unwrap());
    assert!(w.list().unwrap().is_empty());
    assert!(!w.toggle_json(&json!({"type": "book", "id": 1})).unwrap());
    assert_eq!(events.lock().len(), 2);
    assert_eq!(events.lock()[0], json!({"what": "list"}));
}

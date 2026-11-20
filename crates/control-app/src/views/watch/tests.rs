use serde_json::json;

use control_core::media::{AnimeDetails, AnimeEpisode, Details, Progress, SeasonSummary, TmdbDetails};

use super::cards::{Item, anime_genre_label, episode_tag, kind_label};
use super::mylist::relative;
use super::play::{Meta, opened_record};
use super::title::{goal, short_date};
use super::{RemoteImages, Tab, WatchRoute};

fn progress(season: Option<i64>, episode: Option<i64>, position: f64, finished: bool) -> Progress {
    Progress {
        kind: "tv".into(),
        id: "1".into(),
        season,
        episode,
        position,
        duration: 1800.0,
        finished,
        title: "T".into(),
        poster: Some("https://p".into()),
        backdrop: None,
        year: Some(2020),
        genres: json!(["Drame"]),
        first_at: 0,
        updated_at: 0,
    }
}

fn series() -> Details {
    let season = |number, episode_count| SeasonSummary { number, episode_count, ..Default::default() };
    Details::Tmdb(Box::new(TmdbDetails { seasons: vec![season(1, 3), season(2, 5)], ..Default::default() }))
}

#[test]
fn routes_and_tabs() {
    assert_eq!(WatchRoute::Movies.section(), "watch");
    assert_eq!(WatchRoute::Title { kind: "anime".into(), id: "1".into() }.section(), "anime");
    assert_eq!(WatchRoute::Shelf { source: "anime:genre:Action".into(), title: String::new() }.section(), "anime");
    assert_eq!(WatchRoute::Shelf { source: "tmdb:movie/popular".into(), title: String::new() }.section(), "watch");
    assert_eq!(WatchRoute::MyList.section(), "mylist");
    assert_eq!(Tab::parse("tv"), Tab::Tv);
    assert_eq!(Tab::parse("x"), Tab::All);
    assert!(Tab::All.accepts("movie") && Tab::All.accepts("tv") && !Tab::All.accepts("anime"));
    assert!(Tab::Movie.accepts("movie") && !Tab::Movie.accepts("tv"));
}

#[test]
fn labels() {
    let _guard = crate::i18n::TEST_LANG.lock().unwrap_or_else(|e| e.into_inner());
    crate::i18n::set_language("fr");
    assert_eq!(kind_label("tv"), "Série");
    assert_eq!(anime_genre_label("Slice of Life"), "Tranche de vie");
    assert_eq!(anime_genre_label("Action"), "Action");
    assert_eq!(episode_tag(Some(2), Some(3)).as_deref(), Some("S2 · E3"));
    assert_eq!(episode_tag(None, Some(7)).as_deref(), Some("Ép. 7"));
    assert_eq!(episode_tag(None, None), None);
    assert_eq!(short_date("2023-04-05").as_deref(), Some("5 avr. 2023"));
    assert_eq!(short_date(""), None);
    let now = 1_000_000_000_000;
    assert_eq!(relative(now - 5 * 60_000, now), "il y a 5 minutes");
    assert_eq!(relative(now - 86_400_000 - 10, now), "hier");
    assert_eq!(relative(now - 3 * 86_400_000, now), "il y a 3 jours");
    assert_eq!(relative(now - 90 * 86_400_000, now), "il y a 3 mois");
    crate::i18n::set_language("en");
    assert_eq!(anime_genre_label("Slice of Life"), "Slice of Life");
    assert_eq!(short_date("2023-04-05").as_deref(), Some("Apr 5, 2023"));
    assert_eq!(relative(now - 3 * 86_400_000, now), "3 days ago");
    crate::i18n::set_language("fr");
}

#[test]
fn goal_follows_progress() {
    let _guard = crate::i18n::TEST_LANG.lock().unwrap_or_else(|e| e.into_inner());
    crate::i18n::set_language("fr");
    let d = series();
    // Jamais vu : premier épisode de la première saison.
    let g = goal("tv", &d, &[]);
    assert_eq!((g.season, g.episode, g.label.as_str()), (Some(1), Some(1), "Lecture"));
    assert_eq!(g.sub.as_deref(), Some("S1 · E1"));
    // Épisode ouvert dans le navigateur (position inconnue) : on y retourne, sans reprise.
    let g = goal("tv", &d, &[progress(Some(1), Some(2), 0.0, false)]);
    assert_eq!((g.season, g.episode, g.label.as_str()), (Some(1), Some(2), "Lecture"));
    // Bien avancé : reprise.
    let g = goal("tv", &d, &[progress(Some(1), Some(2), 600.0, false)]);
    assert_eq!(g.label, "Reprendre");
    assert_eq!(g.sub.as_deref(), Some("S1 · E2 · 20 min restantes"));
    // Fini : le suivant, d'une saison à l'autre ; au bout : depuis le début.
    let g = goal("tv", &d, &[progress(Some(1), Some(3), 0.0, true)]);
    assert_eq!((g.season, g.episode, g.label.as_str()), (Some(2), Some(1), "Épisode suivant"));
    let g = goal("tv", &d, &[progress(Some(2), Some(5), 0.0, true)]);
    assert_eq!((g.season, g.episode, g.label.as_str()), (Some(1), Some(1), "Revoir depuis le début"));
    // Film d'animation : pas d'épisode.
    let film = Details::Anime(Box::new(AnimeDetails { is_movie: true, ..Default::default() }));
    let g = goal("anime", &film, &[]);
    assert_eq!((g.season, g.episode, g.sub), (None, None, None));
    let show = Details::Anime(Box::new(AnimeDetails {
        episodes: (1..=4).map(|number| AnimeEpisode { number, ..Default::default() }).collect(),
        ..Default::default()
    }));
    let mut p = progress(None, Some(2), 0.0, true);
    p.kind = "anime".into();
    let g = goal("anime", &show, &[p]);
    assert_eq!((g.season, g.episode), (None, Some(3)));
    assert_eq!(g.sub.as_deref(), Some("Ép. 3"));
}

#[test]
fn opened_record_keeps_known_progress() {
    let meta = Meta {
        title: "Série".into(),
        poster: None,
        backdrop: Some("https://b".into()),
        year: Some(2021),
        genres: vec![json!("Drame")],
    };
    // Nouvel épisode : position 0, rien de fini.
    let rec = opened_record("tv", "1", Some(1), Some(4), &[progress(Some(1), Some(2), 900.0, false)], &meta);
    assert_eq!((rec.position, rec.duration, rec.finished), (0.0, 0.0, false));
    assert_eq!((rec.season, rec.episode), (Some(1), Some(4)));
    assert_eq!(rec.title, "Série");
    // Épisode commencé ailleurs (serveur web) : sa position est gardée, il passe seulement en tête.
    let rec = opened_record("tv", "1", Some(1), Some(2), &[progress(Some(1), Some(2), 900.0, false)], &meta);
    assert_eq!((rec.position, rec.duration, rec.finished), (900.0, 1800.0, false));
}

#[test]
fn opened_record_saves_without_watch_time() {
    // Enregistré deux fois de suite à la même position, l'épisode n'ajoute aucun temps regardé.
    let db: control_core::library::SharedDb =
        std::sync::Arc::new(parking_lot::Mutex::new(control_core::db::open_in_memory().expect("base")));
    let watch = control_core::media::Watch::new(db.clone(), control_core::model::emit_nothing());
    let rec = opened_record("movie", "603", None, None, &[], &Meta::default());
    watch.save(&rec).expect("enregistré");
    watch.save(&rec).expect("enregistré");
    let rows = watch.show("movie", "603").expect("lu");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].position, 0.0);
    let seconds: f64 =
        db.lock().query_row("SELECT COALESCE(SUM(seconds), 0) FROM watch_time", [], |r| r.get(0)).expect("somme");
    assert_eq!(seconds, 0.0);
    // Ni reprise ni historique pour une simple ouverture.
    assert!(watch.continue_list(10).expect("lu").is_empty());
}

#[test]
fn items_convert_to_list_entries() {
    let item = Item {
        kind: "tv".into(),
        id: "7".into(),
        title: "X".into(),
        year: Some(2001),
        rating: Some(8.5),
        ..Default::default()
    };
    let e = item.entry();
    assert_eq!((e.kind.as_str(), e.id.as_str(), e.year, e.rating), ("tv", "7", Some(2001), Some(8.5)));
}

#[test]
fn image_cache_is_bounded() {
    let images = RemoteImages::default();
    for i in 0..(super::images::MAX_IMAGES + 25) {
        images.mark_failed_for_test(&format!("https://x/{i}"));
    }
    assert_eq!(images.len(), super::images::MAX_IMAGES);
}

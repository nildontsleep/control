use super::*;
use serde_json::json;

const IMG: &str = "https://image.tmdb.org/t/p";

fn trending() -> Value {
    json!({
        "page": 1,
        "total_pages": 1000,
        "results": [
            {"id": 603, "media_type": "movie", "title": "Matrix", "original_title": "The Matrix", "overview": "Neo.",
             "poster_path": "/p.jpg", "backdrop_path": "/b.jpg", "release_date": "1999-03-31", "vote_average": 8.216,
             "genre_ids": [28, 878]},
            {"id": 1399, "media_type": "tv", "name": "Game of Thrones", "original_name": "Game of Thrones",
             "poster_path": null, "backdrop_path": "/got.jpg", "first_air_date": "2011-04-17", "vote_average": 0},
            {"id": 287, "media_type": "person", "name": "Brad Pitt", "profile_path": "/x.jpg"},
            {"id": 5, "media_type": "movie", "title": "Sans image", "poster_path": null, "backdrop_path": null}
        ]
    })
}

#[test]
fn list_keeps_movies_and_series_with_images() {
    let page = map_page(IMG, &trending(), None, 1);
    assert_eq!(page.page, 1);
    assert_eq!(page.total_pages, 500);
    assert_eq!(page.items.len(), 2);
    let m = &page.items[0];
    assert_eq!(m.key, "movie:603");
    assert_eq!(m.id, "603");
    assert_eq!(m.poster.as_deref(), Some("https://image.tmdb.org/t/p/w342/p.jpg"));
    assert_eq!(m.backdrop.as_deref(), Some("https://image.tmdb.org/t/p/w1280/b.jpg"));
    assert_eq!((m.year, m.rating), (Some(1999), Some(8.2)));
    assert_eq!(m.genre_ids, vec![28, 878]);
    let tv = &page.items[1];
    assert_eq!(
        (tv.kind.as_str(), tv.title.as_str(), tv.poster.as_ref(), tv.rating),
        ("tv", "Game of Thrones", None, None)
    );

    // JSON identique à l'original.
    assert_eq!(
        serde_json::to_value(m).unwrap(),
        json!({"key": "movie:603", "type": "movie", "id": "603", "title": "Matrix", "original": "The Matrix",
               "overview": "Neo.", "poster": "https://image.tmdb.org/t/p/w342/p.jpg",
               "backdrop": "https://image.tmdb.org/t/p/w1280/b.jpg", "year": 1999, "rating": 8.2, "genreIds": [28, 878]})
    );
    assert_eq!(serde_json::to_value(&page).unwrap()["totalPages"], 500);
}

#[test]
fn forced_type_for_typed_lists() {
    let d = json!({"results": [{"id": 1, "title": "A", "poster_path": "/a.jpg"}]});
    assert_eq!(map_page(IMG, &d, Some("movie"), 3).items[0].key, "movie:1");
    assert_eq!(map_page(IMG, &d, Some("movie"), 3).page, 3);
    assert!(map_page(IMG, &d, None, 1).items.is_empty());
    assert!(map_page(IMG, &Value::Null, None, 1).items.is_empty());
}

#[test]
fn movie_details() {
    let d = json!({
        "id": 603, "title": "Matrix", "original_title": "The Matrix", "overview": "Neo.", "tagline": "Bienvenue",
        "poster_path": "/p.jpg", "backdrop_path": "/b.jpg", "release_date": "1999-03-31", "vote_average": 8.2,
        "runtime": 136, "status": "Released", "genres": [{"id": 28, "name": "Action"}, {"id": 878, "name": "Science-Fiction"}],
        "credits": {
            "cast": (0..20).map(|i| json!({"id": i, "name": format!("Acteur {i}"), "character": "Neo", "profile_path": if i == 0 { json!("/k.jpg") } else { Value::Null }})).collect::<Vec<_>>(),
            "crew": [{"job": "Director", "name": "Lana"}, {"job": "Producer", "name": "Joel"}, {"job": "Director", "name": "Lilly"}]
        },
        "videos": {"results": [
            {"site": "YouTube", "type": "Teaser", "key": "teaser", "official": true},
            {"site": "Vimeo", "type": "Trailer", "key": "vimeo", "official": true},
            {"site": "YouTube", "type": "Trailer", "key": "unofficial", "official": false},
            {"site": "YouTube", "type": "Trailer", "key": "official", "official": true}
        ]},
        "release_dates": {"results": [
            {"iso_3166_1": "US", "release_dates": [{"certification": "R"}]},
            {"iso_3166_1": "FR", "release_dates": [{"certification": ""}, {"certification": "12"}]}
        ]},
        "recommendations": {"results": [{"id": 604, "title": "Reloaded", "poster_path": "/r.jpg"}]},
        "similar": {"results": []}
    });
    let det = map_details(IMG, &d, "movie", "FR");
    assert_eq!(det.item.key, "movie:603");
    assert_eq!(det.tagline, "Bienvenue");
    assert_eq!(det.runtime, Some(136));
    assert_eq!(det.genres, vec!["Action", "Science-Fiction"]);
    assert_eq!(det.creators, vec!["Lana", "Lilly"]);
    assert_eq!(det.certification.as_deref(), Some("12"));
    assert_eq!(det.trailer.as_deref(), Some("official"));
    assert_eq!(det.cast.len(), 18);
    assert_eq!(det.cast[0].profile.as_deref(), Some("https://image.tmdb.org/t/p/w185/k.jpg"));
    assert_eq!(det.cast[0].role, "Neo");
    assert_eq!(det.recommendations[0].key, "movie:604");
    assert!(det.seasons.is_empty());
    assert_eq!(det.number_of_seasons, None);
    assert_eq!(map_details(IMG, &d, "movie", "US").certification.as_deref(), Some("R"));

    let v = serde_json::to_value(&det).unwrap();
    for field in ["key", "type", "genreIds", "tagline", "releaseDate", "numberOfSeasons", "numberOfEpisodes", "similar"]
    {
        assert!(v.get(field).is_some(), "{field}");
    }
    assert_eq!(v["releaseDate"], "1999-03-31");
    assert_eq!(v["numberOfSeasons"], Value::Null);
    // Relu depuis le cache.
    assert_eq!(serde_json::from_value::<TmdbDetails>(v).unwrap(), det);
}

#[test]
fn series_details_and_season() {
    let d = json!({
        "id": 1399, "name": "Game of Thrones", "first_air_date": "2011-04-17", "episode_run_time": [60],
        "number_of_seasons": 8, "number_of_episodes": 73, "created_by": [{"name": "D. B."}, {"name": "D. W."}],
        "content_ratings": {"results": [{"iso_3166_1": "FR", "rating": "16"}]},
        "seasons": [
            {"season_number": 0, "name": "Spéciaux", "episode_count": 10},
            {"season_number": 1, "name": "Saison 1", "poster_path": "/s1.jpg", "air_date": "2011-04-17", "episode_count": 10}
        ]
    });
    let det = map_details(IMG, &d, "tv", "FR");
    // Sans image : la fiche reste utilisable.
    assert_eq!((det.item.key.as_str(), det.item.poster.as_ref()), ("tv:1399", None));
    assert_eq!(det.item.year, Some(2011));
    assert_eq!(det.runtime, Some(60));
    assert_eq!(det.creators, vec!["D. B.", "D. W."]);
    assert_eq!(det.certification.as_deref(), Some("16"));
    assert_eq!(det.trailer, None);
    assert_eq!(det.seasons.len(), 1);
    assert_eq!(det.seasons[0].poster.as_deref(), Some("https://image.tmdb.org/t/p/w342/s1.jpg"));
    assert_eq!(
        serde_json::to_value(&det.seasons[0]).unwrap(),
        json!({"number": 1, "name": "Saison 1", "overview": "", "poster": "https://image.tmdb.org/t/p/w342/s1.jpg",
               "airDate": "2011-04-17", "episodeCount": 10})
    );

    let s = map_season(
        IMG,
        &json!({
            "season_number": 1, "name": "Saison 1",
            "episodes": [{"episode_number": 1, "name": "L'hiver vient", "still_path": "/e1.jpg", "air_date": "2011-04-17",
                          "runtime": 62, "vote_average": 7.86}]
        }),
    );
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        json!({"number": 1, "name": "Saison 1", "overview": "", "episodes": [
            {"number": 1, "name": "L'hiver vient", "overview": "", "still": "https://image.tmdb.org/t/p/w300/e1.jpg",
             "airDate": "2011-04-17", "runtime": 62, "rating": 7.9}]})
    );
}

#[test]
fn sources_and_requests() {
    assert_eq!(parse_source("trending").unwrap().0, "/trending/all/week");
    let (path, forced, extra) = parse_source("movie/top_rated").unwrap();
    assert_eq!((path.as_str(), forced), ("/movie/top_rated", Some("movie")));
    assert!(extra.is_empty());
    let (path, forced, extra) = parse_source("discover/tv/16").unwrap();
    assert_eq!((path.as_str(), forced), ("/discover/tv", Some("tv")));
    assert!(extra.contains(&("with_genres", "16".to_string())));
    for bad in ["movie/../x", "movie", "person/popular", "discover/person/1", ""] {
        assert_eq!(parse_source(bad).unwrap_err().code, ErrorCode::Http, "{bad}");
    }

    let params: Params = vec![("page", "1".into()), ("language", "fr-FR".into()), ("query", "le roi".into())];
    assert_eq!(cache_key("/search/multi", &params), "tmdb:/search/multi?language=fr-FR&page=1&query=le roi");

    let r = build_request(DEFAULT_BASE, "/search/multi", &params, "abc").unwrap();
    assert_eq!(r.url, "https://api.themoviedb.org/3/search/multi?api_key=abc&page=1&language=fr-FR&query=le+roi");
    assert!(!r.headers.iter().any(|(k, _)| *k == "authorization"));
    let r = build_request(DEFAULT_BASE, "/authentication", &Vec::new(), "eyJhbGciOi").unwrap();
    assert_eq!(r.url, "https://api.themoviedb.org/3/authentication");
    assert!(r.headers.contains(&("authorization", "Bearer eyJhbGciOi".to_string())));
    assert_eq!(build_request(DEFAULT_BASE, "/x", &Vec::new(), "").unwrap_err().code, ErrorCode::NoKey);
}

#[test]
fn helpers() {
    assert_eq!(year("2024-01-02"), Some(2024));
    assert_eq!(year(""), None);
    assert_eq!(year("abcd"), None);
    assert_eq!(round1(&json!(7.25)), Some(7.3));
    assert_eq!(round1(&json!(0)), None);
    assert_eq!(
        map_genres(&json!({"genres": [{"id": 16, "name": "Animation"}]})),
        vec![Genre { id: json!(16), name: "Animation".into() }]
    );
    assert!(check_id("123").is_ok());
    assert_eq!(check_id("12/../x").unwrap_err().code, ErrorCode::NotFound);
}

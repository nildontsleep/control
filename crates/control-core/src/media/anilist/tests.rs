use super::*;

fn media(id: i64, title: &str, poster: Option<&str>) -> Value {
    json!({
        "id": id, "idMal": id + 1000,
        "title": {"romaji": title, "english": format!("{title} (en)"), "native": "日本"},
        "coverImage": {"extraLarge": poster, "large": null, "color": "#e4a15d"},
        "bannerImage": null,
        "description": "Un <i>héros</i>.<br>Deuxième ligne &amp; fin.<br><br><br><br>(Source: Crunchyroll)",
        "episodes": 24, "format": "TV", "status": "RELEASING", "season": "FALL", "seasonYear": 2024,
        "averageScore": 84, "genres": ["Action", "Drama"], "startDate": {"year": 2023},
        "nextAiringEpisode": {"episode": 8, "airingAt": 1_700_000_000}
    })
}

#[test]
fn anime_item_shape() {
    let a = map_anime(&media(21, "One Piece", Some("https://img/op.jpg")), TitlePref::Romaji).unwrap();
    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        json!({"key": "anime:21", "type": "anime", "id": "21", "title": "One Piece", "original": "日本",
               "overview": "Un héros.\nDeuxième ligne & fin.", "poster": "https://img/op.jpg", "backdrop": null,
               "year": 2024, "rating": 8.4, "genres": ["Action", "Drama"], "format": "TV", "status": "RELEASING",
               "episodes": 24, "available": 7, "color": "#e4a15d", "idMal": 1021})
    );
    assert_eq!(map_anime(&media(21, "One Piece", None), TitlePref::English).unwrap().title, "One Piece (en)");
    assert_eq!(map_anime(&Value::Null, TitlePref::Romaji), None);
}

#[test]
fn text_cleanup_and_seasons() {
    assert_eq!(clean("a<BR />b &lt;c&gt; &quot;d&quot; &#039;e&#039;&nbsp;f (source: X)"), "a\nb <c> \"d\" 'e' f");
    assert_eq!(clean(""), "");
    assert_eq!(season_of(0), "WINTER");
    assert_eq!(season_of(5), "SPRING");
    assert_eq!(season_of(8), "SUMMER");
    assert_eq!(season_of(11), "FALL");
    let finished = json!({"episodes": 12, "nextAiringEpisode": null});
    assert_eq!(available_episodes(&finished), Some(12));
    assert_eq!(available_episodes(&json!({"nextAiringEpisode": {"episode": 1}})), Some(0));
}

#[test]
fn home_and_list_pages() {
    let d = json!({
        "trending": {"media": [media(1, "A", Some("https://p/1")), media(2, "B", None)]},
        "season": {"media": []},
        "popular": {"media": [media(3, "C", Some("https://p/3"))]}
    });
    let shelves = map_home(&d, TitlePref::Romaji);
    assert_eq!(shelves.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), HOME_SHELVES);
    assert_eq!(shelves[0].source, "anime:trending");
    assert_eq!(shelves[0].items.len(), 1);
    assert!(shelves[1].items.is_empty());
    assert!(shelves[4].items.is_empty());

    let p = map_page(
        &json!({"pageInfo": {"currentPage": 2, "lastPage": 9}, "media": [media(4, "D", Some("https://p/4"))]}),
        TitlePref::Romaji,
        2,
    );
    assert_eq!((p.page, p.total_pages, p.items[0].key.as_str()), (2, 9, "anime:4"));
    assert_eq!(map_genres(&json!({"GenreCollection": ["Action", "Hentai", "Drama"]})), vec!["Action", "Drama"]);
}

#[test]
fn queries() {
    let (q, vars) = list_query("genre:Slice of Life", 3, ("FALL", 2026)).unwrap();
    assert!(q.starts_with("query List($page: Int, $genre: String)"));
    assert!(q.contains("genre: $genre, sort: POPULARITY_DESC"));
    assert!(q.contains("fragment M on Media"));
    assert_eq!(vars, json!({"page": 3, "genre": "Slice of Life"}));
    let (_, vars) = list_query("season", 1, ("FALL", 2026)).unwrap();
    assert_eq!(vars, json!({"page": 1, "season": "FALL", "year": 2026}));
    assert!(list_query("movies", 1, ("FALL", 2026)).unwrap().0.contains("format: MOVIE"));
    assert_eq!(list_query("nope", 1, ("FALL", 2026)).unwrap_err().code, ErrorCode::Http);
    assert!(home_query().contains("movies: Page(perPage: 20)"));
    assert!(details_query().ends_with("nextAiringEpisode { episode airingAt }\n}"));

    let r = build_request(DEFAULT_URL, "{ GenreCollection }", &json!({}));
    assert_eq!(r.method, Method::Post);
    let body: Value = serde_json::from_str(r.body.as_deref().unwrap()).unwrap();
    assert_eq!(body, json!({"query": "{ GenreCollection }", "variables": {}}));

    assert_eq!(unwrap_data(json!({"data": {"x": 1}})).unwrap(), json!({"x": 1}));
    let err = unwrap_data(json!({"data": null, "errors": [{"message": "Too Many Requests."}]})).unwrap_err();
    assert_eq!((err.code, err.message.as_str()), (ErrorCode::Http, "Too Many Requests."));
    assert_eq!(unwrap_data(Value::Null).unwrap(), Value::Null);
}

#[test]
fn details_with_episodes_relations_and_cast() {
    let mut m = media(21, "One Piece", Some("https://img/op.jpg"));
    m["duration"] = json!(24);
    m["trailer"] = json!({"id": "yt123", "site": "youtube"});
    m["studios"] = json!({"nodes": [{"name": "Toei"}]});
    m["characters"] = json!({"edges": [
        {"role": "MAIN", "node": {"id": 40, "name": {"full": "Luffy"}, "image": {"medium": "https://c/40"}}},
        {"role": "SUPPORTING", "node": {"id": 41, "name": {"full": "Koby"}, "image": null}}
    ]});
    m["streamingEpisodes"] = json!([
        {"title": "Episode 2 - Le grand combat", "thumbnail": "https://t/2"},
        {"title": "Épisode 1 : Début", "thumbnail": null},
        {"title": "Bonus"}
    ]);
    let mut sequel = media(22, "Suite", Some("https://p/22"));
    sequel["type"] = json!("ANIME");
    let mut prequel = media(20, "Avant", Some("https://p/20"));
    prequel["type"] = json!("ANIME");
    let mut manga = media(23, "Manga", Some("https://p/23"));
    manga["type"] = json!("MANGA");
    m["relations"] = json!({"edges": [
        {"relationType": "SEQUEL", "node": sequel},
        {"relationType": "ADAPTATION", "node": prequel.clone()},
        {"relationType": "PREQUEL", "node": prequel},
        {"relationType": "PREQUEL", "node": manga}
    ]});
    m["recommendations"] = json!({"nodes": [
        {"mediaRecommendation": media(30, "Reco", Some("https://p/30"))},
        {"mediaRecommendation": null},
        {"mediaRecommendation": media(31, "Sans affiche", None)}
    ]});

    let d = map_details(&m, TitlePref::Romaji).unwrap();
    assert!(!d.is_movie);
    assert_eq!(d.runtime, Some(24));
    assert_eq!(d.trailer.as_deref(), Some("yt123"));
    assert_eq!(d.studios, vec!["Toei"]);
    assert_eq!(
        d.cast[0],
        Person { id: 40, name: "Luffy".into(), role: "Principal".into(), profile: Some("https://c/40".into()) }
    );
    assert_eq!(d.cast[1].role, "Secondaire");
    // 7 épisodes sortis (le 8 arrive).
    assert_eq!(d.episodes.len(), 7);
    assert_eq!(d.episodes[0], AnimeEpisode { number: 1, name: "Début".into(), still: None });
    assert_eq!(
        d.episodes[1],
        AnimeEpisode { number: 2, name: "Le grand combat".into(), still: Some("https://t/2".into()) }
    );
    assert_eq!(
        d.related.iter().map(|r| (r.relation.as_str(), r.item.id.as_str())).collect::<Vec<_>>(),
        [("PREQUEL", "20"), ("SEQUEL", "22")]
    );
    assert_eq!(d.recommendations.len(), 1);
    assert_eq!(d.next_airing, Some(NextAiring { episode: 8, at: 1_700_000_000_000 }));

    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(v["isMovie"], false);
    assert_eq!(v["episodes"][1], json!({"number": 2, "name": "Le grand combat", "still": "https://t/2"}));
    assert_eq!(v["related"][0]["relation"], "PREQUEL");
    assert_eq!(v["related"][0]["key"], "anime:20");
    assert_eq!(v["nextAiring"], json!({"episode": 8, "at": 1_700_000_000_000_i64}));
    assert_eq!(serde_json::from_value::<AnimeDetails>(v).unwrap(), d);

    // Film : pas d'épisodes. Série terminée sans liste : 12 épisodes par défaut.
    let mut film = media(5, "Film", Some("https://p/5"));
    film["format"] = json!("MOVIE");
    film["nextAiringEpisode"] = Value::Null;
    assert!(map_details(&film, TitlePref::Romaji).unwrap().episodes.is_empty());
    let mut unknown = media(6, "Inconnu", Some("https://p/6"));
    unknown["episodes"] = Value::Null;
    unknown["nextAiringEpisode"] = Value::Null;
    assert_eq!(map_details(&unknown, TitlePref::Romaji).unwrap().episodes.len(), 12);
    assert_eq!(map_details(&Value::Null, TitlePref::Romaji).unwrap_err().code, ErrorCode::NotFound);
}

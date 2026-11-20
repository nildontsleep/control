//! Le service entier, avec un transport qui rend des réponses enregistrées (aucun accès réseau).

use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{Value, json};

use super::*;
use crate::model::emit_nothing;

/// Répond selon un morceau de l'adresse (TMDB) ou de la requête GraphQL (AniList) ; note chaque requête.
struct Canned {
    routes: Vec<(&'static str, Result<Value, MediaError>)>,
    seen: Mutex<Vec<HttpRequest>>,
}

impl Fetch for Canned {
    fn fetch(&self, req: &HttpRequest) -> Result<Value, MediaError> {
        self.seen.lock().push(req.clone());
        let hay = format!("{} {}", req.url, req.body.as_deref().unwrap_or(""));
        self.routes
            .iter()
            .find(|(needle, _)| hay.contains(needle))
            .map(|(_, r)| r.clone())
            .unwrap_or_else(|| Err(MediaError::new(ErrorCode::NotFound, format!("pas de réponse pour {}", req.url))))
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    store: Store,
    db: SharedDb,
    fetch: Arc<Canned>,
    service: MediaService,
}

fn setup(routes: Vec<(&'static str, Result<Value, MediaError>)>, fallback: Option<&'static str>) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("settings.json"));
    let db: SharedDb = Arc::new(Mutex::new(crate::db::open_in_memory().unwrap()));
    let fetch = Arc::new(Canned { routes, seen: Mutex::new(Vec::new()) });
    let service = MediaService::with_options(
        db.clone(),
        store.clone(),
        emit_nothing(),
        MediaOptions {
            endpoints: Endpoints::default(),
            fetch: fetch.clone(),
            fallback_key: fallback.map(|k| Arc::new(move || k.to_string()) as KeyProvider),
        },
    );
    Setup { _dir: dir, store, db, fetch, service }
}

fn movie_list(id: i64) -> Value {
    json!({"page": 1, "total_pages": 3, "results": [{"id": id, "title": format!("Film {id}"), "poster_path": "/p.jpg"}]})
}

fn offline() -> MediaError {
    MediaError::new(ErrorCode::Network, "hors ligne")
}

#[test]
fn tmdb_home_skips_failed_shelves() {
    let s = setup(
        vec![
            ("/movie/popular", Ok(movie_list(1))),
            ("/movie/now_playing", Err(offline())),
            ("/movie/top_rated", Ok(json!({"results": []}))),
            ("/movie/upcoming", Ok(movie_list(4))),
            ("/genre/movie/list", Ok(json!({"genres": [{"id": 28, "name": "Action"}]}))),
        ],
        None,
    );
    s.store.patch(&json!({"media": {"tmdbKey": " abc "}}));
    let home = s.service.home("movie", "en").unwrap();
    let v = serde_json::to_value(&home).unwrap();
    assert_eq!(
        v["shelves"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect::<Vec<_>>(),
        ["moviePopular", "upcoming"]
    );
    assert_eq!(v["shelves"][0]["source"], "tmdb:movie/popular");
    assert_eq!(v["shelves"][0]["items"][0]["key"], "movie:1");
    assert_eq!(v["genres"], json!([{"id": 28, "name": "Action"}]));
    let seen = s.fetch.seen.lock();
    assert!(seen.iter().all(|r| r.url.contains("api_key=abc") && r.url.contains("language=en-US")));
}

#[test]
fn tmdb_without_key() {
    let s = setup(vec![], None);
    let err = s.service.home("all", "fr").unwrap_err();
    assert_eq!(err.code, ErrorCode::NoKey);
    assert_eq!(
        s.service.handle("media:home", &[json!("all"), json!("fr")]).unwrap(),
        json!({"ok": false, "code": "nokey", "message": "Aucune clé TMDB"})
    );
    assert!(s.fetch.seen.lock().is_empty());
    assert_eq!(s.service.home("nope", "fr").unwrap(), Home::default());
}

#[test]
fn server_key_is_used_when_the_account_has_none() {
    let s = setup(vec![("/tv/1399?", Ok(json!({"id": 1399, "name": "GoT", "poster_path": "/g.jpg"})))], Some("srv"));
    assert_eq!(s.service.tmdb_key(), "srv");
    let d = s.service.details("tv", "1399", "fr").unwrap();
    assert!(matches!(&d, Details::Tmdb(t) if t.item.title == "GoT"));
    assert!(s.fetch.seen.lock()[0].url.contains("api_key=srv"));
    s.store.patch(&json!({"media": {"tmdbKey": "mine"}}));
    assert_eq!(s.service.tmdb_key(), "mine");
    assert_eq!(s.service.details("person", "1", "fr").unwrap_err().code, ErrorCode::Http);
    assert_eq!(s.service.details("movie", "../x", "fr").unwrap_err().code, ErrorCode::NotFound);
}

#[test]
fn list_is_cached_and_served_offline() {
    let s = setup(vec![("/tv/popular", Ok(movie_list(7)))], Some("k"));
    let page = s.service.list("tmdb:tv/popular", 1, "fr").unwrap();
    assert_eq!(serde_json::to_value(&page).unwrap()["items"][0]["key"], "tv:7");
    s.service.list("tmdb:tv/popular", 1, "fr").unwrap();
    assert_eq!(s.fetch.seen.lock().len(), 1);
    assert_eq!(s.service.list("bogus:x", 1, "fr").unwrap_err().code, ErrorCode::Http);

    // La même clé de cache que la version d'origine.
    let key = "tmdb:/tv/popular?language=fr-FR&page=1";
    assert!(s.service.cache.get::<Page<TmdbItem>>(key).is_some());
    // Périmé et hors ligne : la page d'avant sert encore.
    s.service.cache.set(key, &page.map(|_| TmdbItem { key: "tv:old".into(), ..Default::default() }), -1);
    let s2 = MediaService::with_options(
        s.db.clone(),
        s.store.clone(),
        emit_nothing(),
        MediaOptions {
            endpoints: Endpoints::default(),
            fetch: Arc::new(Canned { routes: vec![("/tv/popular", Err(offline()))], seen: Mutex::new(Vec::new()) }),
            fallback_key: Some(Arc::new(|| "k".to_string())),
        },
    );
    let page = s2.list("tmdb:tv/popular", 1, "fr").unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(matches!(&page.items[0], CatalogItem::Tmdb(t) if t.key == "tv:old"));
    s2.clear_cache().unwrap();
    assert_eq!(s2.list("tmdb:tv/popular", 1, "fr").unwrap_err().code, ErrorCode::Network);
}

#[test]
fn anime_home_list_and_details() {
    let anime = |id: i64| {
        json!({"id": id, "title": {"romaji": format!("Anime {id}")}, "coverImage": {"large": format!("https://p/{id}")},
               "format": "TV", "episodes": 3})
    };
    let s = setup(
        vec![
            ("GenreCollection", Ok(json!({"data": {"GenreCollection": ["Action", "Hentai"]}}))),
            ("query Home", Ok(json!({"data": {"trending": {"media": [anime(1)]}}}))),
            (
                "query List",
                Ok(json!({"data": {"Page": {"pageInfo": {"currentPage": 2, "lastPage": 4}, "media": [anime(2)]}}})),
            ),
            ("query Details", Ok(json!({"data": {"Media": anime(3)}}))),
            ("query Search", Err(MediaError::new(ErrorCode::RateLimit, "trop"))),
        ],
        None,
    );
    s.store.patch(&json!({"media": {"animeTitles": "english"}}));
    let home = serde_json::to_value(s.service.home("anime", "fr").unwrap()).unwrap();
    assert_eq!(home["shelves"].as_array().unwrap().len(), 5);
    assert_eq!(home["shelves"][0]["items"][0]["key"], "anime:1");
    assert_eq!(home["genres"], json!([{"id": "Action", "name": "Action"}]));

    let page = serde_json::to_value(s.service.list("anime:genre:Action", 2, "fr").unwrap()).unwrap();
    assert_eq!((page["page"].as_i64(), page["totalPages"].as_i64()), (Some(2), Some(4)));
    let seen = s.fetch.seen.lock().clone();
    let body: Value = serde_json::from_str(seen.last().unwrap().body.as_deref().unwrap()).unwrap();
    assert_eq!(body["variables"], json!({"page": 2, "genre": "Action"}));
    assert!(s.service.cache.get::<Page<AnimeItem>>("anilist:english:list:genre:Action:2").is_some());

    let d = s.service.handle("media:details", &[json!("anime"), json!(3), json!("fr")]).unwrap();
    assert_eq!(d["ok"], true);
    assert_eq!(d["data"]["episodes"].as_array().unwrap().len(), 3);
    assert_eq!(d["data"]["isMovie"], false);
    assert_eq!(s.service.details("anime", "abc", "fr").unwrap_err().code, ErrorCode::NotFound);

    // Recherche : AniList en échec, pas de clé TMDB.
    let r = serde_json::to_value(s.service.search("  naruto ", "fr")).unwrap();
    assert_eq!(r, json!({"tmdb": [], "anime": [], "tmdbError": "nokey"}));
    assert_eq!(
        serde_json::to_value(s.service.search("a", "fr")).unwrap(),
        json!({"tmdb": [], "anime": [], "tmdbError": null})
    );
}

#[test]
fn search_reports_tmdb_errors() {
    let s = setup(
        vec![
            (
                "/search/multi",
                Err(MediaError { code: ErrorCode::Auth, message: "Invalid API key".into(), status: 401 }),
            ),
            (
                "query Search",
                Ok(
                    json!({"data": {"Page": {"media": [{"id": 9, "title": {"romaji": "N"}, "coverImage": {"large": "https://p"}}]}}}),
                ),
            ),
        ],
        Some("bad"),
    );
    let r = s.service.search("naruto", "fr");
    assert_eq!(r.tmdb_error.as_deref(), Some("auth"));
    assert_eq!(r.anime.len(), 1);
    assert_eq!(s.service.test_key("bad").unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(s.service.test_key("  ").unwrap_err().code, ErrorCode::NoKey);
}

#[test]
fn handle_watch_channels_and_stream() {
    let s = setup(vec![], None);
    let h = |c: &str, args: &[Value]| s.service.handle(c, args).unwrap();
    let saved = h("watch:save", &[json!({"type": "movie", "id": 603, "position": 120, "duration": 100})]);
    assert_eq!(saved, json!({"ok": true, "data": {"finished": true}}));
    assert_eq!(h("watch:save", &[json!({"type": "x"})]), json!({"ok": true, "data": null}));
    assert_eq!(h("watch:show", &[json!("movie"), json!(603)])["data"][0]["id"], "603");
    assert_eq!(h("watch:continue", &[]), json!({"ok": true, "data": []}));
    assert_eq!(h("watch:history", &[])["data"].as_array().unwrap().len(), 1);
    assert_eq!(
        h("watchlist:toggle", &[json!({"type": "tv", "id": "1", "title": "T"})]),
        json!({"ok": true, "data": true})
    );
    assert_eq!(h("watchlist:list", &[])["data"][0]["title"], "T");
    assert_eq!(h("watch:remove", &[json!("movie"), json!("603")]), json!({"ok": true, "data": null}));
    assert_eq!(h("watch:clear", &[])["ok"], true);
    assert_eq!(h("media:clear-cache", &[])["ok"], true);
    assert_eq!(h("app:env", &[]), json!({"streamOverride": null}));
    assert!(s.service.handle("media:nope", &[]).is_none());

    s.store.patch(&json!({"media": {"streamBase": "https://embed.vidrift.in", "dub": true}}));
    let target = EmbedTarget { kind: "anime".into(), id: "21".into(), season: None, episode: Some(1) };
    let url = s.service.embed_url(&target, &EmbedOptions::default());
    assert!(url.starts_with("https://embed.vidrift.in/embed/anilist/21/1?"));
    assert!(url.ends_with("&dub=1"));
}

#[test]
fn endpoints_origins() {
    let e = Endpoints {
        tmdb_img: "http://127.0.0.1:8123/img".into(),
        stream_override: Some("http://127.0.0.1:9000".into()),
        ..Endpoints::default()
    };
    assert_eq!(e.image_origins(), vec!["http://127.0.0.1:8123"]);
    assert_eq!(e.frame_origins().last().map(String::as_str), Some("http://127.0.0.1:9000"));
    assert!(Endpoints::default().image_origins().is_empty());
}

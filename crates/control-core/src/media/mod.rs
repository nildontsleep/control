//! Service « Films, séries et animés » : catalogues TMDB (films, séries) et AniList (animés), mis en cache dans la
//! base, progression de visionnage et « Ma liste ».
//!
//! Les méthodes de [`MediaService`] reprennent les gestionnaires IPC d'origine (`media:*`, `watch:*`,
//! `watchlist:*`) et rendent les mêmes objets JSON ; [`MediaService::handle`] les expose par nom de canal, avec
//! l'enveloppe `{ ok, data }` / `{ ok: false, code, message }`. Méthodes bloquantes : à appeler depuis un thread de
//! travail.

pub mod anilist;
pub mod cache;
pub mod error;
pub mod http;
pub mod stream;
pub mod tmdb;
pub mod types;
pub mod watch;

use std::sync::Arc;

use serde_json::{Value, json};

pub use anilist::{AniList, TitlePref};
pub use cache::Cache;
pub use error::{ErrorCode, MediaError, envelope};
pub use http::{Fetch, HttpRequest, Method, UreqFetch};
pub use stream::{EmbedOptions, EmbedTarget, EpisodeRef, OFFICIAL_STREAMS, embed_url, neighbour, resume_at};
pub use tmdb::{Lang, Tmdb};
pub use types::*;
pub use watch::{ListEntry, Progress, Saved, Watch, WatchRecord, WatchlistItem};

use crate::library::SharedDb;
use crate::model::Emit;
use crate::store::Store;

/// Adresses des services. Les variables d'environnement d'origine (`CONTROL_TMDB_BASE`, `CONTROL_TMDB_IMG`,
/// `CONTROL_ANILIST_URL`, `CONTROL_STREAM_BASE`) les remplacent, pour les tests de bout en bout.
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoints {
    pub tmdb_base: String,
    pub tmdb_img: String,
    pub anilist_url: String,
    /// Serveur de lecture imposé (sinon le réglage `media.streamBase` s'applique).
    pub stream_override: Option<String>,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            tmdb_base: tmdb::DEFAULT_BASE.into(),
            tmdb_img: tmdb::DEFAULT_IMG.into(),
            anilist_url: anilist::DEFAULT_URL.into(),
            stream_override: None,
        }
    }
}

impl Endpoints {
    pub fn from_env() -> Endpoints {
        let var = |name: &str| std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let d = Endpoints::default();
        Endpoints {
            tmdb_base: var("CONTROL_TMDB_BASE").unwrap_or(d.tmdb_base).trim_end_matches('/').to_string(),
            tmdb_img: var("CONTROL_TMDB_IMG").unwrap_or(d.tmdb_img).trim_end_matches('/').to_string(),
            anilist_url: var("CONTROL_ANILIST_URL").unwrap_or(d.anilist_url),
            stream_override: var("CONTROL_STREAM_BASE").map(|v| v.trim_end_matches('/').to_string()),
        }
    }

    /// Origines d'images en plus de celles de TMDB (CSP `img-src`) : seulement un serveur de test.
    pub fn image_origins(&self) -> Vec<String> {
        if self.tmdb_img == tmdb::DEFAULT_IMG {
            Vec::new()
        } else {
            stream::origin(&self.tmdb_img).into_iter().collect()
        }
    }

    /// Origines que la page peut intégrer en iframe (CSP `frame-src`).
    pub fn frame_origins(&self) -> Vec<String> {
        stream::frame_origins(self.stream_override.as_deref())
    }
}

/// Clé TMDB fournie d'office (serveur web), utilisée quand le compte n'a pas la sienne.
pub type KeyProvider = Arc<dyn Fn() -> String + Send + Sync>;

pub struct MediaOptions {
    pub endpoints: Endpoints,
    pub fetch: Arc<dyn Fetch>,
    pub fallback_key: Option<KeyProvider>,
}

impl Default for MediaOptions {
    fn default() -> Self {
        MediaOptions { endpoints: Endpoints::from_env(), fetch: Arc::new(UreqFetch::default()), fallback_key: None }
    }
}

pub struct MediaService {
    store: Store,
    endpoints: Endpoints,
    fallback_key: Option<KeyProvider>,
    cache: Arc<Cache>,
    tmdb: Tmdb,
    anilist: AniList,
    watch: Watch,
}

/// Rangées TMDB de l'accueil : (identifiant, source).
fn tmdb_shelves(kind: &str) -> &'static [(&'static str, &'static str)] {
    match kind {
        "all" => &[
            ("trending", "trending"),
            ("moviePopular", "movie/popular"),
            ("tvPopular", "tv/popular"),
            ("movieTop", "movie/top_rated"),
            ("tvTop", "tv/top_rated"),
        ],
        "movie" => &[
            ("moviePopular", "movie/popular"),
            ("nowPlaying", "movie/now_playing"),
            ("movieTop", "movie/top_rated"),
            ("upcoming", "movie/upcoming"),
        ],
        "tv" => &[("tvPopular", "tv/popular"), ("onTheAir", "tv/on_the_air"), ("tvTop", "tv/top_rated")],
        _ => &[],
    }
}

impl MediaService {
    /// Service réel : réseau par `ureq`, adresses d'origine (ou celles de l'environnement).
    pub fn new(db: SharedDb, store: Store, emit: Emit) -> MediaService {
        MediaService::with_options(db, store, emit, MediaOptions::default())
    }

    pub fn with_options(db: SharedDb, store: Store, emit: Emit, opts: MediaOptions) -> MediaService {
        let cache = Arc::new(Cache::new(db.clone()));
        let e = &opts.endpoints;
        MediaService {
            tmdb: Tmdb::new(&e.tmdb_base, &e.tmdb_img, cache.clone(), opts.fetch.clone()),
            anilist: AniList::new(&e.anilist_url, cache.clone(), opts.fetch.clone()),
            watch: Watch::new(db, emit),
            cache,
            store,
            endpoints: opts.endpoints,
            fallback_key: opts.fallback_key,
        }
    }

    pub fn endpoints(&self) -> &Endpoints {
        &self.endpoints
    }

    pub fn watch(&self) -> &Watch {
        &self.watch
    }

    /// Clé TMDB en usage : celle des réglages, sinon celle du serveur.
    pub fn tmdb_key(&self) -> String {
        let own = self.store.get("media")["tmdbKey"].as_str().unwrap_or_default().trim().to_string();
        if !own.is_empty() {
            return own;
        }
        self.fallback_key.as_ref().map(|f| f().trim().to_string()).unwrap_or_default()
    }

    fn title_pref(&self) -> TitlePref {
        TitlePref::parse(self.store.get("media")["animeTitles"].as_str().unwrap_or_default())
    }

    /// Serveur vidrift à utiliser.
    pub fn stream_base(&self) -> String {
        let media = self.store.get("media");
        stream::stream_base(media["streamBase"].as_str().unwrap_or_default(), self.endpoints.stream_override.as_deref())
    }

    /// Adresse du lecteur pour `target` (doublage selon le réglage `media.dub` si `opts.dub` n'est pas déjà mis).
    pub fn embed_url(&self, target: &EmbedTarget, opts: &EmbedOptions) -> String {
        let mut opts = opts.clone();
        opts.dub |= self.store.get("media")["dub"] == true;
        embed_url(&self.stream_base(), target, &opts)
    }

    // ─── Catalogue ──────────────────────────────────────────────────────────────────────────────────────────────

    /// Accueil d'un onglet (`all`, `movie`, `tv` ou `anime`) : rangées et genres.
    pub fn home(&self, kind: &str, lang: &str) -> Result<Home, MediaError> {
        let lang = Lang::parse(lang);
        if kind == "anime" {
            let pref = self.title_pref();
            let (home, genres) = std::thread::scope(|s| {
                let genres = s.spawn(|| self.anilist.genres(pref));
                let home = self.anilist.home(pref);
                (home, genres.join().ok().and_then(Result::ok).unwrap_or_default())
            });
            return Ok(Home {
                shelves: home?
                    .into_iter()
                    .map(|s| Shelf {
                        id: s.id,
                        source: s.source,
                        items: s.items.into_iter().map(CatalogItem::Anime).collect(),
                    })
                    .collect(),
                genres: genres.into_iter().map(|g| Genre { id: Value::String(g.clone()), name: g }).collect(),
            });
        }

        let key = self.tmdb_key();
        let defs = tmdb_shelves(kind);
        let (results, genres) = std::thread::scope(|s| {
            let handles: Vec<_> =
                defs.iter().map(|(_, source)| s.spawn(|| self.tmdb.list(&key, source, 1, lang))).collect();
            let genres = if kind == "movie" || kind == "tv" { self.tmdb.genres(&key, kind, lang).ok() } else { None };
            let results: Vec<Result<Page<TmdbItem>, MediaError>> = handles
                .into_iter()
                .map(|h| h.join().unwrap_or_else(|_| Err(MediaError::new(ErrorCode::Error, "Erreur interne"))))
                .collect();
            (results, genres.unwrap_or_default())
        });
        // Une rangée en échec est simplement omise ; toutes en échec : l'erreur remonte (clé refusée, hors ligne…).
        if !results.is_empty() && results.iter().all(Result::is_err) {
            if let Some(Err(err)) = results.into_iter().next() {
                return Err(err);
            }
            return Ok(Home::default());
        }
        let shelves = defs
            .iter()
            .zip(results)
            .filter_map(|((id, source), r)| {
                let items: Vec<CatalogItem> = r.ok()?.items.into_iter().map(CatalogItem::Tmdb).collect();
                (!items.is_empty()).then(|| Shelf { id: id.to_string(), source: format!("tmdb:{source}"), items })
            })
            .collect();
        Ok(Home { shelves, genres })
    }

    /// Une page d'une source (`tmdb:movie/popular`, `tmdb:discover/tv/16`, `anime:genre:Action`…).
    pub fn list(&self, source: &str, page: i64, lang: &str) -> Result<Page<CatalogItem>, MediaError> {
        let page = page.max(1);
        match source.split_once(':') {
            Some(("anime", rest)) => Ok(self.anilist.list(rest, page, self.title_pref())?.map(CatalogItem::Anime)),
            Some(("tmdb", rest)) => {
                Ok(self.tmdb.list(&self.tmdb_key(), rest, page, Lang::parse(lang))?.map(CatalogItem::Tmdb))
            }
            _ => Err(MediaError::unknown_source(source)),
        }
    }

    /// Fiche d'un titre (`movie`, `tv` ou `anime`).
    pub fn details(&self, kind: &str, id: &str, lang: &str) -> Result<Details, MediaError> {
        match kind {
            "anime" => Ok(Details::Anime(Box::new(self.anilist.details(id, self.title_pref())?))),
            "movie" | "tv" => {
                Ok(Details::Tmdb(Box::new(self.tmdb.details(&self.tmdb_key(), kind, id, Lang::parse(lang))?)))
            }
            _ => Err(MediaError::new(ErrorCode::Http, format!("Type inconnu : {kind}"))),
        }
    }

    /// Épisodes d'une saison de série.
    pub fn season(&self, id: &str, number: i64, lang: &str) -> Result<Season, MediaError> {
        self.tmdb.season(&self.tmdb_key(), id, number, Lang::parse(lang))
    }

    /// Recherche dans les deux catalogues. Ne renvoie jamais d'erreur : un catalogue en échec rend une liste vide
    /// (et `tmdbError` dit pourquoi pour TMDB).
    pub fn search(&self, query: &str, lang: &str) -> SearchResults {
        let text = query.trim();
        if text.chars().count() < 2 {
            return SearchResults::default();
        }
        let key = self.tmdb_key();
        let pref = self.title_pref();
        let (tmdb, anime) = std::thread::scope(|s| {
            let anime = s.spawn(|| self.anilist.search(text, pref));
            let tmdb = if key.is_empty() { None } else { Some(self.tmdb.search(&key, text, Lang::parse(lang))) };
            (tmdb, anime.join().ok().and_then(Result::ok).unwrap_or_default())
        });
        match tmdb {
            None => SearchResults { tmdb: Vec::new(), anime, tmdb_error: Some("nokey".into()) },
            Some(Ok(tmdb)) => SearchResults { tmdb, anime, tmdb_error: None },
            Some(Err(err)) => SearchResults { tmdb: Vec::new(), anime, tmdb_error: Some(err.code.as_str().into()) },
        }
    }

    /// Vérifie une clé TMDB sans l'enregistrer.
    pub fn test_key(&self, key: &str) -> Result<bool, MediaError> {
        self.tmdb.test_key(key)
    }

    pub fn clear_cache(&self) -> Result<(), MediaError> {
        self.cache.clear()
    }

    // ─── Visionnage ─────────────────────────────────────────────────────────────────────────────────────────────

    pub fn watch_save(&self, rec: &WatchRecord) -> Result<Saved, MediaError> {
        self.watch.save(rec)
    }

    pub fn watch_show(&self, kind: &str, id: &str) -> Result<Vec<Progress>, MediaError> {
        self.watch.show(kind, id)
    }

    /// « Continuer à regarder » (24 titres au plus).
    pub fn watch_continue(&self) -> Result<Vec<Progress>, MediaError> {
        self.watch.continue_list(24)
    }

    /// Historique (80 entrées au plus).
    pub fn watch_history(&self) -> Result<Vec<Progress>, MediaError> {
        self.watch.history(80)
    }

    pub fn watch_remove(&self, kind: &str, id: &str) -> Result<(), MediaError> {
        self.watch.remove_show(kind, id)
    }

    pub fn watch_clear(&self) -> Result<(), MediaError> {
        self.watch.clear_history()
    }

    pub fn watchlist_list(&self) -> Result<Vec<WatchlistItem>, MediaError> {
        self.watch.list()
    }

    pub fn watchlist_toggle(&self, item: &ListEntry) -> Result<bool, MediaError> {
        self.watch.toggle(item)
    }

    // ─── Canaux d'origine ───────────────────────────────────────────────────────────────────────────────────────

    /// Appelle le gestionnaire d'un canal IPC d'origine (`media:home`, `watch:save`…) avec ses arguments JSON, et
    /// rend l'enveloppe `{ ok, data }` ou `{ ok: false, code, message }`. `None` : canal inconnu.
    pub fn handle(&self, channel: &str, args: &[Value]) -> Option<Value> {
        let arg = |i: usize| args.get(i).unwrap_or(&Value::Null);
        let s = |i: usize| match arg(i) {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => String::new(),
        };
        let n = |i: usize, default: i64| arg(i).as_i64().or_else(|| s(i).parse().ok()).unwrap_or(default);
        let lang = |i: usize| match s(i) {
            l if l.is_empty() => "fr".to_string(),
            l => l,
        };
        Some(match channel {
            "media:home" => envelope(self.home(&s(0), &lang(1))),
            "media:list" => envelope(self.list(&s(0), n(1, 1), &lang(2))),
            "media:details" => envelope(self.details(&s(0), &s(1), &lang(2))),
            "media:season" => envelope(self.season(&s(0), n(1, 1), &lang(2))),
            "media:search" => envelope(Ok(self.search(&s(0), &lang(1)))),
            "media:test-key" => envelope(self.test_key(&s(0))),
            "media:clear-cache" => envelope(self.clear_cache()),
            "watch:save" => envelope(self.watch.save_json(arg(0))),
            "watch:show" => envelope(self.watch_show(&s(0), &s(1))),
            "watch:continue" => envelope(self.watch_continue()),
            "watch:history" => envelope(self.watch_history()),
            "watch:remove" => envelope(self.watch_remove(&s(0), &s(1))),
            "watch:clear" => envelope(self.watch_clear()),
            "watchlist:list" => envelope(self.watchlist_list()),
            "watchlist:toggle" => envelope(self.watch.toggle_json(arg(0))),
            "app:env" => json!({ "streamOverride": self.endpoints.stream_override }),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests;

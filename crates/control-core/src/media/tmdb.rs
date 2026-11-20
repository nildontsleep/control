//! TMDB : films et séries (catalogue, fiches, saisons, recherche). La clé vient des réglages de l'utilisateur, ou
//! du serveur web pour les comptes qui n'en ont pas.
//!
//! La construction des requêtes et la normalisation des réponses sont des fonctions pures ; seul [`Tmdb`] touche
//! au réseau (par son transport) et au cache.

use std::sync::Arc;

use serde_json::Value;

use super::cache::{Cache, DAY, HOUR, MINUTE};
use super::error::{ErrorCode, MediaError};
use super::http::{Fetch, HttpRequest, Method};
use super::types::{
    Genre, Page, Person, Season, SeasonSummary, TmdbDetails, TmdbEpisode, TmdbItem, first_text, float, id_string, int,
    items, text,
};

pub const DEFAULT_BASE: &str = "https://api.themoviedb.org/3";
pub const DEFAULT_IMG: &str = "https://image.tmdb.org/t/p";

const TTL_LIST: i64 = 30 * MINUTE;
const TTL_DETAILS: i64 = 6 * HOUR;
const TTL_SEASON: i64 = 6 * HOUR;
const TTL_GENRES: i64 = 7 * DAY;
const TTL_SEARCH: i64 = 10 * MINUTE;

/// Langue de l'interface : `fr` (par défaut) ou `en`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Fr,
    En,
}

impl Lang {
    pub fn parse(s: &str) -> Lang {
        if s == "en" { Lang::En } else { Lang::Fr }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::Fr => "fr",
            Lang::En => "en",
        }
    }

    /// Paramètre `language` de TMDB.
    pub fn language(self) -> &'static str {
        match self {
            Lang::Fr => "fr-FR",
            Lang::En => "en-US",
        }
    }

    /// Pays dont on lit la classification (« Tous publics », « PG-13 »).
    pub fn region(self) -> &'static str {
        match self {
            Lang::Fr => "FR",
            Lang::En => "US",
        }
    }
}

type Params = Vec<(&'static str, String)>;

// ─── Normalisation (fonctions pures) ────────────────────────────────────────────────────────────────────────────

/// Adresse d'une image TMDB à une taille donnée.
pub fn img(base: &str, size: &str, path: &Value) -> Option<String> {
    text(path).map(|p| format!("{base}/{size}{p}"))
}

/// Année d'une date « AAAA-MM-JJ ».
pub fn year(date: &str) -> Option<i64> {
    let head: String = date.chars().take(4).collect();
    head.trim().parse::<i64>().ok().filter(|y| *y != 0)
}

/// Note arrondie à une décimale ; 0 ou absente : `None`.
pub fn round1(v: &Value) -> Option<f64> {
    float(v).map(|n| (n * 10.0).round() / 10.0)
}

/// Une affiche. `forced` donne le type quand la réponse ne le précise pas (listes `movie/…`, `tv/…`).
/// Les personnes et les titres sans aucune image sont écartés.
pub fn map_item(img_base: &str, m: &Value, forced: Option<&str>) -> Option<TmdbItem> {
    let kind = forced.or_else(|| m["media_type"].as_str())?;
    if kind != "movie" && kind != "tv" {
        return None;
    }
    let poster = img(img_base, "w342", &m["poster_path"]);
    let backdrop = img(img_base, "w1280", &m["backdrop_path"]);
    if poster.is_none() && backdrop.is_none() {
        return None;
    }
    let id = id_string(&m["id"]);
    Some(TmdbItem {
        key: format!("{kind}:{id}"),
        kind: kind.to_string(),
        id,
        title: first_text(&[&m["title"], &m["name"]]),
        original: first_text(&[&m["original_title"], &m["original_name"]]),
        overview: first_text(&[&m["overview"]]),
        poster,
        backdrop,
        year: year(&first_text(&[&m["release_date"], &m["first_air_date"]])),
        rating: round1(&m["vote_average"]),
        genre_ids: items(&m["genre_ids"]).iter().filter_map(Value::as_i64).collect(),
    })
}

pub fn map_list(img_base: &str, results: &Value, forced: Option<&str>) -> Vec<TmdbItem> {
    items(results).iter().filter_map(|m| map_item(img_base, m, forced)).collect()
}

/// Une page de liste (TMDB refuse d'aller au-delà de la page 500).
pub fn map_page(img_base: &str, d: &Value, forced: Option<&str>, page: i64) -> Page<TmdbItem> {
    Page {
        items: map_list(img_base, &d["results"], forced),
        page: int(&d["page"]).unwrap_or(page),
        total_pages: int(&d["total_pages"]).unwrap_or(1).min(500),
    }
}

/// Genres de TMDB, tels quels (`{ id, name }`).
pub fn map_genres(d: &Value) -> Vec<Genre> {
    items(&d["genres"])
        .iter()
        .map(|g| Genre { id: g["id"].clone(), name: g["name"].as_str().unwrap_or_default().to_string() })
        .collect()
}

/// Bande-annonce YouTube : une « Trailer » officielle de préférence, sinon un « Teaser ».
pub fn pick_trailer(videos: &Value) -> Option<String> {
    let mut list: Vec<&Value> = items(&videos["results"])
        .iter()
        .filter(|v| v["site"] == "YouTube" && matches!(v["type"].as_str(), Some("Trailer" | "Teaser")))
        .collect();
    let score = |v: &Value| (if v["type"] == "Trailer" { 2 } else { 0 }) + i32::from(v["official"] == true);
    // Tri stable, comme celui de JavaScript : à score égal, l'ordre de TMDB.
    list.sort_by_key(|v| std::cmp::Reverse(score(v)));
    list.first().and_then(|v| text(&v["key"])).map(str::to_string)
}

/// Classification d'âge pour le pays demandé.
pub fn certification(d: &Value, kind: &str, region: &str) -> Option<String> {
    if kind == "movie" {
        let entry = items(&d["release_dates"]["results"]).iter().find(|r| r["iso_3166_1"] == region)?;
        return items(&entry["release_dates"]).iter().find_map(|x| text(&x["certification"])).map(str::to_string);
    }
    items(&d["content_ratings"]["results"])
        .iter()
        .find(|r| r["iso_3166_1"] == region)
        .and_then(|r| text(&r["rating"]))
        .map(str::to_string)
}

/// Fiche complète (réponse de `/{type}/{id}` avec `append_to_response`).
pub fn map_details(img_base: &str, d: &Value, kind: &str, region: &str) -> TmdbDetails {
    let item = map_item(img_base, d, Some(kind)).unwrap_or_else(|| {
        let id = id_string(&d["id"]);
        TmdbItem {
            key: format!("{kind}:{id}"),
            kind: kind.to_string(),
            id,
            title: first_text(&[&d["title"], &d["name"]]),
            original: first_text(&[&d["original_title"], &d["original_name"]]),
            overview: first_text(&[&d["overview"]]),
            year: year(&first_text(&[&d["release_date"], &d["first_air_date"]])),
            rating: round1(&d["vote_average"]),
            ..TmdbItem::default()
        }
    });
    let names = |list: &Value| -> Vec<String> {
        items(list).iter().filter_map(|c| c["name"].as_str()).map(str::to_string).collect()
    };
    let mut creators: Vec<String> = if kind == "movie" {
        items(&d["credits"]["crew"])
            .iter()
            .filter(|c| c["job"] == "Director")
            .filter_map(|c| c["name"].as_str())
            .map(str::to_string)
            .collect()
    } else {
        names(&d["created_by"])
    };
    creators.truncate(3);
    TmdbDetails {
        item,
        tagline: first_text(&[&d["tagline"]]),
        runtime: int(&d["runtime"]).or_else(|| int(&d["episode_run_time"][0])),
        genres: names(&d["genres"]),
        status: first_text(&[&d["status"]]),
        release_date: first_text(&[&d["release_date"], &d["first_air_date"]]),
        number_of_seasons: int(&d["number_of_seasons"]),
        number_of_episodes: int(&d["number_of_episodes"]),
        creators,
        certification: certification(d, kind, region),
        trailer: pick_trailer(&d["videos"]),
        cast: items(&d["credits"]["cast"])
            .iter()
            .take(18)
            .map(|c| Person {
                id: c["id"].as_i64().unwrap_or_default(),
                name: first_text(&[&c["name"]]),
                role: first_text(&[&c["character"]]),
                profile: img(img_base, "w185", &c["profile_path"]),
            })
            .collect(),
        seasons: items(&d["seasons"])
            .iter()
            .filter(|s| s["season_number"].as_i64().is_some_and(|n| n > 0))
            .map(|s| SeasonSummary {
                number: s["season_number"].as_i64().unwrap_or_default(),
                name: first_text(&[&s["name"]]),
                overview: first_text(&[&s["overview"]]),
                poster: img(img_base, "w342", &s["poster_path"]),
                air_date: first_text(&[&s["air_date"]]),
                episode_count: int(&s["episode_count"]).unwrap_or(0),
            })
            .collect(),
        recommendations: map_list(img_base, &d["recommendations"]["results"], Some(kind)),
        similar: map_list(img_base, &d["similar"]["results"], Some(kind)),
    }
}

/// Saison d'une série, avec ses épisodes.
pub fn map_season(img_base: &str, d: &Value) -> Season {
    Season {
        number: d["season_number"].as_i64(),
        name: first_text(&[&d["name"]]),
        overview: first_text(&[&d["overview"]]),
        episodes: items(&d["episodes"])
            .iter()
            .map(|e| TmdbEpisode {
                number: e["episode_number"].as_i64().unwrap_or_default(),
                name: first_text(&[&e["name"]]),
                overview: first_text(&[&e["overview"]]),
                still: img(img_base, "w300", &e["still_path"]),
                air_date: first_text(&[&e["air_date"]]),
                runtime: int(&e["runtime"]),
                rating: round1(&e["vote_average"]),
            })
            .collect(),
    }
}

// ─── Requêtes (fonctions pures) ─────────────────────────────────────────────────────────────────────────────────

/// Liste demandée par l'interface : chemin TMDB, type imposé et paramètres en plus.
///
/// Sources : `trending`, `movie/popular` (et les autres listes `movie/…`, `tv/…`), `discover/{movie|tv}/{genre}`.
pub fn parse_source(source: &str) -> Result<(String, Option<&'static str>, Params), MediaError> {
    let mut parts = source.split('/');
    let kind = parts.next().unwrap_or_default();
    let a = parts.next().unwrap_or_default();
    let b = parts.next().unwrap_or_default();
    let typed = |t: &str| match t {
        "movie" => Some("movie"),
        "tv" => Some("tv"),
        _ => None,
    };
    if kind == "trending" {
        return Ok(("/trending/all/week".into(), None, Vec::new()));
    }
    if kind == "discover"
        && let Some(t) = typed(a)
    {
        let extra = vec![
            ("with_genres", b.to_string()),
            ("sort_by", "popularity.desc".into()),
            ("vote_count.gte", "30".into()),
            ("include_adult", "false".into()),
        ];
        return Ok((format!("/discover/{t}"), Some(t), extra));
    }
    if let Some(t) = typed(kind)
        && !a.is_empty()
        && a.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')
    {
        return Ok((format!("/{t}/{a}"), Some(t), Vec::new()));
    }
    Err(MediaError::unknown_source(source))
}

/// Clé du cache : le chemin et les paramètres triés (même clé que la version d'origine).
pub fn cache_key(path: &str, params: &Params) -> String {
    let mut sorted: Vec<&(&str, String)> = params.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let query: Vec<String> = sorted.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("tmdb:{path}?{}", query.join("&"))
}

/// Requête TMDB. Un « jeton d'accès » v4 est un JWT (`eyJ…`) : il passe en en-tête, pas en paramètre.
pub fn build_request(base: &str, path: &str, params: &Params, key: &str) -> Result<HttpRequest, MediaError> {
    if key.is_empty() {
        return Err(MediaError::no_key());
    }
    let mut url = url::Url::parse(&format!("{base}{path}"))
        .map_err(|e| MediaError::new(ErrorCode::Http, format!("Adresse TMDB invalide : {e}")))?;
    let mut headers = vec![("accept", "application/json".to_string())];
    {
        let mut q = url.query_pairs_mut();
        if key.starts_with("eyJ") {
            headers.push(("authorization", format!("Bearer {key}")));
        } else {
            q.append_pair("api_key", key);
        }
        for (k, v) in params {
            if !v.is_empty() {
                q.append_pair(k, v);
            }
        }
    }
    let url = url.to_string().trim_end_matches('?').to_string();
    Ok(HttpRequest { method: Method::Get, url, headers, body: None })
}

/// Identifiant TMDB : des chiffres seulement (il finit dans le chemin de la requête).
fn check_id(id: &str) -> Result<&str, MediaError> {
    if !id.is_empty() && id.bytes().all(|c| c.is_ascii_digit()) {
        Ok(id)
    } else {
        Err(MediaError::new(ErrorCode::NotFound, "Identifiant invalide"))
    }
}

// ─── Client ─────────────────────────────────────────────────────────────────────────────────────────────────────

pub struct Tmdb {
    base: String,
    img: String,
    cache: Arc<Cache>,
    fetch: Arc<dyn Fetch>,
}

impl Tmdb {
    pub fn new(base: &str, img: &str, cache: Arc<Cache>, fetch: Arc<dyn Fetch>) -> Tmdb {
        Tmdb { base: base.trim_end_matches('/').to_string(), img: img.trim_end_matches('/').to_string(), cache, fetch }
    }

    fn request(&self, key: &str, path: &str, params: &Params) -> Result<Value, MediaError> {
        self.fetch.fetch(&build_request(&self.base, path, params, key)?)
    }

    fn memo<T>(
        &self,
        key: &str,
        path: &str,
        params: Params,
        ttl: i64,
        map: impl FnOnce(&Value) -> T,
    ) -> Result<T, MediaError>
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        // Comme l'original : une réponse encore fraîche sert même si la clé a été retirée entre-temps.
        self.cache.memo(&cache_key(path, &params), ttl, || Ok(map(&self.request(key, path, &params)?)))
    }

    /// Teste une clé sans la mémoriser.
    pub fn test_key(&self, key: &str) -> Result<bool, MediaError> {
        self.request(key.trim(), "/authentication", &Vec::new())?;
        Ok(true)
    }

    pub fn list(&self, key: &str, source: &str, page: i64, lang: Lang) -> Result<Page<TmdbItem>, MediaError> {
        let (path, forced, extra) = parse_source(source)?;
        let mut params = vec![("language", lang.language().to_string()), ("page", page.to_string())];
        params.extend(extra);
        let img = self.img.clone();
        self.memo(key, &path, params, TTL_LIST, move |d| map_page(&img, d, forced, page))
    }

    /// Genres de films (`movie`) ou de séries (`tv`).
    pub fn genres(&self, key: &str, kind: &str, lang: Lang) -> Result<Vec<Genre>, MediaError> {
        let path = match kind {
            "movie" | "tv" => format!("/genre/{kind}/list"),
            _ => return Err(MediaError::new(ErrorCode::Http, format!("Type inconnu : {kind}"))),
        };
        self.memo(key, &path, vec![("language", lang.language().to_string())], TTL_GENRES, map_genres)
    }

    pub fn search(&self, key: &str, query: &str, lang: Lang) -> Result<Vec<TmdbItem>, MediaError> {
        let params = vec![
            ("language", lang.language().to_string()),
            ("query", query.trim().to_string()),
            ("include_adult", "false".into()),
            ("page", "1".into()),
        ];
        self.memo(key, "/search/multi", params, TTL_SEARCH, |d| map_list(&self.img, &d["results"], None))
    }

    pub fn details(&self, key: &str, kind: &str, id: &str, lang: Lang) -> Result<TmdbDetails, MediaError> {
        let id = check_id(id)?;
        let ratings = if kind == "movie" { "release_dates" } else { "content_ratings" };
        let params = vec![
            ("language", lang.language().to_string()),
            ("append_to_response", format!("credits,videos,recommendations,similar,{ratings}")),
            ("include_video_language", format!("{},en,null", lang.code())),
        ];
        self.memo(key, &format!("/{kind}/{id}"), params, TTL_DETAILS, |d| {
            map_details(&self.img, d, kind, lang.region())
        })
    }

    pub fn season(&self, key: &str, id: &str, number: i64, lang: Lang) -> Result<Season, MediaError> {
        let id = check_id(id)?;
        let params = vec![("language", lang.language().to_string())];
        self.memo(key, &format!("/tv/{id}/season/{number}"), params, TTL_SEASON, |d| map_season(&self.img, d))
    }
}

#[cfg(test)]
mod tests;

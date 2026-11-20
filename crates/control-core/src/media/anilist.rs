//! AniList : catalogue d'animés (API GraphQL publique, sans clé).
//!
//! Requêtes et normalisation en fonctions pures ; [`AniList`] ajoute le transport et le cache.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use regex::Regex;
use serde_json::{Value, json};

use super::cache::{Cache, DAY, HOUR, MINUTE};
use super::error::{ErrorCode, MediaError};
use super::http::{Fetch, HttpRequest, Method};
use super::types::{
    AnimeDetails, AnimeEpisode, AnimeItem, NextAiring, Page, Person, RelatedAnime, Shelf, first_text, id_string, int,
    items, text,
};

pub const DEFAULT_URL: &str = "https://graphql.anilist.co";

const TTL_LIST: i64 = 30 * MINUTE;
const TTL_DETAILS: i64 = 6 * HOUR;
const TTL_SEARCH: i64 = 10 * MINUTE;
const TTL_GENRES: i64 = 7 * DAY;

const FRAGMENT: &str = "
fragment M on Media {
  id idMal
  title { romaji english native }
  coverImage { extraLarge large color }
  bannerImage
  description(asHtml: false)
  episodes format status season seasonYear averageScore genres
  startDate { year }
  nextAiringEpisode { episode airingAt }
}";

/// Listes nommées : leurs arguments GraphQL.
fn named(kind: &str) -> Option<&'static str> {
    match kind {
        "trending" => Some("sort: TRENDING_DESC"),
        "popular" => Some("sort: POPULARITY_DESC"),
        "top" => Some("sort: SCORE_DESC"),
        "movies" => Some("format: MOVIE, sort: POPULARITY_DESC"),
        _ => None,
    }
}

/// Rangées de l'accueil, dans l'ordre.
const HOME_SHELVES: [&str; 5] = ["trending", "season", "popular", "top", "movies"];

/// Titres liés gardés, par ordre de priorité (les séries très longues en ont des dizaines).
const RELATION_ORDER: [&str; 7] = ["PREQUEL", "SEQUEL", "PARENT", "SUMMARY", "SIDE_STORY", "SPIN_OFF", "ALTERNATIVE"];

/// Langue des titres (réglage `media.animeTitles`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitlePref {
    Romaji,
    English,
}

impl TitlePref {
    pub fn parse(s: &str) -> TitlePref {
        if s == "english" { TitlePref::English } else { TitlePref::Romaji }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TitlePref::Romaji => "romaji",
            TitlePref::English => "english",
        }
    }
}

// ─── Normalisation (fonctions pures) ────────────────────────────────────────────────────────────────────────────

static BR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>").expect("regex constante"));
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").expect("regex constante"));
static ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&(amp|lt|gt|quot|#039|nbsp);").expect("regex constante"));
static SOURCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\(Source:[^)]*\)").expect("regex constante"));
static BLANKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").expect("regex constante"));
static EPISODE_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:Episode|Épisode)\s+(\d+)\s*[-–:]\s*(.*)$").expect("regex constante"));

/// Résumé AniList en texte brut : sauts de ligne gardés, balises, entités et mention « (Source: …) » retirées.
pub fn clean(html: &str) -> String {
    let s = BR.replace_all(html, "\n");
    let s = TAG.replace_all(&s, "");
    let s = ENTITY.replace_all(&s, |c: &regex::Captures<'_>| match &c[1] {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "#039" => "'",
        _ => " ",
    });
    let s = SOURCE.replace_all(&s, "");
    let s = BLANKS.replace_all(&s, "\n\n");
    s.trim().to_string()
}

/// Saison d'animés d'un mois (0 = janvier).
pub fn season_of(month0: u8) -> &'static str {
    match month0 {
        0..=2 => "WINTER",
        3..=5 => "SPRING",
        6..=8 => "SUMMER",
        _ => "FALL",
    }
}

/// Saison en cours (heure locale).
pub fn current_season() -> (&'static str, i32) {
    let now = crate::util::local_datetime(crate::util::now_ms());
    (season_of(u8::from(now.month()) - 1), now.year())
}

pub fn pick_title(t: &Value, pref: TitlePref) -> String {
    match pref {
        TitlePref::English => first_text(&[&t["english"], &t["romaji"], &t["native"]]),
        TitlePref::Romaji => first_text(&[&t["romaji"], &t["english"], &t["native"]]),
    }
}

/// Nombre d'épisodes déjà sortis.
pub fn available_episodes(m: &Value) -> Option<i64> {
    match int(&m["nextAiringEpisode"]["episode"]) {
        Some(next) => Some((next - 1).max(0)),
        None => int(&m["episodes"]),
    }
}

pub fn map_anime(m: &Value, pref: TitlePref) -> Option<AnimeItem> {
    if !m.is_object() {
        return None;
    }
    let t = &m["title"];
    let id = id_string(&m["id"]);
    Some(AnimeItem {
        key: format!("anime:{id}"),
        kind: "anime".into(),
        id,
        title: pick_title(t, pref),
        original: first_text(&[&t["native"]]),
        overview: clean(m["description"].as_str().unwrap_or_default()),
        poster: text(&m["coverImage"]["extraLarge"]).or_else(|| text(&m["coverImage"]["large"])).map(str::to_string),
        backdrop: text(&m["bannerImage"]).map(str::to_string),
        year: int(&m["seasonYear"]).or_else(|| int(&m["startDate"]["year"])),
        rating: int(&m["averageScore"]).map(|s| s as f64 / 10.0),
        genres: items(&m["genres"]).iter().filter_map(Value::as_str).map(str::to_string).collect(),
        format: first_text(&[&m["format"]]),
        status: first_text(&[&m["status"]]),
        episodes: int(&m["episodes"]),
        available: available_episodes(m),
        color: text(&m["coverImage"]["color"]).map(str::to_string),
        id_mal: int(&m["idMal"]),
    })
}

/// Animés d'une liste, sans ceux qui n'ont pas d'affiche.
pub fn map_media_list(list: &Value, pref: TitlePref) -> Vec<AnimeItem> {
    items(list).iter().filter_map(|m| map_anime(m, pref)).filter(|a| a.poster.is_some()).collect()
}

/// Rangées de l'accueil (réponse de [`home_query`]).
pub fn map_home(d: &Value, pref: TitlePref) -> Vec<Shelf<AnimeItem>> {
    HOME_SHELVES
        .iter()
        .map(|id| Shelf {
            id: id.to_string(),
            source: format!("anime:{id}"),
            items: map_media_list(&d[*id]["media"], pref),
        })
        .collect()
}

/// Une page de liste (`Page` de la réponse de [`list_query`]).
pub fn map_page(p: &Value, pref: TitlePref, page: i64) -> Page<AnimeItem> {
    Page {
        items: map_media_list(&p["media"], pref),
        page: int(&p["pageInfo"]["currentPage"]).unwrap_or(page),
        total_pages: int(&p["pageInfo"]["lastPage"]).unwrap_or(1),
    }
}

/// Genres, sans le genre réservé aux adultes.
pub fn map_genres(d: &Value) -> Vec<String> {
    items(&d["GenreCollection"])
        .iter()
        .filter_map(Value::as_str)
        .filter(|g| *g != "Hentai")
        .map(str::to_string)
        .collect()
}

/// Fiche complète (champ `Media` de la réponse de [`details_query`]).
pub fn map_details(m: &Value, pref: TitlePref) -> Result<AnimeDetails, MediaError> {
    let base = map_anime(m, pref).ok_or_else(|| MediaError::new(ErrorCode::NotFound, "Animé introuvable"))?;
    let is_movie = m["format"] == "MOVIE";
    let streaming = items(&m["streamingEpisodes"]);
    let mut count = base.available.unwrap_or(0);
    if count == 0 && !is_movie {
        count = if streaming.is_empty() { 12 } else { streaming.len() as i64 };
    }

    // Titres et vignettes des épisodes, quand le service de streaming les donne (« Episode 3 - Titre »).
    let mut named: HashMap<i64, (String, Option<String>)> = HashMap::new();
    for e in streaming {
        if let Some(hit) = EPISODE_TITLE.captures(e["title"].as_str().unwrap_or_default())
            && let Ok(n) = hit[1].parse::<i64>()
        {
            named.insert(n, (hit[2].to_string(), text(&e["thumbnail"]).map(str::to_string)));
        }
    }
    let episodes = if is_movie {
        Vec::new()
    } else {
        (1..=count)
            .map(|n| {
                let (name, still) = named.get(&n).cloned().unwrap_or_default();
                AnimeEpisode { number: n, name, still }
            })
            .collect()
    };

    let rank = |r: &str| RELATION_ORDER.iter().position(|x| *x == r);
    let mut edges: Vec<(usize, &Value)> = items(&m["relations"]["edges"])
        .iter()
        .filter(|e| e["node"]["type"] == "ANIME")
        .filter_map(|e| rank(e["relationType"].as_str().unwrap_or_default()).map(|r| (r, e)))
        .collect();
    edges.sort_by_key(|(r, _)| *r);
    let related = edges
        .into_iter()
        .filter_map(|(_, e)| {
            let item = map_anime(&e["node"], pref)?;
            item.poster.as_ref()?;
            Some(RelatedAnime { relation: e["relationType"].as_str().unwrap_or_default().to_string(), item })
        })
        .take(14)
        .collect();

    let next = &m["nextAiringEpisode"];
    Ok(AnimeDetails {
        key: base.key,
        kind: base.kind,
        id: base.id,
        title: base.title,
        original: base.original,
        overview: base.overview,
        poster: base.poster,
        backdrop: base.backdrop,
        year: base.year,
        rating: base.rating,
        genres: base.genres,
        format: base.format,
        status: base.status,
        available: base.available,
        color: base.color,
        id_mal: base.id_mal,
        tagline: String::new(),
        runtime: int(&m["duration"]),
        is_movie,
        studios: items(&m["studios"]["nodes"]).iter().filter_map(|s| s["name"].as_str()).map(str::to_string).collect(),
        trailer: if m["trailer"]["site"] == "youtube" { text(&m["trailer"]["id"]).map(str::to_string) } else { None },
        cast: items(&m["characters"]["edges"])
            .iter()
            .map(|e| Person {
                id: e["node"]["id"].as_i64().unwrap_or_default(),
                name: first_text(&[&e["node"]["name"]["full"]]),
                role: if e["role"] == "MAIN" { "Principal" } else { "Secondaire" }.to_string(),
                profile: text(&e["node"]["image"]["medium"]).map(str::to_string),
            })
            .collect(),
        episodes,
        related,
        recommendations: items(&m["recommendations"]["nodes"])
            .iter()
            .filter_map(|n| map_anime(&n["mediaRecommendation"], pref))
            .filter(|a| a.poster.is_some())
            .collect(),
        next_airing: next.is_object().then(|| NextAiring {
            episode: next["episode"].as_i64().unwrap_or_default(),
            at: next["airingAt"].as_i64().unwrap_or_default() * 1000,
        }),
    })
}

// ─── Requêtes (fonctions pures) ─────────────────────────────────────────────────────────────────────────────────

/// Les rangées de l'accueil en une seule requête (AniList limite à 90 requêtes par minute).
pub fn home_query() -> String {
    let page = |alias: &str, args: &str| {
        format!("{alias}: Page(perPage: 20) {{ media(type: ANIME, isAdult: false, {args}) {{ ...M }} }}")
    };
    format!(
        "query Home($season: MediaSeason, $year: Int) {{\n  {}\n  {}\n  {}\n  {}\n  {}\n}} {FRAGMENT}",
        page("trending", "sort: TRENDING_DESC"),
        page("season", "season: $season, seasonYear: $year, sort: POPULARITY_DESC"),
        page("popular", "sort: POPULARITY_DESC"),
        page("top", "sort: SCORE_DESC"),
        page("movies", "format: MOVIE, sort: POPULARITY_DESC"),
    )
}

/// Requête et variables d'une liste : `trending`, `popular`, `top`, `movies`, `season` ou `genre:{nom}`.
pub fn list_query(source: &str, page: i64, season: (&str, i32)) -> Result<(String, Value), MediaError> {
    let (kind, arg) = source.split_once(':').unwrap_or((source, ""));
    let mut vars = json!({ "page": page });
    let (declare, args) = match kind {
        "season" => {
            vars["season"] = json!(season.0);
            vars["year"] = json!(season.1);
            (
                "$page: Int, $season: MediaSeason, $year: Int",
                "season: $season, seasonYear: $year, sort: POPULARITY_DESC",
            )
        }
        "genre" => {
            vars["genre"] = json!(arg);
            ("$page: Int, $genre: String", "genre: $genre, sort: POPULARITY_DESC")
        }
        _ => ("$page: Int", named(kind).ok_or_else(|| MediaError::unknown_source(source))?),
    };
    let query = format!(
        "query List({declare}) {{\n  Page(page: $page, perPage: 24) {{\n    pageInfo {{ currentPage lastPage hasNextPage }}\n    \
         media(type: ANIME, isAdult: false, {args}) {{ ...M }}\n  }}\n}} {FRAGMENT}"
    );
    Ok((query, vars))
}

pub fn search_query() -> String {
    format!(
        "query Search($q: String) {{\n  Page(perPage: 20) {{ media(type: ANIME, isAdult: false, search: $q, sort: SEARCH_MATCH) {{ ...M }} }}\n}} {FRAGMENT}"
    )
}

const DETAILS_BODY: &str = "query Details($id: Int) {
  Media(id: $id, type: ANIME) {
    ...M
    duration
    trailer { id site }
    studios(isMain: true) { nodes { name } }
    characters(sort: [ROLE, RELEVANCE], perPage: 14) {
      edges { role node { id name { full } image { medium } } }
    }
    streamingEpisodes { title thumbnail }
    relations { edges { relationType(version: 2) node { ...M type } } }
    recommendations(sort: RATING_DESC, perPage: 14) { nodes { mediaRecommendation { ...M } } }
  }
}";

pub fn details_query() -> String {
    format!("{DETAILS_BODY} {FRAGMENT}")
}

pub fn build_request(url: &str, query: &str, variables: &Value) -> HttpRequest {
    HttpRequest {
        method: Method::Post,
        url: url.to_string(),
        headers: vec![("content-type", "application/json".into()), ("accept", "application/json".into())],
        body: Some(json!({ "query": query, "variables": variables }).to_string()),
    }
}

/// Le champ `data` d'une réponse GraphQL, ou sa première erreur.
pub fn unwrap_data(response: Value) -> Result<Value, MediaError> {
    if let Some(first) = response["errors"].as_array().and_then(|e| e.first()) {
        return Err(MediaError::new(ErrorCode::Http, first["message"].as_str().unwrap_or("Erreur AniList")));
    }
    Ok(match response {
        Value::Object(mut m) => m.remove("data").unwrap_or(Value::Null),
        _ => Value::Null,
    })
}

// ─── Client ─────────────────────────────────────────────────────────────────────────────────────────────────────

pub struct AniList {
    url: String,
    cache: Arc<Cache>,
    fetch: Arc<dyn Fetch>,
}

impl AniList {
    pub fn new(url: &str, cache: Arc<Cache>, fetch: Arc<dyn Fetch>) -> AniList {
        AniList { url: url.to_string(), cache, fetch }
    }

    fn query(&self, query: &str, variables: &Value) -> Result<Value, MediaError> {
        unwrap_data(self.fetch.fetch(&build_request(&self.url, query, variables))?)
    }

    fn memo<T>(
        &self,
        pref: TitlePref,
        key: &str,
        ttl: i64,
        f: impl FnOnce() -> Result<T, MediaError>,
    ) -> Result<T, MediaError>
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        self.cache.memo(&format!("anilist:{}:{key}", pref.as_str()), ttl, f)
    }

    /// Accueil : `{ shelves }` (les genres sont demandés à part).
    pub fn home(&self, pref: TitlePref) -> Result<Vec<Shelf<AnimeItem>>, MediaError> {
        let (season, year) = current_season();
        #[derive(serde::Serialize, serde::Deserialize, Default)]
        #[serde(default)]
        struct Cached {
            shelves: Vec<Shelf<AnimeItem>>,
        }
        let cached = self.memo(pref, &format!("home:{season}{year}"), TTL_LIST, || {
            let d = self.query(&home_query(), &json!({ "season": season, "year": year }))?;
            Ok(Cached { shelves: map_home(&d, pref) })
        })?;
        Ok(cached.shelves)
    }

    pub fn list(&self, source: &str, page: i64, pref: TitlePref) -> Result<Page<AnimeItem>, MediaError> {
        let (query, vars) = list_query(source, page, current_season())?;
        self.memo(pref, &format!("list:{source}:{page}"), TTL_LIST, || {
            Ok(map_page(&self.query(&query, &vars)?["Page"], pref, page))
        })
    }

    pub fn genres(&self, pref: TitlePref) -> Result<Vec<String>, MediaError> {
        self.memo(pref, "genres", TTL_GENRES, || Ok(map_genres(&self.query("{ GenreCollection }", &json!({}))?)))
    }

    pub fn search(&self, query: &str, pref: TitlePref) -> Result<Vec<AnimeItem>, MediaError> {
        let text = query.trim();
        self.memo(pref, &format!("search:{}", text.to_lowercase()), TTL_SEARCH, || {
            let d = self.query(&search_query(), &json!({ "q": text }))?;
            Ok(map_media_list(&d["Page"]["media"], pref))
        })
    }

    pub fn details(&self, id: &str, pref: TitlePref) -> Result<AnimeDetails, MediaError> {
        let id: i64 = id.trim().parse().map_err(|_| MediaError::new(ErrorCode::NotFound, "Identifiant invalide"))?;
        self.memo(pref, &format!("details:{id}"), TTL_DETAILS, || {
            map_details(&self.query(&details_query(), &json!({ "id": id }))?["Media"], pref)
        })
    }
}

#[cfg(test)]
mod tests;

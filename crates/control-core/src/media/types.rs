//! Données du catalogue, aux noms JSON (camelCase) de la version d'origine : le serveur web les envoie telles
//! quelles, et le cache les garde sous cette forme. Les champs absents prennent une valeur vide à la lecture, pour
//! relire sans erreur un cache écrit par l'ancienne version.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Film ou série TMDB, dans une rangée ou une liste.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TmdbItem {
    /// `movie:123`.
    pub key: String,
    /// `movie` ou `tv`.
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub title: String,
    pub original: String,
    pub overview: String,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub year: Option<i64>,
    /// Note sur 10, à une décimale.
    pub rating: Option<f64>,
    pub genre_ids: Vec<i64>,
}

/// Animé AniList.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AnimeItem {
    /// `anime:123`.
    pub key: String,
    /// Toujours `anime`.
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub title: String,
    pub original: String,
    pub overview: String,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub year: Option<i64>,
    pub rating: Option<f64>,
    pub genres: Vec<String>,
    /// `TV`, `MOVIE`, `OVA`…
    pub format: String,
    pub status: String,
    /// Nombre d'épisodes prévus.
    pub episodes: Option<i64>,
    /// Épisodes déjà sortis.
    pub available: Option<i64>,
    pub color: Option<String>,
    pub id_mal: Option<i64>,
}

/// Une affiche, quel que soit le catalogue.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CatalogItem {
    Tmdb(TmdbItem),
    Anime(AnimeItem),
}

/// Une page de liste.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub page: i64,
    pub total_pages: i64,
}

impl<T> Page<T> {
    pub fn map<U>(self, f: impl FnMut(T) -> U) -> Page<U> {
        Page { items: self.items.into_iter().map(f).collect(), page: self.page, total_pages: self.total_pages }
    }
}

/// Une rangée de l'accueil. `source` se passe tel quel à [`crate::media::MediaService::list`] (« Tout voir »).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Shelf<T> {
    pub id: String,
    pub source: String,
    pub items: Vec<T>,
}

/// Genre : identifiant numérique chez TMDB, nom chez AniList.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Genre {
    pub id: Value,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct Home {
    pub shelves: Vec<Shelf<CatalogItem>>,
    pub genres: Vec<Genre>,
}

/// Acteur (TMDB) ou personnage (AniList).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Person {
    pub id: i64,
    pub name: String,
    pub role: String,
    pub profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SeasonSummary {
    pub number: i64,
    pub name: String,
    pub overview: String,
    pub poster: Option<String>,
    pub air_date: String,
    pub episode_count: i64,
}

/// Fiche TMDB complète.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TmdbDetails {
    #[serde(flatten)]
    pub item: TmdbItem,
    pub tagline: String,
    /// Minutes (d'un épisode pour une série).
    pub runtime: Option<i64>,
    pub genres: Vec<String>,
    pub status: String,
    pub release_date: String,
    pub number_of_seasons: Option<i64>,
    pub number_of_episodes: Option<i64>,
    /// Réalisateurs (films) ou créateurs (séries), trois au plus.
    pub creators: Vec<String>,
    pub certification: Option<String>,
    /// Identifiant YouTube.
    pub trailer: Option<String>,
    pub cast: Vec<Person>,
    pub seasons: Vec<SeasonSummary>,
    pub recommendations: Vec<TmdbItem>,
    pub similar: Vec<TmdbItem>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TmdbEpisode {
    pub number: i64,
    pub name: String,
    pub overview: String,
    pub still: Option<String>,
    pub air_date: String,
    pub runtime: Option<i64>,
    pub rating: Option<f64>,
}

/// Saison d'une série TMDB, avec ses épisodes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Season {
    pub number: Option<i64>,
    pub name: String,
    pub overview: String,
    pub episodes: Vec<TmdbEpisode>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AnimeEpisode {
    pub number: i64,
    pub name: String,
    pub still: Option<String>,
}

/// Titre lié à un animé (suite, préquelle…).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RelatedAnime {
    /// `PREQUEL`, `SEQUEL`…
    pub relation: String,
    #[serde(flatten)]
    pub item: AnimeItem,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NextAiring {
    pub episode: i64,
    /// Millisecondes depuis l'époque Unix.
    pub at: i64,
}

/// Fiche AniList complète. Reprend les champs d'[`AnimeItem`], sauf `episodes` qui devient la liste des épisodes
/// (comme dans l'original, où la liste remplace le nombre).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AnimeDetails {
    pub key: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub title: String,
    pub original: String,
    pub overview: String,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub year: Option<i64>,
    pub rating: Option<f64>,
    pub genres: Vec<String>,
    pub format: String,
    pub status: String,
    pub available: Option<i64>,
    pub color: Option<String>,
    pub id_mal: Option<i64>,
    pub tagline: String,
    /// Minutes par épisode.
    pub runtime: Option<i64>,
    pub is_movie: bool,
    pub studios: Vec<String>,
    pub trailer: Option<String>,
    pub cast: Vec<Person>,
    pub episodes: Vec<AnimeEpisode>,
    pub related: Vec<RelatedAnime>,
    pub recommendations: Vec<AnimeItem>,
    pub next_airing: Option<NextAiring>,
}

/// Fiche d'un titre, quel que soit le catalogue.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Details {
    Tmdb(Box<TmdbDetails>),
    Anime(Box<AnimeDetails>),
}

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub tmdb: Vec<TmdbItem>,
    pub anime: Vec<AnimeItem>,
    /// Code d'erreur TMDB (`nokey` sans clé), ou `null`.
    pub tmdb_error: Option<String>,
}

// ─── Lecture des réponses JSON, avec la logique « falsy » de JavaScript ─────────────────────────────────────────

/// Texte non vide.
pub(crate) fn text(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| !s.is_empty())
}

/// Premier texte non vide, sinon "".
pub(crate) fn first_text(values: &[&Value]) -> String {
    values.iter().find_map(|v| text(v)).unwrap_or_default().to_string()
}

/// Nombre entier non nul.
pub(crate) fn int(v: &Value) -> Option<i64> {
    let n = v.as_i64().or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f as i64))?;
    (n != 0).then_some(n)
}

/// Nombre non nul.
pub(crate) fn float(v: &Value) -> Option<f64> {
    v.as_f64().filter(|f| *f != 0.0 && f.is_finite())
}

/// Identifiant en texte, comme `String(id)`.
pub(crate) fn id_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Éléments d'un tableau (aucun si ce n'en est pas un).
pub(crate) fn items(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}

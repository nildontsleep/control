//! LRCLIB (https://lrclib.net) : gratuit, sans clé. Construction des adresses, lecture des réponses et choix du
//! meilleur résultat, en fonctions pures ; le transport passe par [`Http`].

use std::time::Duration;

use serde_json::Value;

use super::LyricsError;

pub const DEFAULT_API: &str = "https://lrclib.net/api";
/// LRCLIB demande un agent qui dise qui l'appelle.
pub const USER_AGENT: &str =
    concat!("control/", env!("CARGO_PKG_VERSION"), " (https://github.com/nildontsleep/control)");
const TIMEOUT: Duration = Duration::from_secs(10);

/// Ce que l'on sait du morceau pour le chercher.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Query {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Secondes.
    pub duration: f64,
}

impl Query {
    /// Durée arrondie à la seconde (0 si inconnue), comme `Math.round` d'origine.
    pub fn seconds(&self) -> i64 {
        if self.duration.is_finite() && self.duration > 0.0 { (self.duration + 0.5).floor() as i64 } else { 0 }
    }
}

/// Une fiche de LRCLIB (seuls les champs utiles).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Record {
    pub synced: Option<String>,
    pub plain: Option<String>,
    pub instrumental: bool,
    /// Secondes (0 si absente).
    pub duration: f64,
}

impl Record {
    /// Lecture tolérante : un champ absent, nul ou d'un autre type compte comme vide.
    pub fn from_json(v: &Value) -> Option<Record> {
        let o = v.as_object()?;
        let text = |k: &str| o.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
        Some(Record {
            synced: text("syncedLyrics"),
            plain: text("plainLyrics"),
            instrumental: o.get("instrumental").and_then(Value::as_bool).unwrap_or(false),
            duration: o.get("duration").and_then(Value::as_f64).unwrap_or(0.0),
        })
    }

    pub fn has_lyrics(&self) -> bool {
        self.synced.is_some() || self.plain.is_some() || self.instrumental
    }
}

/// Recherche exacte (`/get`) : seulement avec un artiste et un titre.
pub fn exact_url(api: &str, q: &Query) -> Option<String> {
    if q.artist.is_empty() || q.title.is_empty() {
        return None;
    }
    let mut params = vec![("track_name", q.title.clone()), ("artist_name", q.artist.clone())];
    if !q.album.is_empty() {
        params.push(("album_name", q.album.clone()));
    }
    let s = q.seconds();
    if s != 0 {
        params.push(("duration", s.to_string()));
    }
    Some(with_params(&format!("{api}/get"), &params))
}

/// Recherche large (`/search`) : par titre et artiste, ou par texte libre sans artiste.
pub fn search_url(api: &str, q: &Query) -> String {
    let params = if q.artist.is_empty() {
        vec![("q", q.title.clone())]
    } else {
        vec![("track_name", q.title.clone()), ("artist_name", q.artist.clone())]
    };
    with_params(&format!("{api}/search"), &params)
}

fn with_params(base: &str, params: &[(&str, String)]) -> String {
    let mut out = String::from(base);
    let query = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(params).finish();
    out.push('?');
    out.push_str(&query);
    out
}

/// Réponse de `/get` : la fiche, si c'en est une.
pub fn parse_get(body: &str) -> Result<Option<Record>, serde_json::Error> {
    Ok(Record::from_json(&serde_json::from_str::<Value>(body)?))
}

/// Réponse de `/search` : les fiches lisibles (une réponse qui n'est pas une liste n'en donne aucune).
pub fn parse_search(body: &str) -> Result<Vec<Record>, serde_json::Error> {
    let v: Value = serde_json::from_str(body)?;
    Ok(v.as_array().map(|a| a.iter().filter_map(Record::from_json).collect()).unwrap_or_default())
}

/// Meilleure fiche d'une recherche : même durée à quatre secondes près (si on la connaît), version synchronisée
/// d'abord, puis la durée la plus proche ; la première qui a des paroles (ou se dit instrumentale).
pub fn pick(list: Vec<Record>, seconds: i64) -> Option<Record> {
    let s = seconds as f64;
    let mut close: Vec<Record> = list.into_iter().filter(|r| seconds == 0 || (r.duration - s).abs() <= 4.0).collect();
    // Tri stable, comme `Array.prototype.sort` : à égalité, l'ordre de LRCLIB.
    close.sort_by(|a, b| {
        b.synced
            .is_some()
            .cmp(&a.synced.is_some())
            .then_with(|| (a.duration - s).abs().total_cmp(&(b.duration - s).abs()))
    });
    close.into_iter().find(Record::has_lyrics)
}

/// Transport HTTP (remplaçable dans les tests).
pub trait Http: Send + Sync {
    /// GET : statut et corps de la réponse, ou message d'erreur réseau.
    fn get(&self, url: &str) -> Result<(u16, String), String>;
}

/// Transport réel, par `ureq`.
pub struct UreqHttp {
    agent: ureq::Agent,
}

impl Default for UreqHttp {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .build();
        UreqHttp { agent: config.into() }
    }
}

impl Http for UreqHttp {
    fn get(&self, url: &str) -> Result<(u16, String), String> {
        let mut res = self.agent.get(url).call().map_err(|e| e.to_string())?;
        let status = res.status().as_u16();
        let body = if (200..300).contains(&status) {
            res.body_mut().read_to_string().map_err(|e| e.to_string())?
        } else {
            String::new()
        };
        Ok((status, body))
    }
}

/// GET avec trois essais : LRCLIB répond parfois « serveur occupé » (429, 5xx). 404 : rien trouvé.
pub(super) fn fetch(http: &dyn Http, url: &str, delay: Duration) -> Result<Option<String>, LyricsError> {
    let mut last = LyricsError::Network(String::new());
    for attempt in 0..3u32 {
        if attempt > 0 {
            std::thread::sleep(delay * attempt);
        }
        match http.get(url) {
            Ok((404, _)) => return Ok(None),
            Ok((s, body)) if (200..300).contains(&s) => return Ok(Some(body)),
            Ok((s, _)) if s == 429 || s >= 500 => last = LyricsError::Busy(s),
            Ok((s, _)) => return Err(LyricsError::Http(s)),
            Err(msg) => last = LyricsError::Network(msg),
        }
    }
    Err(last)
}

/// Cherche des paroles : recherche exacte d'abord, puis recherche large.
pub(super) fn lookup(http: &dyn Http, api: &str, q: &Query, delay: Duration) -> Result<Option<Record>, LyricsError> {
    let bad = |e: serde_json::Error| LyricsError::Network(format!("réponse illisible de LRCLIB ({e})"));
    if let Some(url) = exact_url(api, q)
        && let Some(body) = fetch(http, &url, delay)?
        && let Some(hit) = parse_get(&body).map_err(bad)?.filter(Record::has_lyrics)
    {
        return Ok(Some(hit));
    }
    let Some(body) = fetch(http, &search_url(api, q), delay)? else { return Ok(None) };
    Ok(pick(parse_search(&body).map_err(bad)?, q.seconds()))
}

//! Lecture du JSON de `yt-dlp -J` : analyse d'un lien (vidéo ou playlist) et résultats de recherche.

use serde::Serialize;
use serde_json::Value;

/// Contenu de `dl:info` : `{ type: "video" | "playlist", … }`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Info {
    Video(VideoInfo),
    Playlist(PlaylistInfo),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VideoInfo {
    pub title: Option<String>,
    pub uploader: Option<String>,
    pub duration: Option<f64>,
    pub thumbnail: Option<String>,
    /// Un morceau de musique (artiste connu, ou chaîne « … - Topic » de YouTube Music).
    pub music: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlaylistInfo {
    pub title: String,
    pub uploader: Option<String>,
    pub count: u64,
    pub thumbnail: Option<String>,
    /// Les 60 premiers éléments.
    pub entries: Vec<Entry>,
    pub music: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Entry {
    pub title: Option<String>,
    pub duration: Option<f64>,
    pub uploader: Option<String>,
}

/// Un résultat de recherche.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchResult {
    pub title: Option<String>,
    pub uploader: Option<String>,
    pub duration: Option<f64>,
    pub thumbnail: Option<String>,
    pub url: String,
    pub source: String,
}

/// Texte non vide d'un champ (comme `a || b` en JavaScript : le vide ne compte pas).
fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

fn first_text(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| text(v, k))
}

fn duration(v: &Value) -> Option<f64> {
    v.get("duration").and_then(Value::as_f64).filter(|d| *d != 0.0 && d.is_finite())
}

/// Vignette : `thumbnail`, sinon la dernière (la plus grande) de `thumbnails`.
fn thumb(v: &Value) -> Option<String> {
    text(v, "thumbnail").or_else(|| v.get("thumbnails")?.as_array()?.iter().rev().find_map(|t| text(t, "url")))
}

fn is_topic(v: &Value) -> bool {
    first_text(v, &["uploader", "channel"]).is_some_and(|u| u.ends_with("Topic"))
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Titre lisible tiré de l'adresse (« …/sets/city-ports » → « City ports ») quand yt-dlp n'en donne pas.
pub fn title_from_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let last = parsed.path().split('/').rfind(|s| !s.is_empty()).unwrap_or("");
    let decoded = percent_decode(last)?;
    let words: String =
        decoded.split(['-', '_']).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ").trim().to_string();
    let mut chars = words.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().chain(chars).collect())
}

fn entries(data: &Value) -> Vec<&Value> {
    data.get("entries")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter(|e| e.is_object()).collect())
        .unwrap_or_default()
}

/// Analyse d'un lien, à partir du JSON de `yt-dlp -J --flat-playlist`.
pub fn parse_info(data: &Value, url: &str) -> Info {
    if data.get("_type").and_then(Value::as_str) == Some("playlist") {
        let list = entries(data);
        let entry_url = |e: &Value| first_text(e, &["url", "webpage_url"]).unwrap_or_default();
        let count = data.get("playlist_count").and_then(Value::as_u64).filter(|n| *n > 0).unwrap_or(list.len() as u64);
        return Info::Playlist(PlaylistInfo {
            title: text(data, "title").unwrap_or_else(|| "Playlist".into()),
            uploader: first_text(data, &["uploader", "channel"]),
            count,
            thumbnail: thumb(data).or_else(|| list.first().and_then(|e| thumb(e))),
            entries: list
                .iter()
                .take(60)
                .map(|e| Entry {
                    title: text(e, "title").or_else(|| title_from_url(&entry_url(e))),
                    duration: duration(e),
                    uploader: first_text(e, &["uploader", "channel"]),
                })
                .collect(),
            music: list.iter().any(|e| text(e, "artist").is_some()) || list.first().is_some_and(|e| is_topic(e)),
        });
    }
    Info::Video(VideoInfo {
        title: first_text(data, &["track", "title"]).or_else(|| title_from_url(url)),
        uploader: first_text(data, &["artist", "uploader", "channel"]),
        duration: duration(data),
        thumbnail: thumb(data),
        music: text(data, "artist").is_some() || is_topic(data),
    })
}

/// Résultats d'une recherche (`ytsearch…`, `scsearch…`) : seuls ceux qui ont une adresse http(s) sont gardés.
pub fn parse_search(data: &Value, source: &str) -> Vec<SearchResult> {
    entries(data)
        .into_iter()
        .filter_map(|e| {
            let url = first_text(e, &["webpage_url", "url"])?;
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return None;
            }
            Some(SearchResult {
                title: text(e, "title").or_else(|| title_from_url(&url)),
                uploader: first_text(e, &["uploader", "channel", "artist"]),
                duration: duration(e),
                thumbnail: thumb(e),
                url,
                source: source.to_string(),
            })
        })
        .collect()
}

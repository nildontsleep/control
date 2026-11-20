//! Pochettes de l'éditeur : préparation d'une image (fichier, adresse, envoi du navigateur) et recherche en ligne
//! (Apple Music via l'API iTunes Search, et Deezer). L'analyse des réponses est faite par des fonctions pures.

use std::collections::HashSet;
use std::io::{Cursor, Read};
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use base64::Engine;
use image::ImageReader;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::covers::{encode_jpeg, fit};

/// Taille maximale d'une image source.
const MAX_IMAGE: usize = 25 * 1024 * 1024;
/// Côté maximal de la pochette intégrée au fichier.
const COVER_PX: u32 = 1400;
/// Côté maximal de l'aperçu montré dans l'éditeur.
const PREVIEW_PX: u32 = 480;
const USER_AGENT: &str = "control (https://github.com/nildontsleep/control)";
const ITUNES: &str = "https://itunes.apple.com/search";
const DEEZER: &str = "https://api.deezer.com/search/album";

/// D'où vient une pochette : un fichier local, une adresse `https://`, ou une image envoyée par le navigateur
/// (base64, éventuellement en adresse `data:`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CoverSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

/// Pochette préparée : le jeton à renvoyer avec la modification, et un aperçu (adresse `data:` JPEG).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedCover {
    pub token: String,
    pub preview: String,
    /// Dimensions de l'image d'origine.
    pub width: u32,
    pub height: u32,
}

/// Image prête à intégrer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedImage {
    pub data: Vec<u8>,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
    pub preview: String,
}

fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(9))).build())
}

/// Octets de l'image d'une source.
pub(super) fn load(src: &CoverSource) -> anyhow::Result<Vec<u8>> {
    if let Some(data) = src.data.as_deref().filter(|d| !d.is_empty()) {
        let raw = match data.strip_prefix("data:") {
            Some(rest) => rest.split_once(',').map_or(rest, |(_, b)| b),
            None => data,
        };
        return base64::engine::general_purpose::STANDARD
            .decode(raw.trim())
            .map_err(|_| anyhow!("Image envoyée illisible."));
    }
    if let Some(path) = src.path.as_deref().filter(|p| !p.is_empty()) {
        let md = std::fs::metadata(path).context("Image introuvable.")?;
        if !md.is_file() || md.len() as usize > MAX_IMAGE {
            bail!("Image trop volumineuse.");
        }
        return std::fs::read(path).context("Image illisible.");
    }
    if let Some(url) = src.url.as_deref().filter(|u| u.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://"))) {
        let mut res = match agent().get(url).header("user-agent", USER_AGENT).call() {
            Ok(res) => res,
            Err(ureq::Error::StatusCode(code)) => bail!("Téléchargement impossible ({code})."),
            Err(err) => bail!("Téléchargement impossible ({err})."),
        };
        let mut out = Vec::new();
        res.body_mut()
            .as_reader()
            .take(MAX_IMAGE as u64 + 1)
            .read_to_end(&mut out)
            .map_err(|err| anyhow!("Téléchargement impossible ({err})."))?;
        return Ok(out);
    }
    bail!("Source de pochette invalide.")
}

/// Réduit l'image si besoin (1400 px au plus, en JPEG) et prépare un aperçu de 480 px au plus. Un JPEG déjà assez
/// petit est gardé tel quel, sans perte.
pub fn prepare_image(buffer: &[u8]) -> anyhow::Result<PreparedImage> {
    if buffer.is_empty() || buffer.len() > MAX_IMAGE {
        bail!("Image trop volumineuse ou vide.");
    }
    let image = ImageReader::new(Cursor::new(buffer))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.decode().ok())
        .ok_or_else(|| anyhow!("Format d’image non pris en charge."))?;
    let (width, height) = (image.width(), image.height());
    let is_jpeg = buffer.starts_with(&[0xFF, 0xD8]);
    let (data, fitted) = if !is_jpeg || width.max(height) > COVER_PX || buffer.len() > 3 * 1024 * 1024 {
        let fitted = fit(&image, COVER_PX);
        (encode_jpeg(&fitted, 90), fitted)
    } else {
        (buffer.to_vec(), image)
    };
    if data.is_empty() {
        bail!("Format d’image non pris en charge.");
    }
    let small = encode_jpeg(&fit(&fitted, PREVIEW_PX), 82);
    let preview = format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(small));
    Ok(PreparedImage { data, mime: "image/jpeg", width, height, preview })
}

// ─── Recherche en ligne ──────────────────────────────────────────────────────

/// Un album trouvé en ligne.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverHit {
    /// « Apple Music » ou « Deezer ».
    pub source: String,
    pub title: String,
    pub artist: String,
    pub year: Option<u32>,
    pub genre: String,
    pub tracks: Option<u32>,
    /// Image de 300 px environ, pour la grille.
    pub preview: String,
    /// Grande image, à passer à `stage_cover`.
    pub full: String,
}

static ARTWORK_SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+x\d+bb").expect("regex constante"));

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn count_of(v: &Value, key: &str) -> Option<u32> {
    v.get(key).and_then(Value::as_u64).filter(|n| *n > 0).map(|n| n.min(u32::MAX as u64) as u32)
}

/// Résultats de `itunes.apple.com/search?entity=album`.
pub fn parse_itunes(json: &Value) -> Vec<CoverHit> {
    let Some(results) = json.get("results").and_then(Value::as_array) else { return Vec::new() };
    results
        .iter()
        .filter_map(|r| {
            let art = str_of(r, "artworkUrl100");
            if art.is_empty() {
                return None;
            }
            let year = str_of(r, "releaseDate").get(..4).and_then(|y| y.parse::<u32>().ok()).filter(|y| *y > 0);
            Some(CoverHit {
                source: "Apple Music".into(),
                title: str_of(r, "collectionName").into(),
                artist: str_of(r, "artistName").into(),
                year,
                genre: str_of(r, "primaryGenreName").into(),
                tracks: count_of(r, "trackCount"),
                preview: ARTWORK_SIZE.replace(art, "300x300bb").into_owned(),
                full: ARTWORK_SIZE.replace(art, "1200x1200bb").into_owned(),
            })
        })
        .collect()
}

/// Résultats de `api.deezer.com/search/album`.
pub fn parse_deezer(json: &Value) -> Vec<CoverHit> {
    let Some(data) = json.get("data").and_then(Value::as_array) else { return Vec::new() };
    data.iter()
        .filter_map(|r| {
            let pick = |keys: &[&str]| keys.iter().map(|k| str_of(r, k)).find(|s| !s.is_empty()).map(str::to_string);
            let full = pick(&["cover_xl", "cover_big"])?;
            Some(CoverHit {
                source: "Deezer".into(),
                title: str_of(r, "title").into(),
                artist: r.get("artist").map(|a| str_of(a, "name")).unwrap_or("").into(),
                year: None,
                genre: String::new(),
                tracks: count_of(r, "nb_tracks"),
                preview: pick(&["cover_medium", "cover_big"]).unwrap_or_default(),
                full,
            })
        })
        .collect()
}

/// Les deux services en alternance, sans doublon évident (même titre et même artiste), 16 au plus.
pub fn merge_hits(apple: Vec<CoverHit>, deezer: Vec<CoverHit>) -> Vec<CoverHit> {
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    let mut a = apple.into_iter();
    let mut d = deezer.into_iter();
    loop {
        let (x, y) = (a.next(), d.next());
        if x.is_none() && y.is_none() {
            break;
        }
        for item in [x, y].into_iter().flatten() {
            if seen.insert(format!("{}|{}", item.title, item.artist).to_lowercase()) {
                merged.push(item);
            }
        }
    }
    merged.truncate(16);
    merged
}

fn get_json(agent: &ureq::Agent, url: &str, query: &[(&str, &str)]) -> anyhow::Result<Value> {
    let mut req = agent.get(url).header("user-agent", USER_AGENT).header("accept", "application/json");
    for (k, v) in query {
        req = req.query(*k, *v);
    }
    Ok(req.call()?.body_mut().read_json::<Value>()?)
}

/// Interroge les deux services en parallèle. L'adresse de chacun peut être remplacée par `CONTROL_ITUNES_URL` et
/// `CONTROL_DEEZER_URL` (tests de bout en bout).
pub(super) fn search(artist: &str, album: &str) -> anyhow::Result<Vec<CoverHit>> {
    let query = format!("{artist} {album}").split_whitespace().collect::<Vec<_>>().join(" ");
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let itunes_url = std::env::var("CONTROL_ITUNES_URL").unwrap_or_else(|_| ITUNES.to_string());
    let deezer_url = std::env::var("CONTROL_DEEZER_URL").unwrap_or_else(|_| DEEZER.to_string());
    let agent = agent();
    let (apple, deezer) = std::thread::scope(|s| {
        let apple = s.spawn(|| {
            get_json(&agent, &itunes_url, &[("media", "music"), ("entity", "album"), ("limit", "12"), ("term", &query)])
        });
        let deezer = s.spawn(|| get_json(&agent, &deezer_url, &[("q", &query), ("limit", "12")]));
        let join = |h: std::thread::ScopedJoinHandle<'_, anyhow::Result<Value>>| {
            h.join().unwrap_or_else(|_| Err(anyhow!("recherche interrompue")))
        };
        (join(apple), join(deezer))
    });
    if let (Err(a), Err(d)) = (&apple, &deezer) {
        log::warn!("Recherche de pochettes impossible : {a} ; {d}");
        bail!("Recherche impossible : vérifiez la connexion.");
    }
    let apple = apple.map(|v| parse_itunes(&v)).unwrap_or_default();
    let deezer = deezer.map(|v| parse_deezer(&v)).unwrap_or_default();
    Ok(merge_hits(apple, deezer))
}

//! Pochettes pour Discord. Discord n'affiche que des images accessibles par une adresse publique (celles de son
//! propre CDN, signées et expirantes, sont refusées) : la pochette d'un fichier local est donc envoyée à un
//! hébergeur temporaire sans compte. On essaie les hébergeurs dans l'ordre jusqu'à ce que l'un réponde, et les
//! adresses obtenues sont gardées dans un petit cache (`discord-covers.json`, même format que l'original).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::activity::CoverRef;
use crate::util::{now_ms, write_atomic};

/// Litterbox garde 72 h : on renvoie un peu avant.
pub const COVER_TTL_MS: i64 = 60 * 3600 * 1000;
/// Après un échec, on attend avant de réessayer la même pochette.
pub const COVER_RETRY_MS: i64 = 2 * 60 * 1000;
const MAX_SIZE: usize = 8 * 1024 * 1024;
const AGENT: &str = "control (https://github.com/nildontsleep/control)";

static CATBOX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https://(?:[\w-]+\.)?catbox\.moe/[\w./-]+$").expect("regex constante"));
static UGUU: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https://(?:[\w-]+\.)?uguu\.se/[\w./%-]+$").expect("regex constante"));
static COVER_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https://(?:[\w-]+\.)?(?:catbox\.moe|uguu\.se)/[\w./%-]+$").expect("regex constante")
});

/// Trouve le fichier image d'une pochette (`None` s'il n'existe pas).
pub type CoverFileFn = Arc<dyn Fn(&CoverRef) -> Option<PathBuf> + Send + Sync>;

/// Type réel d'une image d'après ses premiers octets (une pochette peut être en WebP ou en PNG sous une
/// extension .jpg) : (extension, type MIME).
pub fn sniff_image(buf: &[u8]) -> Option<(&'static str, &'static str)> {
    if buf.len() > 3 && buf.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(("jpg", "image/jpeg"))
    } else if buf.len() > 4 && buf.starts_with(&[0x89, 0x50, 0x4e, 0x47]) {
        Some(("png", "image/png"))
    } else if buf.len() > 12 && buf.starts_with(b"RIFF") && &buf[8..12] == b"WEBP" {
        Some(("webp", "image/webp"))
    } else if buf.len() > 4 && buf.starts_with(b"GIF8") {
        Some(("gif", "image/gif"))
    } else {
        None
    }
}

/// Un hébergeur temporaire.
#[derive(Debug, Clone)]
pub struct Host {
    pub name: &'static str,
    pub url: String,
    pub fields: Vec<(&'static str, &'static str)>,
    /// Nom du champ du fichier.
    pub file: &'static str,
    pub ttl_ms: i64,
    pub parse: fn(&str) -> Option<String>,
}

fn parse_catbox(t: &str) -> Option<String> {
    CATBOX.is_match(t).then(|| t.to_string())
}

fn parse_uguu(t: &str) -> Option<String> {
    let v: Value = serde_json::from_str(t).ok()?;
    let u = v.get("files")?.get(0)?.get("url")?.as_str()?;
    UGUU.is_match(u).then(|| u.to_string())
}

fn parse_any(t: &str) -> Option<String> {
    (t.starts_with("https://") || t.starts_with("http://")).then(|| t.to_string())
}

/// Les hébergeurs, dans l'ordre. `CONTROL_COVER_HOST_URL` les remplace par un seul (essais).
pub fn hosts() -> Vec<Host> {
    if let Ok(url) = std::env::var("CONTROL_COVER_HOST_URL")
        && !url.is_empty()
    {
        return vec![Host {
            name: "essai",
            url,
            fields: vec![("reqtype", "fileupload"), ("time", "72h")],
            file: "fileToUpload",
            ttl_ms: COVER_TTL_MS,
            parse: parse_any,
        }];
    }
    vec![
        Host {
            name: "litterbox",
            url: "https://litterbox.catbox.moe/resources/internals/api.php".into(),
            fields: vec![("reqtype", "fileupload"), ("time", "72h")],
            file: "fileToUpload",
            ttl_ms: COVER_TTL_MS,
            parse: parse_catbox,
        },
        Host {
            name: "uguu",
            url: "https://uguu.se/upload".into(),
            fields: vec![],
            file: "files[]",
            ttl_ms: 50 * 60 * 1000,
            parse: parse_uguu,
        },
    ]
}

/// Corps `multipart/form-data` : champs texte puis le fichier.
pub fn multipart(
    boundary: &str,
    fields: &[(&str, &str)],
    file_field: &str,
    ext: &str,
    mime: &str,
    data: &[u8],
) -> Vec<u8> {
    let mut body = Vec::with_capacity(data.len() + 512);
    for (k, v) in fields {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{file_field}\"; filename=\"cover.{ext}\"\r\nContent-Type: {mime}\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

/// Une adresse obtenue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverEntry {
    pub url: String,
    /// Millisecondes Unix.
    pub at: i64,
    /// Expiration (millisecondes Unix) ; absente des anciens caches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exp: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

impl CoverEntry {
    pub fn valid(&self, now: i64) -> bool {
        now < self.exp.unwrap_or(self.at + COVER_TTL_MS)
    }
}

/// Adresses déjà obtenues et échecs récents.
#[derive(Debug, Default)]
pub struct CoverCache {
    file: Option<PathBuf>,
    entries: HashMap<String, CoverEntry>,
    /// Clé → (instant de l'échec, raison affichée dans les réglages).
    failed: HashMap<String, (i64, String)>,
}

impl CoverCache {
    pub fn load(file: Option<PathBuf>) -> CoverCache {
        let now = now_ms();
        let mut entries = HashMap::new();
        if let Some(f) = &file
            && let Ok(text) = std::fs::read_to_string(f)
            && let Ok(Value::Object(saved)) = serde_json::from_str::<Value>(&text)
        {
            for (key, v) in saved {
                if let Ok(e) = serde_json::from_value::<CoverEntry>(v)
                    && COVER_URL.is_match(&e.url)
                    && e.valid(now)
                {
                    entries.insert(key, e);
                }
            }
        }
        CoverCache { file, entries, failed: HashMap::new() }
    }

    /// Adresse encore valable pour cette clé.
    pub fn hit(&self, key: &str, now: i64) -> Option<&CoverEntry> {
        self.entries.get(key).filter(|e| e.valid(now))
    }

    fn recently_failed(&self, key: &str, now: i64) -> Option<String> {
        self.failed.get(key).filter(|(at, _)| now - at < COVER_RETRY_MS).map(|(_, d)| d.clone())
    }

    pub fn failure(&self, key: &str) -> Option<String> {
        self.failed.get(key).map(|(_, d)| d.clone())
    }

    fn insert(&mut self, key: &str, entry: CoverEntry) {
        self.entries.insert(key.to_string(), entry);
        self.failed.remove(key);
        self.save();
    }

    fn save(&mut self) {
        let Some(file) = self.file.clone() else { return };
        let now = now_ms();
        self.entries.retain(|_, e| e.valid(now));
        let text = serde_json::to_string(&self.entries).unwrap_or_else(|_| "{}".into());
        // Cache facultatif : une erreur d'écriture n'empêche rien.
        let _ = write_atomic(&file, text.as_bytes());
    }
}

/// Résultat de la recherche d'une pochette : l'adresse et l'hébergeur, ou la raison de l'échec.
pub type CoverResult = Result<(String, String), String>;

/// Adresse publique de la pochette : celle déjà obtenue, sinon on envoie le fichier à un hébergeur. Bloquant.
pub fn cover_url(
    cache: &parking_lot::Mutex<CoverCache>,
    cover_file: Option<&CoverFileFn>,
    agent: &ureq::Agent,
    r: &CoverRef,
) -> CoverResult {
    let key = r.key();
    let now = now_ms();
    {
        let c = cache.lock();
        if let Some(e) = c.hit(&key, now) {
            return Ok((e.url.clone(), e.host.clone().unwrap_or_else(|| "hébergeur".into())));
        }
        if let Some(detail) = c.recently_failed(&key, now) {
            return Err(detail);
        }
    }
    match upload(cover_file, agent, r) {
        Ok((url, host, ttl)) => {
            let now = now_ms();
            let entry = CoverEntry {
                url: url.clone(),
                at: now,
                exp: Some(now + ttl - 10 * 60 * 1000),
                host: Some(host.into()),
            };
            cache.lock().insert(&key, entry);
            Ok((url, host.to_string()))
        }
        Err(detail) => {
            // Hors ligne ou hébergeur indisponible : le logo de control reste affiché, et la raison l'est dans les réglages.
            cache.lock().failed.insert(key, (now_ms(), detail.clone()));
            Err(detail)
        }
    }
}

fn flat(s: &str, n: usize) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(n).collect()
}

fn upload(
    cover_file: Option<&CoverFileFn>,
    agent: &ureq::Agent,
    r: &CoverRef,
) -> Result<(String, &'static str, i64), String> {
    let file = cover_file.and_then(|f| f(r)).ok_or_else(|| "fichier de pochette introuvable".to_string())?;
    let data = read_cover(&file)?;
    let (ext, mime) = sniff_image(&data).ok_or_else(|| "format d’image non reconnu".to_string())?;
    let size_note = format!("{ext}, {} Ko", (data.len() as f64 / 1024.0).round().max(1.0) as u64);
    let mut errors = Vec::new();
    for host in hosts() {
        let boundary = format!("----control{}", hex::encode(rand::random::<[u8; 12]>()));
        let body = multipart(&boundary, &host.fields, host.file, ext, mime, &data);
        let attempt = agent
            .post(&host.url)
            .header("content-type", format!("multipart/form-data; boundary={boundary}"))
            .header("user-agent", AGENT)
            .header("accept", "*/*")
            .config()
            .timeout_global(Some(Duration::from_secs(20)))
            .build()
            .send(&body[..]);
        let result = match attempt {
            Ok(mut res) => {
                let status = res.status().as_u16();
                let reply = res.body_mut().read_to_string().unwrap_or_default();
                let reply = reply.trim();
                if !(200..300).contains(&status) {
                    Err(format!("HTTP {status} « {} »", flat(reply, 60)))
                } else {
                    (host.parse)(reply).ok_or_else(|| format!("réponse inattendue « {} »", flat(reply, 60)))
                }
            }
            Err(e) => Err(flat(&e.to_string(), 90)),
        };
        match result {
            Ok(url) => return Ok((url, host.name, host.ttl_ms)),
            Err(e) => errors.push(format!("{} : {}", host.name, flat(&e, 90))),
        }
    }
    Err(format!("{} ({size_note})", errors.join(" ; ")))
}

fn read_cover(file: &Path) -> Result<Vec<u8>, String> {
    let data = std::fs::read(file).map_err(|e| format!("réseau ({})", flat(&e.to_string(), 60)))?;
    if data.is_empty() || data.len() > MAX_SIZE {
        return Err("pochette vide ou trop lourde".into());
    }
    Ok(data)
}

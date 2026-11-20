//! Outils HTTP : cookies, réponses JSON (gzip), fichiers statiques et médias (requêtes Range, ETag), chemins sûrs.
//! Repris de `server/http.js`.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::UNIX_EPOCH;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::Response;
use parking_lot::Mutex;
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};

pub fn mime_of(file: &Path) -> &'static str {
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "m4a" | "m4b" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "mp4" | "m4v" | "mov" => "video/mp4",
        "webm" | "mkv" => "video/webm",
        _ => "application/octet-stream",
    }
}

fn compressible(kind: &str) -> bool {
    kind.starts_with("text/")
        || kind.starts_with("application/json")
        || kind.starts_with("application/manifest+json")
        || kind.starts_with("application/javascript")
        || kind.starts_with("image/svg")
}

/// `decodeURIComponent`, ou `None` si la suite d'octets n'est pas valide.
pub fn decode_component(s: &str) -> Option<String> {
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

/// `encodeURIComponent`.
pub fn encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn parse_cookies(header: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for part in header.split(';') {
        let Some((name, value)) = part.split_once('=') else { continue };
        let name = name.trim();
        if !name.is_empty() {
            out.insert(name.to_string(), decode_component(value.trim()).unwrap_or_else(|| value.trim().to_string()));
        }
    }
    out
}

pub fn cookie_header(name: &str, value: &str, max_age: Option<i64>, secure: bool) -> String {
    let mut parts = vec![format!("{name}={}", encode_component(value)), "Path=/".into(), "SameSite=Lax".into()];
    parts.push("HttpOnly".into());
    if secure {
        parts.push("Secure".into());
    }
    if let Some(age) = max_age {
        parts.push(format!("Max-Age={age}"));
    }
    parts.join("; ")
}

pub fn accepts_gzip(headers: &HeaderMap) -> bool {
    headers
        .get_all("accept-encoding")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| v.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').any(|w| w == "gzip"))
}

pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    if enc.write_all(data).is_err() {
        return Vec::new();
    }
    enc.finish().unwrap_or_default()
}

pub fn gunzip(data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(data), &mut out)?;
    Ok(out)
}

/// Réponse en construction : statut, en-têtes, corps.
pub fn response(status: u16, headers: &[(&str, String)], body: Body) -> Response {
    let mut res = Response::new(body);
    *res.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let h = res.headers_mut();
    for (k, v) in headers {
        if let (Ok(name), Ok(value)) = (HeaderName::try_from(*k), HeaderValue::from_str(v)) {
            h.append(name, value);
        }
    }
    res
}

pub fn text(status: u16, body: &str) -> Response {
    response(status, &[("content-type", "text/plain".into())], Body::from(body.to_string()))
}

pub fn not_found() -> Response {
    text(404, "Not found")
}

pub fn redirect(location: &str) -> Response {
    response(302, &[("location", location.into()), ("cache-control", "no-store".into())], Body::empty())
}

/// Répond avec ce corps, compressé si le navigateur le sait et que ça vaut la peine.
pub fn buffer(status: u16, gz_ok: bool, data: Vec<u8>, kind: &str, extra: &[(&str, String)]) -> Response {
    let mut headers: Vec<(&str, String)> = vec![("content-type", kind.to_string())];
    headers.extend(extra.iter().cloned());
    if compressible(kind) && data.len() > 1024 && gz_ok {
        let packed = gzip(&data);
        headers.push(("content-encoding", "gzip".into()));
        headers.push(("vary", "Accept-Encoding".into()));
        return response(status, &headers, Body::from(packed));
    }
    response(status, &headers, Body::from(data))
}

/// Réponse JSON (`cache-control: no-store`), compressée au-delà de 1 Ko.
pub fn json(status: u16, gz_ok: bool, data: &Value, extra: &[(&str, String)]) -> Response {
    let raw = serde_json::to_vec(data).unwrap_or_default();
    let mut headers: Vec<(&str, String)> = vec![("cache-control", "no-store".into())];
    for (k, v) in extra {
        if let Some(slot) = headers.iter_mut().find(|(hk, _)| hk.eq_ignore_ascii_case(k)) {
            slot.1 = v.clone();
        } else {
            headers.push((k, v.clone()));
        }
    }
    let kind = extra
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "application/json; charset=utf-8".into());
    headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
    buffer(status, gz_ok, raw, &kind, &headers)
}

/// Erreur JSON `{ ok: false, error: { code, message } }`.
pub fn error_json(status: u16, gz_ok: bool, code: &str, message: &str, extra: &[(&str, String)]) -> Response {
    json(status, gz_ok, &serde_json::json!({ "ok": false, "error": { "code": code, "message": message } }), extra)
}

/// Réponse JSON déjà compressée (gzip) ; décompressée pour un client qui ne sait pas la lire.
pub fn gzipped(status: u16, gz_ok: bool, packed: &[u8]) -> Response {
    let base = [("content-type", "application/json; charset=utf-8".to_string()), ("cache-control", "no-store".into())];
    if gz_ok {
        let mut h = base.to_vec();
        h.push(("content-encoding", "gzip".into()));
        h.push(("vary", "Accept-Encoding".into()));
        return response(status, &h, Body::from(packed.to_vec()));
    }
    response(status, &base, Body::from(gunzip(packed).unwrap_or_default()))
}

/// Date HTTP (`toUTCString`).
pub fn http_date(ms: i64) -> String {
    let t = time::OffsetDateTime::from_unix_timestamp(ms.div_euclid(1000)).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        DAYS[t.weekday().number_days_from_monday() as usize],
        t.day(),
        MONTHS[t.month() as usize - 1],
        t.year(),
        t.hour(),
        t.minute(),
        t.second()
    )
}

/// Comment envoyer un fichier.
#[derive(Debug, Clone, Default)]
pub struct FileOpts {
    pub kind: Option<String>,
    pub cache: Option<String>,
    pub headers: Vec<(&'static str, String)>,
}

/// Fichiers de l'application déjà compressés (clé : chemin + empreinte), pour ne pas recompresser à chaque chargement.
type Packed = (HashMap<String, Arc<Vec<u8>>>, Vec<String>);
static PACKED: LazyLock<Mutex<Packed>> = LazyLock::new(|| Mutex::new((HashMap::new(), Vec::new())));
const PACKED_MAX: usize = 400;

/// Plage demandée hors du fichier (réponse 416).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unsatisfiable;

/// Analyse un en-tête Range `bytes=a-b` : (début, fin incluse), `Err` si hors limites, `Ok(None)` sans Range.
pub fn parse_range(header: &str, size: u64) -> Result<Option<(u64, u64)>, Unsatisfiable> {
    let Some(spec) = header.strip_prefix("bytes=") else { return Ok(None) };
    let Some((a, b)) = spec.split_once('-') else { return Ok(None) };
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if !digits(a) || !digits(b) || size == 0 {
        return Ok(None);
    }
    let (mut start, mut end) = (a.parse::<u64>().unwrap_or(0), b.parse::<u64>().unwrap_or(size - 1));
    if a.is_empty() && !b.is_empty() {
        start = size.saturating_sub(b.parse::<u64>().unwrap_or(0));
        end = size - 1;
    } else if a.is_empty() {
        start = 0;
    }
    end = end.min(size - 1);
    if start > end || start >= size {
        return Err(Unsatisfiable);
    }
    Ok(Some((start, end)))
}

/// Envoie un fichier : requêtes Range (lecture et déplacement dans un morceau), ETag, gzip pour le texte.
pub async fn send_file(method: &Method, req: &HeaderMap, file: &Path, opts: FileOpts) -> Response {
    let Ok(meta) = tokio::fs::metadata(file).await else { return not_found() };
    if !meta.is_file() {
        return not_found();
    }
    let size = meta.len();
    let mtime =
        meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
    let etag = format!("\"{size:x}-{mtime:x}\"");
    let kind = opts.kind.clone().unwrap_or_else(|| mime_of(file).to_string());
    let mut base: Vec<(&str, String)> = vec![
        ("content-type", kind.clone()),
        ("accept-ranges", "bytes".into()),
        ("cache-control", opts.cache.clone().unwrap_or_else(|| "no-cache".into())),
        ("etag", etag.clone()),
        ("last-modified", http_date(mtime)),
    ];
    base.extend(opts.headers.iter().cloned());
    let header = |k: &str| req.get(k).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    if header("if-none-match") == etag {
        return response(304, &base, Body::empty());
    }
    let head = method == Method::HEAD;
    let if_range = header("if-range");
    if if_range.is_empty() || if_range == etag {
        match parse_range(&header("range"), size) {
            Err(Unsatisfiable) => return response(416, &[("content-range", format!("bytes */{size}"))], Body::empty()),
            Ok(Some((start, end))) => {
                let len = end - start + 1;
                let mut h = base.clone();
                h.push(("content-range", format!("bytes {start}-{end}/{size}")));
                h.push(("content-length", len.to_string()));
                if head {
                    return response(206, &h, Body::empty());
                }
                let Ok(mut f) = tokio::fs::File::open(file).await else { return not_found() };
                if f.seek(std::io::SeekFrom::Start(start)).await.is_err() {
                    return not_found();
                }
                let stream = tokio_util::io::ReaderStream::new(f.take(len));
                return response(206, &h, Body::from_stream(stream));
            }
            Ok(None) => {}
        }
    }
    if compressible(&kind) && size < 4 * 1024 * 1024 && accepts_gzip(req) && !head {
        let key = format!("{}|{etag}", file.display());
        let cached = PACKED.lock().0.get(&key).cloned();
        let body = match cached {
            Some(b) => b,
            None => {
                let Ok(data) = tokio::fs::read(file).await else { return not_found() };
                let packed = Arc::new(gzip(&data));
                let mut p = PACKED.lock();
                if p.0.len() >= PACKED_MAX && !p.1.is_empty() {
                    let old = p.1.remove(0);
                    p.0.remove(&old);
                }
                p.0.insert(key.clone(), packed.clone());
                p.1.push(key);
                packed
            }
        };
        let mut h = base.clone();
        h.push(("content-encoding", "gzip".into()));
        h.push(("vary", "Accept-Encoding".into()));
        return response(200, &h, Body::from(body.as_ref().clone()));
    }
    let mut h = base;
    h.push(("content-length", size.to_string()));
    if head {
        return response(200, &h, Body::empty());
    }
    let Ok(f) = tokio::fs::File::open(file).await else { return not_found() };
    response(200, &h, Body::from_stream(tokio_util::io::ReaderStream::new(f)))
}

/// Chemin demandé dans un dossier, sans sortir de celui-ci (`None` si c'est le cas).
pub fn safe_join(root: &Path, url_path: &str) -> Option<PathBuf> {
    let decoded = decode_component(url_path)?;
    if decoded.contains('\0') {
        return None;
    }
    let mut out = root.to_path_buf();
    for part in decoded.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            p => out.push(p),
        }
    }
    (out != root && out.starts_with(root)).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=10-109", 1000), Ok(Some((10, 109))));
        assert_eq!(parse_range("bytes=-50", 1000), Ok(Some((950, 999))));
        assert_eq!(parse_range("bytes=900-", 1000), Ok(Some((900, 999))));
        assert_eq!(parse_range("bytes=900-5000", 1000), Ok(Some((900, 999))));
        assert_eq!(parse_range("bytes=1005-", 1000), Err(Unsatisfiable));
        assert_eq!(parse_range("items=0-1", 1000), Ok(None));
        assert_eq!(parse_range("", 1000), Ok(None));
    }

    #[test]
    fn cookies_and_paths() {
        let c = parse_cookies("a=1; control_session=abc%3D; =x; b");
        assert_eq!(c.get("control_session").map(String::as_str), Some("abc="));
        assert_eq!(
            cookie_header("control_session", "t", Some(10), true),
            "control_session=t; Path=/; SameSite=Lax; HttpOnly; Secure; Max-Age=10"
        );
        let root = Path::new("/app/web");
        assert_eq!(safe_join(root, "/js/app.js"), Some(PathBuf::from("/app/web/js/app.js")));
        assert_eq!(safe_join(root, "/js/%2e%2e/%2e%2e/package.json"), None);
        assert_eq!(safe_join(root, "/../x"), None);
        assert_eq!(safe_join(root, "/"), None);
        assert_eq!(http_date(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(decode_component("%E9"), None);
        assert_eq!(encode_component("a b/é"), "a%20b%2F%C3%A9");
    }
}

//! Pages et fichiers : l'application (avec son état de démarrage), les pages statiques (connexion, assistant,
//! administration), le manifeste, les fichiers de l'interface, le flux d'événements (SSE) et l'API Subsonic.

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use axum::body::Body;
use axum::http::Method;
use axum::response::Response;
use bytes::Bytes;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::{Req, blocking, view_of};
use crate::auth::User;
use crate::http::{self, FileOpts, safe_join};
use crate::hub::{Entry, Hub};
use crate::social::Social;
use crate::subsonic::{Outcome, Params, render};
use crate::util::HttpResult;
use control_core::media::Endpoints;

/// Dossier qui contient `src/renderer` et `src/web` : `CONTROL_ASSETS`, sinon le premier dossier parent du binaire
/// qui les contient (installation ou mise à jour : à côté du binaire ; développement : la racine du dépôt), sinon
/// la racine du dépôt où le binaire a été compilé.
pub fn assets_root() -> PathBuf {
    static ROOT: LazyLock<PathBuf> = LazyLock::new(|| {
        if let Ok(dir) = std::env::var("CONTROL_ASSETS")
            && !dir.is_empty()
        {
            return PathBuf::from(dir);
        }
        let has = |d: &Path| d.join("src/renderer/index.html").is_file();
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.ancestors().skip(1).find(|d| has(d))
        {
            return dir.to_path_buf();
        }
        let built = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        crate::util::resolve(built)
    });
    ROOT.clone()
}

fn renderer_dir() -> PathBuf {
    assets_root().join("src").join("renderer")
}

fn web_dir() -> PathBuf {
    assets_root().join("src").join("web")
}

const PAGE_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; font-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

fn app_csp() -> String {
    let endpoints = Endpoints::from_env();
    let mut images = vec!["'self'".to_string(), "data:".into(), "blob:".into(), "https:".into()];
    images.extend(endpoints.image_origins());
    [
        "default-src 'self'".to_string(),
        "script-src 'self'".into(),
        "style-src 'self' 'unsafe-inline'".into(),
        format!("img-src {}", images.join(" ")),
        "media-src 'self' blob:".into(),
        "font-src 'self'".into(),
        "connect-src 'self'".into(),
        "worker-src 'self'".into(),
        "manifest-src 'self'".into(),
        format!("frame-src {}", endpoints.frame_origins().join(" ")),
        "object-src 'none'".into(),
        "base-uri 'none'".into(),
        "frame-ancestors 'none'".into(),
    ]
    .join("; ")
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// JSON sûr dans une balise `<script>`.
fn for_json(v: &Value) -> String {
    v.to_string().replace('<', "\\u003c").replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029")
}

/// Le gabarit de la page, relu quand le fichier change.
async fn template() -> std::io::Result<String> {
    static CACHE: LazyLock<Mutex<Option<(std::time::SystemTime, String)>>> = LazyLock::new(|| Mutex::new(None));
    let file = renderer_dir().join("index.html");
    let mtime = tokio::fs::metadata(&file).await?.modified()?;
    if let Some((m, t)) = CACHE.lock().as_ref()
        && *m == mtime
    {
        return Ok(t.clone());
    }
    let text = tokio::fs::read_to_string(&file).await?;
    *CACHE.lock() = Some((mtime, text.clone()));
    Ok(text)
}

pub async fn static_page(hub: &Hub, req: &Req, name: &str) -> Response {
    let Ok(html) = tokio::fs::read_to_string(web_dir().join(name)).await else { return http::not_found() };
    let page = html.replace("{{SERVER_NAME}}", &escape_html(&hub.server_name()));
    http::buffer(
        200,
        req.gz,
        page.into_bytes(),
        "text/html; charset=utf-8",
        &[("content-security-policy", PAGE_CSP.into()), ("cache-control", "no-store".into())],
    )
}

pub async fn manifest(hub: &Hub, req: &Req) -> HttpResult<Response> {
    let text = tokio::fs::read_to_string(web_dir().join("manifest.webmanifest"))
        .await
        .map_err(|e| crate::util::http_error(500, "error", e.to_string()))?;
    let mut manifest: Value = serde_json::from_str(&text).unwrap_or_else(|_| json!({}));
    let name = hub.server_name();
    manifest["short_name"] = json!(name.chars().take(12).collect::<String>());
    manifest["name"] = json!(name);
    Ok(http::json(
        200,
        req.gz,
        &manifest,
        &[("content-type", "application/manifest+json".into()), ("cache-control", "no-cache".into())],
    ))
}

/// Fichiers de l'interface : `/js/`, `/styles/`, `/assets/` (src/renderer) et `/web/` (src/web).
pub async fn asset(req: &Req) -> Response {
    let (root, rel) = match req.path.strip_prefix("/web") {
        Some(rest) => (web_dir(), rest.to_string()),
        None => (renderer_dir(), req.path.clone()),
    };
    let Some(file) = safe_join(&root, &rel) else { return http::not_found() };
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let cache = if ["woff2", "woff", "ttf", "png", "jpg", "jpeg", "svg", "ico"].contains(&ext.as_str()) {
        "public, max-age=86400"
    } else {
        "no-cache"
    };
    http::send_file(&req.method, &req.headers, &file, FileOpts { cache: Some(cache.into()), ..FileOpts::default() })
        .await
}

fn person(p: &Value) -> Value {
    json!({ "id": p["id"], "username": p["username"], "name": p["name"], "avatar": p["avatar"], "color": p["color"] })
}

/// L'état de démarrage d'un compte (ce que le bureau obtient par IPC synchrone).
fn boot(hub: &Hub, user: &User, entry: &Entry, viewing: Option<&User>) -> Value {
    let upload_max = hub.upload_max_mb();
    let caps = json!({
        "editFiles": user.is_admin() && viewing.is_none(),
        "download": user.is_admin() || hub.setting_bool("allowUserDownloads"),
        "upload": user.is_admin() || hub.setting_bool("allowUserUploads"),
        "uploadMaxMb": upload_max,
        "manageServer": user.is_admin(),
        "installTools": user.is_admin(),
        "readOnly": viewing.is_some(),
    });
    let me = hub.accounts.by_id(&user.id).map(|u| Social::profile(&u)).unwrap_or_else(|| Social::profile(user));
    let libraries: Vec<Value> =
        hub.social.others(user).into_iter().filter(|(_, _, a)| a.browse).map(|(_, p, _)| person(&p)).collect();
    let mut me_json = person(&me);
    me_json["role"] = json!(user.role);
    let music_dir = hub
        .own_folders(&user.id)
        .into_iter()
        .next()
        .or_else(|| hub.common_folders().into_iter().next())
        .unwrap_or_default();
    json!({
        "settings": entry.core.store.settings_value(),
        "env": { "streamOverride": Endpoints::from_env().stream_override },
        "user": me_json,
        "viewing": viewing.map(|o| person(&Social::profile(o))),
        "libraries": libraries,
        "caps": caps,
        "server": { "name": hub.server_name() },
        "info": {
            "version": crate::hub::version(), "platform": "web", "electron": null, "chrome": null, "portable": false,
            "appimage": false, "dataDir": "", "musicDir": music_dir, "downloadDir": hub.download_dir_for(user).to_string_lossy(),
        },
        "clientId": hex::encode(rand::random::<[u8; 8]>()),
    })
}

/// La page de l'application, avec l'état de démarrage de ce compte.
pub async fn index(hub: &Arc<Hub>, req: &Req, user: User) -> HttpResult<Response> {
    let h = hub.clone();
    let lib = req.query("lib").map(str::to_string);
    let header = req.header("x-library").to_string();
    let state = blocking(move || -> HttpResult<Option<(Value, String)>> {
        let entry = h.entry(&user)?;
        let fake = Req {
            method: Method::GET,
            path: String::new(),
            query: lib.map(|l| vec![("lib".to_string(), l)]).unwrap_or_default(),
            headers: {
                let mut m = axum::http::HeaderMap::new();
                if let Ok(v) = axum::http::HeaderValue::from_str(&header) {
                    m.insert("x-library", v);
                }
                m
            },
            ip: String::new(),
            https: false,
            gz: false,
        };
        // Bibliothèque introuvable ou plus partagée : retour à la sienne.
        let Ok(view) = view_of(&h, &fake, &user, true) else { return Ok(None) };
        let boot = boot(&h, &user, &entry, view.as_ref().map(|v| &v.owner));
        let lang = if entry.core.store.get("language") == "en" { "en" } else { "fr" };
        Ok(Some((boot, lang.to_string())))
    })
    .await??;
    let Some((boot, lang)) = state else { return Ok(http::redirect("/")) };
    let name = escape_html(&hub.server_name());
    let head = [
        "<meta name=\"theme-color\" content=\"#0c0c0f\">".to_string(),
        "<meta name=\"apple-mobile-web-app-capable\" content=\"yes\">".into(),
        "<meta name=\"mobile-web-app-capable\" content=\"yes\">".into(),
        "<meta name=\"apple-mobile-web-app-status-bar-style\" content=\"black-translucent\">".into(),
        format!("<meta name=\"apple-mobile-web-app-title\" content=\"{name}\">"),
        "<link rel=\"manifest\" href=\"/manifest.webmanifest\">".into(),
        "<link rel=\"apple-touch-icon\" href=\"/web/icon-180.png\">".into(),
        format!("<script id=\"boot\" type=\"application/json\">{}</script>", for_json(&boot)),
        "<script src=\"/web/shim.js\"></script>".into(),
    ]
    .join("\n  ");
    let html = template()
        .await
        .map_err(|e| crate::util::http_error(500, "error", format!("Page de l’application introuvable : {e}")))?
        .replacen("<html lang=\"fr\"", &format!("<html lang=\"{lang}\""), 1)
        .replacen("width=device-width, initial-scale=1", "width=device-width, initial-scale=1, viewport-fit=cover", 1)
        .replacen("<title>control</title>", &format!("<title>{name}</title>\n  {head}"), 1)
        .replacen("</head>", "  <link rel=\"stylesheet\" href=\"/web/app-web.css\">\n</head>", 1)
        .replacen("</body>", "<script src=\"/web/app-web.js\" type=\"module\"></script>\n</body>", 1);
    Ok(http::buffer(
        200,
        req.gz,
        html.into_bytes(),
        "text/html; charset=utf-8",
        &[("content-security-policy", app_csp()), ("cache-control", "no-store".into())],
    ))
}

/// Retire la page de la liste des clients quand le flux se ferme.
struct ClientGuard {
    entry: Arc<Entry>,
    id: String,
    conn: u64,
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.entry.clients.remove(&self.id, self.conn);
        self.entry.touch();
    }
}

/// Événements en direct (SSE) : `event: <canal>\ndata: <json>\n\n`.
pub async fn events(hub: &Arc<Hub>, req: &Req, user: User) -> HttpResult<Response> {
    let mut client: String =
        req.query("c").unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric()).take(32).collect();
    if client.is_empty() {
        client = hex::encode(rand::random::<[u8; 6]>());
    }
    let h = hub.clone();
    let entry = blocking(move || h.entry(&user)).await??;
    // Un client qui se reconnecte remplace son ancienne connexion.
    let (conn, rx) = entry.clients.add(&client);
    entry.touch();
    let guard = ClientGuard { entry, id: client, conn };
    let start = futures_util::stream::iter([
        Ok::<_, std::convert::Infallible>(Bytes::from_static(b"retry: 2000\n\n")),
        Ok(Bytes::from_static(b"event: ready\ndata: null\n\n")),
    ]);
    let rest = futures_util::stream::unfold((rx, guard), |(mut rx, guard)| async move {
        rx.recv().await.map(|b| (Ok::<_, std::convert::Infallible>(b), (rx, guard)))
    });
    use futures_util::StreamExt as _;
    Ok(http::response(
        200,
        &[
            ("content-type", "text/event-stream".into()),
            ("cache-control", "no-cache, no-transform".into()),
            ("connection", "keep-alive".into()),
            ("x-accel-buffering", "no".into()),
        ],
        Body::from_stream(start.chain(rest)),
    ))
}

/// API compatible Subsonic : paramètres de l'adresse (et du corps d'un POST de formulaire).
pub async fn subsonic(hub: &Arc<Hub>, req: Req, body: Body) -> Response {
    let raw = req.path.trim_start_matches("/rest/");
    let decoded = crate::http::decode_component(raw).unwrap_or_else(|| raw.to_string());
    let lower = decoded.to_lowercase();
    let method = lower.strip_suffix(".view").unwrap_or(&lower).to_string();
    let mut params = Params(req.query.clone());
    if req.method == Method::POST
        && req.header("content-type").to_lowercase().contains("application/x-www-form-urlencoded")
    {
        let text = super::read_body(body, 1024 * 1024).await.unwrap_or_default();
        params.0.extend(url::form_urlencoded::parse(&text).map(|(k, v)| (k.into_owned(), v.into_owned())));
    }
    let (h, p, ip, agent) = (hub.clone(), params.clone(), req.ip.clone(), req.header("user-agent").to_string());
    let outcome = tokio::task::spawn_blocking(move || h.subsonic.handle(&h, &method, &p, &ip, &agent))
        .await
        .unwrap_or_else(|_| Err(crate::subsonic::sub_err(0, "Erreur du serveur.")));
    let payload = match outcome {
        Ok(Outcome::File { path, kind, cache, headers }) => {
            let opts = FileOpts { kind, cache: Some(cache.into()), headers };
            return http::send_file(&req.method, &req.headers, &path, opts).await;
        }
        Ok(Outcome::Payload(v)) => Ok(v),
        Err(e) => Err(e),
    };
    let (kind, text) = render(&params, payload.as_ref());
    http::response(200, &[("content-type", kind.into()), ("cache-control", "no-store".into())], Body::from(text))
}

//! Les routes : pages, authentification, RPC, événements en direct et médias. Même routage que `server/app.js`,
//! fait à la main dans un seul gestionnaire (l'ordre des règles compte).

mod account;
mod pages;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method};
use axum::response::Response;
use serde_json::{Value, json};

pub use pages::assets_root;

use crate::auth::User;
use crate::http::{self, FileOpts, accepts_gzip, decode_component, error_json, parse_cookies};
use crate::hub::{Entry, Hub};
use crate::rpc::{self, Call, View};
use crate::util::{HttpError, HttpResult, http_error, inside};

pub const COOKIE: &str = "control_session";
const MEDIA_CACHE: &str = "private, no-cache";

/// Ce qu'on sait d'une requête.
pub struct Req {
    pub method: Method,
    /// Chemin normalisé (segments « . » et « .. » résolus, comme `new URL()`), encore encodé.
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: HeaderMap,
    pub ip: String,
    pub https: bool,
    pub gz: bool,
}

impl Req {
    pub fn header(&self, name: &str) -> &str {
        self.headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("")
    }

    pub fn query(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn cookie(&self) -> String {
        parse_cookies(self.header("cookie")).remove(COOKIE).unwrap_or_default()
    }
}

/// Exécute un travail bloquant (base, noyau, scrypt) hors du fil d'entrée-sortie.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> HttpResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| http_error(500, "error", format!("Tâche interrompue : {e}")))
}

/// Corps de la requête, limité en taille.
pub async fn read_body(body: Body, limit: usize) -> HttpResult<bytes::Bytes> {
    axum::body::to_bytes(body, limit).await.map_err(|_| http_error(413, "too_large", "Requête trop volumineuse."))
}

/// Corps JSON (`{}` s'il est vide).
pub async fn read_json(body: Body, limit: usize) -> HttpResult<Value> {
    let bytes = read_body(body, limit).await?;
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(&bytes).map_err(|_| http_error(400, "bad_json", "JSON invalide."))
}

pub fn require_csrf(req: &Req) -> HttpResult<()> {
    if req.header("x-control") == "1" { Ok(()) } else { Err(http_error(403, "csrf", "Requête refusée.")) }
}

fn client_ip(hub: &Hub, headers: &HeaderMap, addr: SocketAddr) -> String {
    if hub.config.trust_proxy {
        let forwarded = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).unwrap_or("");
        let first = forwarded.split(',').next().unwrap_or("").trim();
        if !first.is_empty() {
            return first.to_string();
        }
    }
    match addr.ip() {
        std::net::IpAddr::V6(v6) => {
            v6.to_ipv4_mapped().map(|v4| format!("::ffff:{v4}")).unwrap_or_else(|| v6.to_string())
        }
        ip => ip.to_string(),
    }
}

fn normalize_path(raw: &str) -> String {
    url::Url::parse("http://localhost")
        .and_then(|base| base.join(raw))
        .map(|u| u.path().to_string())
        .unwrap_or_else(|_| "/".into())
}

/// La bibliothèque d'un autre compte demandée par la requête (en-tête `x-library` ou `?lib=`), ou `None`.
fn view_of(hub: &Hub, req: &Req, user: &User, use_query: bool) -> HttpResult<Option<View>> {
    let mut id = req.header("x-library").to_string();
    if id.is_empty() && use_query {
        id = req.query("lib").unwrap_or("").to_string();
    }
    let id: String = id.chars().take(32).collect();
    if id.is_empty() || id == user.id {
        return Ok(None);
    }
    let owner = hub
        .accounts
        .by_id(&id)
        .filter(|o| !o.disabled)
        .ok_or_else(|| http_error(404, "not_found", "Bibliothèque introuvable."))?;
    if !hub.social.access(user, &owner).browse {
        return Err(http_error(403, "forbidden", "Cette bibliothèque ne vous est pas partagée."));
    }
    let entry = hub.entry(&owner)?;
    Ok(Some(View { owner, entry }))
}

/// Le point d'entrée de toutes les requêtes.
pub async fn handle(
    State(hub): State<Arc<Hub>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let raw = parts.uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_else(|| "/".into());
    let parsed = url::Url::parse("http://localhost").and_then(|b| b.join(&raw)).ok();
    let https = hub.config.trust_proxy
        && parts
            .headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            == "https";
    let req = Req {
        method: parts.method.clone(),
        path: parsed.as_ref().map(|u| u.path().to_string()).unwrap_or_else(|| normalize_path(&raw)),
        query: parsed
            .map(|u| u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect())
            .unwrap_or_default(),
        ip: client_ip(&hub, &parts.headers, addr),
        gz: accepts_gzip(&parts.headers),
        headers: parts.headers,
        https,
    };
    let gz = req.gz;
    let (method, path) = (req.method.clone(), req.path.clone());
    // Le corps est lu en entier avant de répondre (sauf pour un envoi de fichier, lu en flux) : une réponse partie
    // avant la fin du corps obligerait à fermer la connexion, et le navigateur pourrait la réutiliser entre-temps.
    let body = if path == "/api/upload" { Ok(body) } else { read_body(body, 40 * 1024 * 1024).await.map(Body::from) };
    let routed = match body {
        Ok(body) => route(&hub, req, body).await,
        Err(err) => Err(err),
    };
    let mut res = match routed {
        Ok(r) => r,
        Err(err) => error_response(&err, gz, &method, &path),
    };
    let refused_upload = path == "/api/upload" && !res.status().is_success();
    let h = res.headers_mut();
    if refused_upload {
        // Refus en cours d'envoi : le reste du fichier n'est pas lu, la connexion se ferme après la réponse.
        h.insert("connection", HeaderValue::from_static("close"));
    }
    h.insert("referrer-policy", HeaderValue::from_static("same-origin"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    if https {
        h.insert("strict-transport-security", HeaderValue::from_static("max-age=15552000"));
    }
    res
}

fn error_response(err: &HttpError, gz: bool, method: &Method, path: &str) -> Response {
    let status = if err.status < 400 { 500 } else { err.status };
    if status >= 500 {
        eprintln!("[{method} {path}] {}", err.message);
    }
    let message = if status >= 500 { "Erreur du serveur." } else { err.message.as_str() };
    let code = if err.code.is_empty() { "error" } else { err.code.as_str() };
    error_json(status, gz, code, message, &[])
}

async fn route(hub: &Arc<Hub>, req: Req, body: Body) -> HttpResult<Response> {
    let path = req.path.as_str();
    let method = &req.method;
    let get = method == Method::GET || method == Method::HEAD;

    // API compatible Subsonic : elle a sa propre authentification (mot de passe d'application ou clé d'API).
    if path.starts_with("/rest/") && (get || method == Method::POST) {
        return Ok(pages::subsonic(hub, req, body).await);
    }
    if path == "/api/health" {
        return Ok(http::json(
            200,
            req.gz,
            &json!({ "ok": true, "name": "control", "version": crate::hub::version() }),
            &[],
        ));
    }

    // Premier lancement : tant qu'aucun compte n'existe, tout mène à l'assistant.
    let h = hub.clone();
    let initialized = blocking(move || h.accounts.count() > 0).await?;
    if path == "/api/setup/status" {
        return Ok(http::json(
            200,
            req.gz,
            &json!({
                "initialized": initialized, "serverName": hub.server_name(), "tokenRequired": !hub.config.setup_token.is_empty(),
                "folders": hub.config.library_folders, "registrationEnabled": hub.setting_bool("registrationEnabled"),
            }),
            &[],
        ));
    }
    if path == "/api/setup/init" && method == Method::POST {
        return account::setup_init(hub, &req, body).await;
    }
    if path == "/setup" {
        return Ok(if initialized { http::redirect("/") } else { pages::static_page(hub, &req, "setup.html").await });
    }
    if path == "/api/auth/login" && method == Method::POST {
        return account::login(hub, &req, body).await;
    }
    if path == "/api/auth/register" && method == Method::POST {
        return account::register(hub, &req, body).await;
    }
    if path == "/api/auth/logout" && method == Method::POST {
        require_csrf(&req)?;
        let (h, token) = (hub.clone(), req.cookie());
        blocking(move || h.accounts.close(&token)).await?;
        return Ok(http::json(
            200,
            req.gz,
            &json!({ "ok": true }),
            &[("set-cookie", account::session_cookie(hub, &req, "", 0))],
        ));
    }
    if path == "/manifest.webmanifest" {
        return pages::manifest(hub, &req).await;
    }
    if path == "/sw.js" {
        let file = assets_root().join("src/web/sw.js");
        let opts = FileOpts {
            cache: Some("no-cache".into()),
            headers: vec![("service-worker-allowed", "/".into())],
            ..FileOpts::default()
        };
        return Ok(http::send_file(method, &req.headers, &file, opts).await);
    }
    let asset = ["/js/", "/styles/", "/assets/", "/web/"].iter().any(|p| path.starts_with(p));
    if asset && get {
        return Ok(pages::asset(&req).await);
    }

    let (h, token) = (hub.clone(), req.cookie());
    let user = blocking(move || h.accounts.lookup(&token)).await?;
    if path == "/login" {
        if !initialized {
            return Ok(http::redirect("/setup"));
        }
        return Ok(if user.is_some() {
            http::redirect("/")
        } else {
            pages::static_page(hub, &req, "login.html").await
        });
    }

    // Tout le reste demande un compte connecté.
    let Some(user) = user else {
        if !initialized && path == "/" {
            return Ok(http::redirect("/setup"));
        }
        if path == "/" || path == "/admin" {
            return Ok(http::redirect("/login"));
        }
        return Ok(error_json(401, req.gz, "unauthorized", "Connexion requise.", &[]));
    };

    if path == "/api/auth/me" {
        let name = if user.display_name.is_empty() { &user.username } else { &user.display_name };
        let me = json!({ "id": user.id, "username": user.username, "role": user.role, "name": name });
        return Ok(http::json(200, req.gz, &json!({ "ok": true, "user": me }), &[]));
    }
    if path == "/" || path == "/index.html" {
        return pages::index(hub, &req, user).await;
    }
    if path == "/admin" {
        return Ok(if user.is_admin() {
            pages::static_page(hub, &req, "admin.html").await
        } else {
            http::redirect("/")
        });
    }
    if path == "/api/events" && method == Method::GET {
        return pages::events(hub, &req, user).await;
    }
    if path == "/api/upload" && method == Method::POST {
        return upload(hub, &req, user, body).await;
    }
    let rpc_call = path.starts_with("/api/rpc/");
    if (rpc_call || path.starts_with("/api/send/")) && method == Method::POST {
        require_csrf(&req)?;
        let raw = &path[if rpc_call { 9 } else { 10 }..];
        let channel = decode_component(raw).ok_or_else(|| http_error(400, "bad_request", "Commande illisible."))?;
        let body = read_json(body, 40 * 1024 * 1024).await?;
        let args: Vec<Value> = body.get("args").and_then(Value::as_array).cloned().unwrap_or_default();
        let client: String = req.header("x-client").chars().take(32).collect();
        let headers = req.headers.clone();
        let gz = req.gz;
        let h = hub.clone();
        let req_view = Req {
            headers,
            method: req.method.clone(),
            path: req.path.clone(),
            query: Vec::new(),
            ip: req.ip.clone(),
            https: req.https,
            gz,
        };
        let out = blocking(move || -> HttpResult<Response> {
            let entry = h.entry(&user)?;
            let view = view_of(&h, &req_view, &user, false)?;
            let call = Call { hub: &h, entry: &entry, user: &user, client: &client };
            if !rpc_call {
                rpc::send(&call, &channel, &args, view.as_ref())?;
                return Ok(http::response(204, &[], Body::empty()));
            }
            // La liste de tous les morceaux est le plus gros envoi : elle est construite à part et gardée prête.
            if channel == "lib:tracks" && view.is_none() {
                return Ok(http::gzipped(200, gz, &h.tracks_payload(&entry)));
            }
            let result = rpc::invoke(&call, &channel, &args, view.as_ref())?;
            Ok(http::json(200, gz, &json!({ "ok": true, "result": result }), &[]))
        })
        .await??;
        return Ok(out);
    }
    let media = ["/media/", "/cover/", "/vthumb/", "/avatar/"].iter().any(|p| path.starts_with(p));
    if media && get {
        return media_route(hub, &req, user).await;
    }
    Ok(error_json(404, req.gz, "not_found", "Introuvable.", &[]))
}

async fn upload(hub: &Arc<Hub>, req: &Req, user: User, body: Body) -> HttpResult<Response> {
    require_csrf(req)?;
    if !user.is_admin() && !hub.setting_bool("allowUserUploads") {
        return Err(http_error(403, "forbidden", "Les envois de musique sont réservés à l’administrateur."));
    }
    let h = hub.clone();
    let u = user.clone();
    let entry: Arc<Entry> = blocking(move || h.entry(&u)).await??;
    let root = hub.download_dir_for(&user).join("Envois");
    let declared = req.header("content-length").parse::<u64>().ok();
    let max = hub.upload_max_mb() as u64 * 1048576;
    let result = crate::upload::receive(
        body,
        entry,
        root,
        req.query("name").unwrap_or(""),
        req.query("dir").unwrap_or(""),
        declared,
        max,
    )
    .await?;
    Ok(http::json(200, req.gz, &json!({ "ok": true, "result": result }), &[]))
}

fn hash_name(name: &str, suffix_ok: bool) -> bool {
    let Some(stem) = name.strip_suffix(".jpg") else { return false };
    let stem = if suffix_ok { stem.strip_suffix("-s").unwrap_or(stem) } else { stem };
    stem.len() == 20 && stem.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

async fn media_route(hub: &Arc<Hub>, req: &Req, user: User) -> HttpResult<Response> {
    let path = req.path.as_str();
    let long =
        |cache: &str| FileOpts { kind: Some("image/jpeg".into()), cache: Some(cache.into()), ..FileOpts::default() };
    // Pochettes et avatars : des images nommées par leur empreinte, communes à tous les comptes connectés.
    if let Some(name) = path.strip_prefix("/cover/") {
        if !hash_name(name, true) {
            return Ok(http::not_found());
        }
        return Ok(http::send_file(
            &req.method,
            &req.headers,
            &hub.covers_dir.join(name),
            long("private, max-age=31536000, immutable"),
        )
        .await);
    }
    if let Some(name) = path.strip_prefix("/avatar/") {
        if !hash_name(name, false) {
            return Ok(http::not_found());
        }
        return Ok(http::send_file(
            &req.method,
            &req.headers,
            &hub.avatars_dir.join(name),
            long("private, max-age=31536000, immutable"),
        )
        .await);
    }
    // Fichiers d'un compte : le sien, ou celui d'un autre compte qui le partage (?lib=).
    let (h, headers, lib) = (hub.clone(), req.headers.clone(), req.query("lib").map(str::to_string));
    let is_media = path.starts_with("/media/");
    let id = path[if is_media { 7 } else { 8 }..].parse::<i64>().ok();
    let file = blocking(move || -> HttpResult<Option<std::path::PathBuf>> {
        let fake = Req {
            method: Method::GET,
            path: String::new(),
            query: lib.map(|l| vec![("lib".into(), l)]).unwrap_or_default(),
            headers,
            ip: String::new(),
            https: false,
            gz: false,
        };
        let view = view_of(&h, &fake, &user, true)?;
        let entry = match &view {
            Some(v) => v.entry.clone(),
            None => h.entry(&user)?,
        };
        let Some(id) = id else { return Ok(None) };
        if is_media {
            let roots = match &view {
                Some(v) => h.owner_roots(&v.owner),
                None => h.roots_for(&user.id),
            };
            return Ok(entry.core.library.track_path(id).filter(|f| inside(&roots, f)).map(Into::into));
        }
        let ffmpeg = h.tools.path(control_core::tools::Tool::Ffmpeg);
        Ok(entry.core.library.video_thumb(id, ffmpeg.as_deref()))
    })
    .await??;
    let Some(file) = file else { return Ok(http::not_found()) };
    let opts = if is_media {
        FileOpts { cache: Some(MEDIA_CACHE.into()), ..FileOpts::default() }
    } else {
        long("private, max-age=3600")
    };
    Ok(http::send_file(&req.method, &req.headers, &file, opts).await)
}

//! Authentification : premier compte (assistant), connexion, inscription. Essais limités par adresse réelle.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::body::Body;
use axum::response::Response;
use serde_json::{Value, json};

use super::{COOKIE, Req, blocking, read_json, require_csrf};
use crate::http::{self, cookie_header, error_json};
use crate::hub::Hub;
use crate::util::{HttpResult, http_error};
use control_core::core::text;

pub fn secure_cookie(hub: &Hub, req: &Req) -> bool {
    match hub.config.cookie_secure.as_str() {
        "auto" => req.https,
        v => matches!(v, "1" | "true" | "yes" | "on"),
    }
}

pub fn session_cookie(hub: &Hub, req: &Req, token: &str, max_age: i64) -> String {
    cookie_header(COOKIE, token, Some(max_age), secure_cookie(hub, req))
}

fn user_json(u: &crate::auth::User) -> Value {
    json!({ "id": u.id, "username": u.username, "role": u.role })
}

pub async fn login(hub: &Arc<Hub>, req: &Req, body: Body) -> HttpResult<Response> {
    require_csrf(req)?;
    let body = read_json(body, 64 * 1024).await?;
    let ip = req.ip.clone();
    let name: String = text(&body["username"]).chars().take(64).collect();
    let password = text(&body["password"]);
    let keys = [format!("ip:{ip}"), format!("user:{ip}:{}", name.to_lowercase())];
    let wait = hub.ip_throttle.wait(&keys[0]).max(hub.login_throttle.wait(&keys[1]));
    if wait > 0 {
        let message = format!("Trop d’essais. Réessayez dans {} min.", (wait + 59) / 60);
        return Ok(error_json(429, req.gz, "too_many", &message, &[("retry-after", wait.to_string())]));
    }
    let h = hub.clone();
    let user = blocking(move || h.accounts.check(&name, &password)).await?;
    let Some(user) = user else {
        hub.login_throttle.fail(&keys[1]);
        hub.ip_throttle.fail(&keys[0]);
        return Ok(error_json(401, req.gz, "invalid_credentials", "Nom ou mot de passe incorrect.", &[]));
    };
    hub.login_throttle.clear(&keys[1]);
    let (h, id, ua) = (hub.clone(), user.id.clone(), req.header("user-agent").to_string());
    let (token, max_age) = blocking(move || h.accounts.open(&id, &ua, &ip)).await?;
    Ok(http::json(
        200,
        req.gz,
        &json!({ "ok": true, "user": user_json(&user) }),
        &[("set-cookie", session_cookie(hub, req, &token, max_age))],
    ))
}

pub async fn register(hub: &Arc<Hub>, req: &Req, body: Body) -> HttpResult<Response> {
    require_csrf(req)?;
    let h = hub.clone();
    let admins = blocking(move || h.accounts.admin_count()).await?;
    if !hub.setting_bool("registrationEnabled") || admins == 0 {
        return Err(http_error(403, "registration_disabled", "Les inscriptions sont fermées."));
    }
    let body = read_json(body, 64 * 1024).await?;
    let key = format!("reg:{}", req.ip);
    if hub.ip_throttle.wait(&key) > 0 {
        return Err(http_error(429, "too_many", "Trop d’essais, réessayez plus tard."));
    }
    hub.ip_throttle.fail(&key);
    let (h, ua, ip) = (hub.clone(), req.header("user-agent").to_string(), req.ip.clone());
    let (user, token, max_age) = blocking(move || -> HttpResult<_> {
        let user = h.accounts.create(&text(&body["username"]), &text(&body["password"]), "user")?;
        if !h.setting_str("usersRoot").is_empty()
            && let Err(err) = h.ensure_personal_folder(&user)
        {
            eprintln!("Dossier du compte : {}", err.message);
        }
        let (token, max_age) = h.accounts.open(&user.id, &ua, &ip);
        Ok((user, token, max_age))
    })
    .await??;
    Ok(http::json(
        200,
        req.gz,
        &json!({ "ok": true, "user": user_json(&user) }),
        &[("set-cookie", session_cookie(hub, req, &token, max_age))],
    ))
}

/// Remet le drapeau « création en cours » à zéro, quoi qu'il arrive.
struct Initializing<'a>(&'a Hub);

impl Drop for Initializing<'_> {
    fn drop(&mut self) {
        self.0.initializing.store(false, Ordering::SeqCst);
    }
}

pub async fn setup_init(hub: &Arc<Hub>, req: &Req, body: Body) -> HttpResult<Response> {
    require_csrf(req)?;
    let h = hub.clone();
    let count = blocking(move || h.accounts.count()).await?;
    // Deux requêtes simultanées ne doivent pas créer deux administrateurs : la création du compte est lente (scrypt).
    if count > 0 || hub.initializing.swap(true, Ordering::SeqCst) {
        return Err(http_error(403, "already_initialized", "Le serveur est déjà configuré."));
    }
    let guard = Initializing(hub);
    let body = read_json(body, 64 * 1024).await?;
    if !hub.config.setup_token.is_empty() && body["token"].as_str() != Some(hub.config.setup_token.as_str()) {
        return Err(http_error(403, "bad_token", "Code d’installation incorrect."));
    }
    let (h, ua, ip) = (hub.clone(), req.header("user-agent").to_string(), req.ip.clone());
    let (token, max_age) = blocking(move || -> HttpResult<_> {
        let user = h.accounts.create(&text(&body["username"]), &text(&body["password"]), "admin")?;
        let name =
            if control_core::core::truthy(&body["serverName"]) { body["serverName"].clone() } else { json!("control") };
        let mut patch = json!({ "serverName": name });
        if control_core::core::truthy(&body["tmdbKey"]) {
            patch["tmdbKey"] = json!(text(&body["tmdbKey"]));
        }
        h.patch_settings(&patch)?;
        // Les dossiers de l'assistant sont ceux de l'administrateur ; les autres comptes recevront les leurs plus tard.
        if let Some(list) = body["libraryFolders"].as_array() {
            for dir in list.iter().take(20) {
                let _ = h.add_own_folder(&user.id, &text(dir)); // dossier absent : ignoré
            }
        }
        Ok(h.accounts.open(&user.id, &ua, &ip))
    })
    .await??;
    drop(guard);
    Ok(http::json(200, req.gz, &json!({ "ok": true }), &[("set-cookie", session_cookie(hub, req, &token, max_age))]))
}

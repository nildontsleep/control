//! API compatible Subsonic / OpenSubsonic (`/rest/…`) : les applications du commerce (Symfonium, Substreamer, DSub,
//! Feishin…) se connectent au serveur et parcourent, écoutent et notent la bibliothèque du compte. Authentification
//! propre à cette API : mot de passe d'application du compte (u + p, ou u + t + s avec t = md5(secret + s)) ou clé
//! d'API (apiKey), jamais le cookie de session ni le mot de passe du compte. Repris de `server/subsonic.js`.

mod methods;
pub mod model;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use md5::{Digest as _, Md5};
use parking_lot::Mutex;
use serde_json::{Map, Value, json};

use crate::auth::{User, safe_equal, sha256_hex};
use crate::hub::{Entry, Hub};
use crate::secrets::Secrets;
use crate::util::now_ms;
use model::Model;

pub const API_VERSION: &str = "1.16.1";
pub const FAVORITES: &str = "Favoris";

pub mod code {
    pub const GENERIC: i64 = 0;
    pub const MISSING: i64 = 10;
    pub const WRONG_CREDS: i64 = 40;
    pub const CONFLICT: i64 = 43;
    pub const BAD_KEY: i64 = 44;
    pub const FORBIDDEN: i64 = 50;
    pub const NOT_FOUND: i64 = 70;
}

/// Erreur Subsonic : code de l'API et message (et, pour le journal seulement, la raison exacte).
#[derive(Debug, Clone)]
pub struct SubError {
    pub code: i64,
    pub message: String,
    pub reason: Option<String>,
}

pub fn sub_err(code: i64, message: impl Into<String>) -> SubError {
    SubError { code, message: message.into(), reason: None }
}

pub type SubResult<T> = Result<T, SubError>;

/// Paramètres de la requête (adresse, et corps d'un POST de formulaire), dans l'ordre.
#[derive(Debug, Clone, Default)]
pub struct Params(pub Vec<(String, String)>);

impl Params {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn all(&self, name: &str) -> Vec<&str> {
        self.0.iter().filter(|(k, _)| k == name).map(|(_, v)| v.as_str()).collect()
    }

    /// Nombre (`num(v, fallback)`).
    pub fn num(&self, name: &str, fallback: f64) -> f64 {
        self.get(name)
            .filter(|v| !v.is_empty())
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|n| n.is_finite())
            .unwrap_or(fallback)
    }

    /// Paramètre obligatoire.
    pub fn need(&self, name: &str) -> SubResult<&str> {
        self.get(name)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| sub_err(code::MISSING, format!("Paramètre manquant : {name}.")))
    }
}

// ─── Sérialisation ─────────────────────────────────────────────────────────────

fn esc(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\0'..='\x08' | '\x0b' | '\x0c' | '\x0e'..='\x1f' => {}
            c => out.push(c),
        }
    }
    out
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n
            .as_f64()
            .filter(|f| f.fract() == 0.0 && f.abs() < 1e15)
            .map(|f| (f as i64).to_string())
            .unwrap_or_else(|| n.to_string()),
        other => other.to_string(),
    }
}

/// `{ clé: valeur }` → `<nom clé="valeur">…</nom>` : les valeurs simples deviennent des attributs, objets et tableaux
/// des éléments. `first` : attributs à écrire en premier, dans cet ordre.
pub fn to_xml(name: &str, obj: &Value, first: &[(&str, Value)]) -> String {
    let mut attrs = String::new();
    let mut children = String::new();
    let mut text = String::new();
    for (k, v) in first {
        attrs.push_str(&format!(" {k}=\"{}\"", esc(&scalar(v))));
    }
    if let Some(map) = obj.as_object() {
        for (key, value) in map {
            match value {
                Value::Null => {}
                Value::Array(items) => {
                    for item in items {
                        if item.is_object() {
                            children.push_str(&to_xml(key, item, &[]));
                        } else if !item.is_null() {
                            children.push_str(&format!("<{key}>{}</{key}>", esc(&scalar(item))));
                        }
                    }
                }
                Value::Object(_) => children.push_str(&to_xml(key, value, &[])),
                v if key == "value" && name == "lyrics" => text = esc(&scalar(v)),
                v => attrs.push_str(&format!(" {key}=\"{}\"", esc(&scalar(v)))),
            }
        }
    }
    if children.is_empty() && text.is_empty() {
        format!("<{name}{attrs}/>")
    } else {
        format!("<{name}{attrs}>{text}{children}</{name}>")
    }
}

/// Le corps d'une réponse : (type, texte), au format demandé (`f` : xml, json ou jsonp).
pub fn render(params: &Params, result: Result<&Value, &SubError>) -> (&'static str, String) {
    let base = [
        ("status", json!(if result.is_ok() { "ok" } else { "failed" })),
        ("version", json!(API_VERSION)),
        ("type", json!("control")),
        ("serverVersion", json!(crate::hub::version())),
        ("openSubsonic", json!(true)),
    ];
    let payload = match result {
        Ok(v) => v.clone(),
        Err(e) => json!({ "error": { "code": e.code, "message": e.message } }),
    };
    let format = params.get("f").unwrap_or("xml").to_lowercase();
    if format == "json" || format == "jsonp" {
        let mut body: Map<String, Value> = base.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        if let Value::Object(p) = payload {
            body.extend(p);
        }
        let text = json!({ "subsonic-response": body }).to_string();
        let cb = params.get("callback").unwrap_or("");
        let valid = !cb.is_empty() && cb.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$');
        if format == "jsonp" && valid {
            return ("text/javascript; charset=utf-8", format!("{cb}({text});"));
        }
        return (
            if format == "jsonp" { "text/javascript; charset=utf-8" } else { "application/json; charset=utf-8" },
            text,
        );
    }
    let mut first = vec![("xmlns", json!("http://subsonic.org/restapi"))];
    first.extend(base.iter().cloned());
    (
        "text/xml; charset=utf-8",
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{}", to_xml("subsonic-response", &payload, &first)),
    )
}

/// Ce que rend une méthode : un contenu à sérialiser, ou un fichier à envoyer tel quel (flux, image).
pub enum Outcome {
    Payload(Value),
    File { path: PathBuf, kind: Option<String>, cache: &'static str, headers: Vec<(&'static str, String)> },
}

pub struct Ctx<'a> {
    pub hub: &'a Arc<Hub>,
    pub user: User,
    pub params: Params,
    pub entry: Arc<Entry>,
}

pub struct Subsonic {
    secrets: Secrets,
    journal: Mutex<VecDeque<Value>>,
}

fn md5_hex(s: &str) -> String {
    hex::encode(Md5::digest(s.as_bytes()))
}

impl Subsonic {
    pub fn new(data_dir: &std::path::Path) -> Subsonic {
        Subsonic { secrets: Secrets::new(data_dir), journal: Mutex::new(VecDeque::new()) }
    }

    /// Les dernières connexions (réussies : seulement le « ping » ; refusées : toutes), la plus récente d'abord.
    pub fn recent(&self) -> Value {
        Value::Array(self.journal.lock().iter().rev().cloned().collect())
    }

    fn note(&self, mut entry: Value) {
        entry["at"] = json!(now_ms());
        let mut j = self.journal.lock();
        j.push_back(entry);
        while j.len() > 40 {
            j.pop_front();
        }
    }

    // ─── Mot de passe d'application ────────────────────────────────────────

    pub fn secret_of(&self, hub: &Hub, user_id: &str) -> Option<String> {
        let sealed: Option<String> = hub
            .db
            .lock()
            .query_row("SELECT subsonic_secret FROM users WHERE id = ?", [user_id], |r| r.get(0))
            .ok()
            .flatten();
        sealed.and_then(|s| self.secrets.open(&s))
    }

    /// Le secret du compte ; en crée un si besoin (ou `renew`).
    pub fn ensure_secret(&self, hub: &Hub, user_id: &str, renew: bool) -> String {
        if !renew && let Some(s) = self.secret_of(hub, user_id) {
            return s;
        }
        use base64::Engine as _;
        let secret = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8; 18]>());
        let _ = hub.db.lock().execute(
            "UPDATE users SET subsonic_secret = ?, subsonic_key_hash = ? WHERE id = ?",
            rusqlite::params![self.secrets.seal(&secret), sha256_hex(&secret), user_id],
        );
        secret
    }

    pub fn revoke_secret(&self, hub: &Hub, user_id: &str) {
        let _ = hub
            .db
            .lock()
            .execute("UPDATE users SET subsonic_secret = NULL, subsonic_key_hash = NULL WHERE id = ?", [user_id]);
    }

    // ─── Authentification ──────────────────────────────────────────────────

    fn authenticate(&self, hub: &Hub, params: &Params) -> SubResult<User> {
        let refuse =
            |code, message: &str, reason: String| SubError { code, message: message.into(), reason: Some(reason) };
        if !hub.setting_bool("subsonicEnabled") {
            return Err(refuse(
                code::FORBIDDEN,
                "L’API Subsonic est désactivée sur ce serveur.",
                "API désactivée dans /admin".into(),
            ));
        }
        let api_key = params.get("apiKey").filter(|s| !s.is_empty());
        let name = params.get("u").filter(|s| !s.is_empty());
        if api_key.is_some() && name.is_some() {
            return Err(refuse(
                code::CONFLICT,
                "Plusieurs méthodes d’authentification fournies.",
                "apiKey et identifiant envoyés ensemble".into(),
            ));
        }
        if let Some(key) = api_key {
            let row = hub
                .db
                .lock()
                .query_row(
                    "SELECT * FROM users WHERE subsonic_key_hash = ? AND disabled = 0",
                    [sha256_hex(key)],
                    User::from_row,
                )
                .ok();
            return row.ok_or_else(|| refuse(code::BAD_KEY, "Clé d’API invalide.", "clé d’API inconnue".into()));
        }
        let Some(name) = name else {
            return Err(refuse(code::MISSING, "Paramètre manquant : u.", "identifiant absent".into()));
        };
        let row = hub
            .db
            .lock()
            .query_row("SELECT * FROM users WHERE username = ? COLLATE NOCASE AND disabled = 0", [name], User::from_row)
            .ok();
        let secret = row.as_ref().and_then(|r| self.secret_of(hub, &r.id));
        let (token, salt) = (params.get("t").filter(|s| !s.is_empty()), params.get("s").filter(|s| !s.is_empty()));
        let mut ok = false;
        if let Some(secret) = &secret {
            if let (Some(t), Some(s)) = (token, salt) {
                ok = safe_equal(md5_hex(&format!("{secret}{s}")).as_bytes(), t.to_lowercase().as_bytes());
            } else if let Some(p) = params.get("p") {
                let given = match p.strip_prefix("enc:") {
                    Some(h) => {
                        hex::decode(h).ok().map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
                    }
                    None => p.to_string(),
                };
                ok = safe_equal(given.as_bytes(), secret.as_bytes());
            } else {
                return Err(refuse(
                    code::MISSING,
                    "Authentification manquante (p, ou t et s).",
                    "ni mot de passe ni jeton envoyé".into(),
                ));
            }
        }
        match (ok, row) {
            (true, Some(user)) => Ok(user),
            (_, row) => {
                let reason = if row.is_none() {
                    format!("compte « {} » introuvable ou désactivé", name.chars().take(40).collect::<String>())
                } else if secret.is_none() {
                    "ce compte n’a pas encore de mot de passe d’application (Mon profil → Applications compatibles Subsonic)".into()
                } else if token.is_some() {
                    "jeton incorrect : ce n’est pas le mot de passe d’application".into()
                } else {
                    "mot de passe incorrect : ce n’est pas le mot de passe d’application".into()
                };
                Err(refuse(code::WRONG_CREDS, "Nom d’utilisateur ou mot de passe d’application incorrect.", reason))
            }
        }
    }

    /// Traite une requête : `method` (sans `.view`, en minuscules), paramètres, adresse réelle du client, agent.
    /// Bloquant.
    pub fn handle(&self, hub: &Arc<Hub>, method: &str, params: &Params, ip: &str, agent: &str) -> SubResult<Outcome> {
        let ip = ip.strip_prefix("::ffff:").unwrap_or(ip).to_string();
        let mode = if params.get("apiKey").is_some_and(|s| !s.is_empty()) {
            "clé d’API"
        } else if params.get("t").is_some_and(|s| !s.is_empty()) {
            "jeton"
        } else if params.get("p").is_some() {
            "mot de passe"
        } else {
            "aucun"
        };
        let cut = |s: Option<&str>, n: usize| s.unwrap_or("").chars().take(n).collect::<String>();
        let who = json!({
            "ip": ip, "method": method, "mode": mode, "user": cut(params.get("u"), 40), "client": cut(params.get("c"), 40),
            "version": cut(params.get("v"), 12), "agent": cut(Some(agent), 80),
        });
        let with = |extra: Value| {
            let mut w = who.clone();
            if let (Some(w), Value::Object(e)) = (w.as_object_mut(), extra) {
                w.extend(e);
            }
            w
        };
        if hub.subsonic_throttle.wait(&ip) > 0 {
            self.note(with(json!({ "ok": false, "reason": "trop d’essais ratés depuis cette adresse (limiteur)" })));
            return Err(sub_err(code::WRONG_CREDS, "Trop d’essais : patientez quelques minutes."));
        }
        let user = match self.authenticate(hub, params) {
            Ok(u) => u,
            Err(err) => {
                if err.code == code::WRONG_CREDS || err.code == code::BAD_KEY {
                    hub.subsonic_throttle.fail(&ip);
                }
                let mut reason = err.reason.clone().unwrap_or_else(|| err.message.clone());
                // Le plus fréquent : le mot de passe du compte saisi à la place du mot de passe d'application (noté ici
                // seulement, jamais dit au client).
                let user = params.get("u").unwrap_or("");
                if err.code == code::WRONG_CREDS
                    && mode == "mot de passe"
                    && !user.is_empty()
                    && let Some(p) = params.get("p").filter(|p| !p.starts_with("enc:"))
                    && hub.accounts.check(user, p).is_some()
                {
                    reason = "c’est le mot de passe du COMPTE qui a été saisi : il faut le mot de passe d’application (Mon profil → Applications compatibles Subsonic)".into();
                }
                self.note(with(json!({ "ok": false, "reason": reason })));
                return Err(err);
            }
        };
        if method == "ping" {
            self.note(with(json!({ "user": user.username, "ok": true, "reason": "connexion acceptée" })));
        }
        if params.get("c").is_none_or(str::is_empty) && method != "ping" {
            return Err(sub_err(code::MISSING, "Paramètre manquant : c."));
        }
        if !methods::exists(method) {
            return Err(sub_err(code::GENERIC, format!("Méthode inconnue : {method}")));
        }
        let entry = hub.entry(&user).map_err(|e| sub_err(code::GENERIC, e.message))?;
        let ctx = Ctx { hub, user, params: params.clone(), entry };
        methods::call(self, &ctx, method)
    }

    // ─── Outils des méthodes ───────────────────────────────────────────────

    pub fn model(&self, entry: &Entry) -> Arc<Model> {
        let library = &entry.core.library;
        let mut slot = entry.sub_model.lock();
        if let Some(m) = slot.as_ref()
            && m.rev == library.rev()
        {
            return m.clone();
        }
        let m = Arc::new(Model::build(library));
        *slot = Some(m.clone());
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_attributes_children_and_escapes() {
        let album = json!({ "id": "al-1", "name": "Nuits & <jours>", "song": [{ "id": "1", "title": "A\"B" }, { "id": "2" }], "vide": null });
        assert_eq!(
            to_xml("album", &album, &[]),
            "<album id=\"al-1\" name=\"Nuits &amp; &lt;jours&gt;\"><song id=\"1\" title=\"A&quot;B\"/><song id=\"2\"/></album>"
        );
        assert_eq!(to_xml("x", &json!({ "extra": { "x": 1 } }), &[]), "<x><extra x=\"1\"/></x>");
        assert_eq!(
            to_xml("lyrics", &json!({ "artist": "Aya", "value": "Ligne 1\nLigne 2" }), &[]),
            "<lyrics artist=\"Aya\">Ligne 1\nLigne 2</lyrics>"
        );
        let (kind, body) = render(&Params::default(), Ok(&json!({ "license": { "valid": true } })));
        assert_eq!(kind, "text/xml; charset=utf-8");
        assert!(body.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<subsonic-response xmlns=\"http://subsonic.org/restapi\" status=\"ok\" version=\"1.16.1\" type=\"control\""));
        assert!(body.contains("<license valid=\"true\"/>"));
        let p = Params(vec![("f".into(), "jsonp".into()), ("callback".into(), "cb".into())]);
        assert!(render(&p, Err(&sub_err(40, "non"))).1.starts_with("cb({\"subsonic-response\""));
    }
}

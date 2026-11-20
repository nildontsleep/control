//! Comptes et sessions. Mots de passe hachés avec scrypt (mêmes paramètres et même encodage que `server/auth.js` :
//! `scrypt:N:r:p:sel:empreinte` en hexadécimal, les empreintes existantes se vérifient), sessions à jeton aléatoire
//! dont seule l'empreinte est gardée : une déconnexion, un compte désactivé ou supprimé les coupe aussitôt.

use std::collections::HashMap;
use std::sync::LazyLock;

use base64::Engine as _;
use parking_lot::Mutex;
use regex::Regex;
use rusqlite::{OptionalExtension, Row, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::serverdb::ServerDb;
use crate::util::{HttpResult, http_error, now_ms};

const N: u32 = 16384;
const R: u32 = 8;
const P: u32 = 1;
const KEYLEN: usize = 64;

pub static USERNAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_.-]{3,24}$").expect("regex constante"));

pub fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn scrypt_raw(password: &[u8], salt: &[u8], n: u32, r: u32, p: u32, len: usize) -> Option<Vec<u8>> {
    if n < 2 || !n.is_power_of_two() || len == 0 || len > 1024 {
        return None;
    }
    let params = scrypt::Params::new(n.trailing_zeros() as u8, r, p, len).ok()?;
    let mut out = vec![0u8; len];
    scrypt::scrypt(password, salt, &params, &mut out).ok()?;
    Some(out)
}

/// Dérive une clé de `len` octets (paramètres par défaut de `crypto.scryptSync` : N = 16384, r = 8, p = 1).
pub fn scrypt_key(password: &[u8], salt: &[u8], len: usize) -> Vec<u8> {
    scrypt_raw(password, salt, N, R, P, len).unwrap_or_default()
}

pub fn hash_password(password: &str) -> String {
    let salt: [u8; 16] = rand::random();
    let hash = scrypt_raw(password.as_bytes(), &salt, N, R, P, KEYLEN).unwrap_or_default();
    format!("scrypt:{N}:{R}:{P}:{}:{}", hex::encode(salt), hex::encode(hash))
}

/// Comparaison en temps constant.
pub fn safe_equal(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn verify_password(password: &str, stored: &str) -> bool {
    let parts: Vec<&str> = stored.split(':').collect();
    let [kind, n, r, p, salt, hash] = parts.as_slice() else { return false };
    if *kind != "scrypt" || hash.is_empty() {
        return false;
    }
    let (Ok(expected), Ok(salt)) = (hex::decode(hash), hex::decode(salt)) else { return false };
    let (Ok(n), Ok(r), Ok(p)) = (n.parse::<u32>(), r.parse::<u32>(), p.parse::<u32>()) else { return false };
    let Some(actual) = scrypt_raw(password.as_bytes(), &salt, n, r, p, expected.len()) else { return false };
    safe_equal(&actual, &expected)
}

/// Limite les tentatives : `max` échecs par fenêtre de `window_ms`.
pub struct Throttle {
    max: usize,
    window_ms: i64,
    hits: Mutex<HashMap<String, Vec<i64>>>,
}

impl Throttle {
    pub fn new(max: usize, window_ms: i64) -> Throttle {
        Throttle { max, window_ms, hits: Mutex::new(HashMap::new()) }
    }

    fn prune(&self, hits: &mut HashMap<String, Vec<i64>>, key: &str, now: i64) -> Vec<i64> {
        let list: Vec<i64> =
            hits.get(key).map(|l| l.iter().copied().filter(|t| now - t < self.window_ms).collect()).unwrap_or_default();
        if list.is_empty() {
            hits.remove(key);
        } else {
            hits.insert(key.to_string(), list.clone());
        }
        list
    }

    /// Secondes à attendre, ou 0 si la tentative est permise.
    pub fn wait(&self, key: &str) -> i64 {
        self.wait_at(key, now_ms())
    }

    pub fn wait_at(&self, key: &str, now: i64) -> i64 {
        let mut hits = self.hits.lock();
        let list = self.prune(&mut hits, key, now);
        if list.len() < self.max {
            return 0;
        }
        (list[0] + self.window_ms - now + 999) / 1000
    }

    pub fn fail(&self, key: &str) {
        let now = now_ms();
        let mut hits = self.hits.lock();
        let mut list = self.prune(&mut hits, key, now);
        list.push(now);
        hits.insert(key.to_string(), list);
    }

    pub fn clear(&self, key: &str) {
        self.hits.lock().remove(key);
    }
}

/// Une ligne de la table `users`.
#[derive(Debug, Clone, Default)]
pub struct User {
    pub id: String,
    pub username: String,
    pub password_hash: String,
    pub role: String,
    pub disabled: bool,
    pub created_at: i64,
    pub last_login: Option<i64>,
    pub display_name: String,
    pub bio: String,
    pub avatar: Option<String>,
    pub color: Option<String>,
    pub folders: String,
    pub share_listening: bool,
    /// Identifiant public de la session qui demande (début de l'empreinte du jeton), quand il y en a une.
    pub sid: String,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }

    pub fn from_row(r: &Row<'_>) -> rusqlite::Result<User> {
        Ok(User {
            id: r.get("id")?,
            username: r.get("username")?,
            password_hash: r.get("password_hash")?,
            role: r.get("role")?,
            disabled: r.get::<_, i64>("disabled")? != 0,
            created_at: r.get("created_at")?,
            last_login: r.get("last_login")?,
            display_name: r.get("display_name")?,
            bio: r.get("bio")?,
            avatar: r.get("avatar")?,
            color: r.get("color")?,
            folders: r.get("folders")?,
            share_listening: r.get::<_, i64>("share_listening")? != 0,
            sid: String::new(),
        })
    }

    /// Ce que l'administration voit d'un compte créé (`publicUser`).
    pub fn public(&self) -> Value {
        json!({
            "id": self.id, "username": self.username, "role": self.role, "disabled": self.disabled,
            "createdAt": self.created_at, "lastLogin": self.last_login,
        })
    }
}

pub struct Accounts {
    db: ServerDb,
    session_ms: i64,
}

fn password_ok(password: &str) -> bool {
    let n = password.encode_utf16().count();
    (8..=200).contains(&n)
}

const LAST_ADMIN: &str = "Il faut garder au moins un administrateur actif.";

impl Accounts {
    pub fn new(db: ServerDb, session_days: f64) -> Accounts {
        Accounts { db, session_ms: (session_days * 24.0 * 3600.0 * 1000.0) as i64 }
    }

    pub fn count(&self) -> i64 {
        self.db.lock().query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0)).unwrap_or(0)
    }

    pub fn admin_count(&self) -> i64 {
        self.db
            .lock()
            .query_row("SELECT COUNT(*) FROM users WHERE role = 'admin' AND disabled = 0", [], |r| r.get(0))
            .unwrap_or(0)
    }

    pub fn by_name(&self, name: &str) -> Option<User> {
        self.db.lock().query_row("SELECT * FROM users WHERE username = ?", [name], User::from_row).optional().ok()?
    }

    pub fn by_id(&self, id: &str) -> Option<User> {
        self.db.lock().query_row("SELECT * FROM users WHERE id = ?", [id], User::from_row).optional().ok()?
    }

    /// Tous les comptes, du plus ancien au plus récent (tels que l'administration les voit).
    pub fn list(&self) -> Vec<Value> {
        let db = self.db.lock();
        let Ok(mut stmt) = db.prepare(
            "SELECT u.id, u.username, u.role, u.disabled, u.created_at, u.last_login, u.display_name, u.avatar,
                    u.color, u.folders,
                    (SELECT COUNT(*) FROM sessions s WHERE s.user_id = u.id AND s.expires_at > ?) AS sessions
             FROM users u ORDER BY u.created_at",
        ) else {
            return Vec::new();
        };
        stmt.query_map([now_ms()], |r| {
            let folders: String = r.get(9)?;
            let list: Value = serde_json::from_str(&folders).ok().filter(Value::is_array).unwrap_or(json!([]));
            Ok(json!({
                "id": r.get::<_, String>(0)?, "username": r.get::<_, String>(1)?, "role": r.get::<_, String>(2)?,
                "disabled": r.get::<_, i64>(3)? != 0, "createdAt": r.get::<_, i64>(4)?,
                "lastLogin": r.get::<_, Option<i64>>(5)?, "displayName": r.get::<_, String>(6)?,
                "avatar": r.get::<_, Option<String>>(7)?, "color": r.get::<_, Option<String>>(8)?,
                "folders": list, "sessions": r.get::<_, i64>(10)?,
            }))
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    /// Identifiants de tous les comptes.
    pub fn ids(&self) -> Vec<String> {
        let db = self.db.lock();
        let Ok(mut stmt) = db.prepare("SELECT id FROM users") else { return Vec::new() };
        stmt.query_map([], |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    pub fn create(&self, username: &str, password: &str, role: &str) -> HttpResult<User> {
        if !USERNAME.is_match(username) {
            return Err(http_error(
                400,
                "invalid_username",
                "Le nom doit faire 3 à 24 caractères : lettres, chiffres, « _ », « . » ou « - ».",
            ));
        }
        if !password_ok(password) {
            return Err(http_error(400, "invalid_password", "Le mot de passe doit faire au moins 8 caractères."));
        }
        if self.by_name(username).is_some() {
            return Err(http_error(409, "username_taken", "Ce nom est déjà pris."));
        }
        let id = hex::encode(rand::random::<[u8; 8]>());
        let hash = hash_password(password);
        let role = if role == "admin" { "admin" } else { "user" };
        let inserted = self.db.lock().execute(
            "INSERT INTO users (id, username, password_hash, role, created_at) VALUES (?, ?, ?, ?, ?)",
            params![id, username, hash, role, now_ms()],
        );
        match inserted {
            Ok(_) => self.by_id(&id).ok_or_else(|| http_error(500, "error", "Compte introuvable.")),
            Err(_) => Err(http_error(409, "username_taken", "Ce nom est déjà pris.")),
        }
    }

    pub fn set_password(&self, id: &str, password: &str) -> HttpResult<()> {
        if !password_ok(password) {
            return Err(http_error(400, "invalid_password", "Le mot de passe doit faire au moins 8 caractères."));
        }
        let hash = hash_password(password);
        let _ = self.db.lock().execute("UPDATE users SET password_hash = ? WHERE id = ?", params![hash, id]);
        self.revoke_user(id);
        Ok(())
    }

    /// Rôle et état d'un compte (seuls les champs fournis changent).
    pub fn update(&self, id: &str, role: Option<&str>, disabled: Option<bool>) -> HttpResult<()> {
        let user = self.by_id(id).ok_or_else(|| http_error(404, "not_found", "Compte introuvable."))?;
        let next_role = match role {
            Some(r @ ("admin" | "user")) => r.to_string(),
            _ => user.role.clone(),
        };
        let next_disabled = disabled.unwrap_or(user.disabled);
        // Il doit toujours rester un administrateur actif.
        if user.is_admin() && !user.disabled && (next_role != "admin" || next_disabled) && self.admin_count() <= 1 {
            return Err(http_error(409, "last_admin", LAST_ADMIN));
        }
        let _ = self.db.lock().execute(
            "UPDATE users SET role = ?, disabled = ? WHERE id = ?",
            params![next_role, next_disabled as i64, id],
        );
        if next_disabled {
            self.revoke_user(id);
        }
        Ok(())
    }

    pub fn remove(&self, id: &str) -> HttpResult<()> {
        let user = self.by_id(id).ok_or_else(|| http_error(404, "not_found", "Compte introuvable."))?;
        if user.is_admin() && !user.disabled && self.admin_count() <= 1 {
            return Err(http_error(409, "last_admin", LAST_ADMIN));
        }
        let _ = self.db.lock().execute("DELETE FROM users WHERE id = ?", [id]);
        Ok(())
    }

    /// Vérifie les identifiants ; renvoie le compte ou `None` (même durée de calcul si le nom n'existe pas).
    pub fn check(&self, username: &str, password: &str) -> Option<User> {
        let user = self.by_name(username);
        let fallback = format!("scrypt:{N}:{R}:{P}:{}:{}", "0".repeat(32), "0".repeat(KEYLEN * 2));
        let stored = user.as_ref().map(|u| u.password_hash.clone()).unwrap_or(fallback);
        let ok = verify_password(password, &stored);
        user.filter(|u| ok && !u.disabled)
    }

    // ─── Sessions ──────────────────────────────────────────────────────────

    /// Ouvre une session : (jeton, durée en secondes).
    pub fn open(&self, user_id: &str, user_agent: &str, ip: &str) -> (String, i64) {
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        let now = now_ms();
        let ua: String = user_agent.chars().take(200).collect();
        let ip: String = ip.chars().take(60).collect();
        let db = self.db.lock();
        let _ = db.execute(
            "INSERT INTO sessions (token_hash, user_id, created_at, expires_at, last_seen, user_agent, ip)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![sha256_hex(&token), user_id, now, now + self.session_ms, now, ua, ip],
        );
        let _ = db.execute("UPDATE users SET last_login = ? WHERE id = ?", params![now, user_id]);
        (token, self.session_ms / 1000)
    }

    /// Le compte de cette session, ou `None`. La session se prolonge tant qu'elle sert.
    pub fn lookup(&self, token: &str) -> Option<User> {
        if token.is_empty() {
            return None;
        }
        let hash = sha256_hex(token);
        let now = now_ms();
        let db = self.db.lock();
        let (expires, last_seen, mut user) = db
            .query_row(
                "SELECT s.expires_at, s.last_seen, u.* FROM sessions s JOIN users u ON u.id = s.user_id
                 WHERE s.token_hash = ?",
                [&hash],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, User::from_row(r)?)),
            )
            .optional()
            .ok()??;
        if expires < now || user.disabled {
            return None;
        }
        if now - last_seen > 60 * 1000 {
            let _ = db.execute(
                "UPDATE sessions SET last_seen = ?, expires_at = ? WHERE token_hash = ?",
                params![now, now + self.session_ms, hash],
            );
        }
        user.sid = hash[..16].to_string();
        Some(user)
    }

    /// Les sessions ouvertes d'un compte, la plus récente d'abord.
    pub fn sessions_of(&self, user_id: &str, current: &str) -> Vec<Value> {
        let db = self.db.lock();
        let Ok(mut stmt) = db.prepare(
            "SELECT substr(token_hash, 1, 16), created_at, last_seen, expires_at, user_agent, ip FROM sessions
             WHERE user_id = ? AND expires_at > ? ORDER BY last_seen DESC",
        ) else {
            return Vec::new();
        };
        stmt.query_map(params![user_id, now_ms()], |r| {
            let id: String = r.get(0)?;
            Ok(json!({
                "id": id, "createdAt": r.get::<_, i64>(1)?, "lastSeen": r.get::<_, i64>(2)?,
                "expiresAt": r.get::<_, i64>(3)?, "userAgent": r.get::<_, Option<String>>(4)?,
                "ip": r.get::<_, Option<String>>(5)?, "current": id == current,
            }))
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    /// Ferme une session d'un compte (jamais celle d'un autre).
    pub fn revoke_session(&self, user_id: &str, id: &str) -> bool {
        self.db
            .lock()
            .execute("DELETE FROM sessions WHERE user_id = ? AND substr(token_hash, 1, 16) = ?", [user_id, id])
            .unwrap_or(0)
            > 0
    }

    /// Ferme toutes les sessions d'un compte sauf celle-ci.
    pub fn revoke_others(&self, user_id: &str, current: &str) -> usize {
        self.db
            .lock()
            .execute("DELETE FROM sessions WHERE user_id = ? AND substr(token_hash, 1, 16) <> ?", [user_id, current])
            .unwrap_or(0)
    }

    pub fn close(&self, token: &str) {
        if !token.is_empty() {
            let _ = self.db.lock().execute("DELETE FROM sessions WHERE token_hash = ?", [sha256_hex(token)]);
        }
    }

    pub fn revoke_user(&self, id: &str) {
        let _ = self.db.lock().execute("DELETE FROM sessions WHERE user_id = ?", [id]);
    }

    pub fn purge(&self) {
        let _ = self.db.lock().execute("DELETE FROM sessions WHERE expires_at < ?", [now_ms()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_verify_like_node() {
        let h = hash_password("long enough password");
        assert!(h.starts_with("scrypt:16384:8:1:"));
        assert!(verify_password("long enough password", &h));
        assert!(!verify_password("autre", &h));
        assert!(!verify_password("x", "bcrypt:1"));
        // Empreinte produite par Node : crypto.scrypt('motdepasse', Buffer.from('00112233445566778899aabbccddeeff', 'hex'), 64).
        let stored = "scrypt:16384:8:1:00112233445566778899aabbccddeeff:9b515168ae358b7fe4906bd2cf88ed63f3a0fb9d4c13c1\
                      92b340ab5ab231633fd80cb97592c861901f75dc0c89ae59d55810ec4c1a28af2b0761f45a20b2bdb1";
        assert!(verify_password("motdepasse", stored));
        assert!(!verify_password("motdepass", stored));
    }

    #[test]
    fn throttle_counts_failures_in_window() {
        let t = Throttle::new(2, 1000);
        assert_eq!(t.wait("a"), 0);
        t.fail("a");
        t.fail("a");
        assert!(t.wait("a") > 0);
        assert_eq!(t.wait("b"), 0);
        assert_eq!(t.wait_at("a", now_ms() + 2000), 0);
        t.fail("a");
        t.fail("a");
        t.clear("a");
        assert_eq!(t.wait("a"), 0);
    }

    #[test]
    fn accounts_sessions_and_last_admin() {
        let dir = tempfile::tempdir().unwrap();
        let db = std::sync::Arc::new(Mutex::new(crate::serverdb::open(&dir.path().join("server.db")).unwrap()));
        let a = Accounts::new(db, 30.0);
        assert_eq!(a.create("x", "long enough password", "admin").unwrap_err().code, "invalid_username");
        assert_eq!(a.create("nil", "court", "admin").unwrap_err().status, 400);
        let nil = a.create("nil", "long enough password", "admin").unwrap();
        assert_eq!(a.create("NIL", "long enough password", "user").unwrap_err().status, 409);
        assert!(a.check("Nil", "long enough password").is_some());
        assert!(a.check("nil", "faux mot de passe").is_none());
        assert!(a.check("personne", "long enough password").is_none());
        let (token, max_age) = a.open(&nil.id, "essai", "127.0.0.1");
        assert_eq!(max_age, 30 * 24 * 3600);
        let me = a.lookup(&token).unwrap();
        assert_eq!((me.username.as_str(), me.sid.len()), ("nil", 16));
        assert_eq!(a.sessions_of(&nil.id, &me.sid)[0]["current"], true);
        assert_eq!(a.update(&nil.id, Some("user"), None).unwrap_err().code, "last_admin");
        assert_eq!(a.remove(&nil.id).unwrap_err().status, 409);
        a.close(&token);
        assert!(a.lookup(&token).is_none());
        assert_eq!(a.list()[0]["folders"], json!([]));
    }
}

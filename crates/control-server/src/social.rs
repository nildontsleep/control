//! Profils et partages : le profil de chaque compte, et qui peut parcourir la bibliothèque d'un compte (browse) ou
//! comparer ses récaps avec lui (compare). Un compte partage avec un autre compte précis, ou avec tout le monde ('*').

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::auth::{Accounts, User};
use crate::serverdb::ServerDb;
use crate::util::{HttpResult, http_error};
use control_core::core::{image_from_data_url, text};

pub const EVERYONE: &str = "*";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Access {
    pub browse: bool,
    pub compare: bool,
    pub own: bool,
}

impl Access {
    pub fn to_json(self) -> Value {
        if self.own {
            json!({ "browse": true, "compare": true, "self": true })
        } else {
            json!({ "browse": self.browse, "compare": self.compare })
        }
    }
}

pub struct Social {
    db: ServerDb,
    accounts: Arc<Accounts>,
    avatars_dir: PathBuf,
}

/// Image carrée (recadrée au centre) de `size` px de côté au plus, en JPEG. `None` si le format n'est pas décodable.
pub fn square_jpeg(data: &[u8], size: u32) -> Option<Vec<u8>> {
    let img = image::ImageReader::new(Cursor::new(data)).with_guessed_format().ok()?.decode().ok()?;
    let (w, h) = (img.width(), img.height());
    let side = w.min(h);
    let cropped = if w == h { img } else { img.crop_imm((w - side) / 2, (h - side) / 2, side, side) };
    let scaled =
        if side > size { cropped.resize_exact(size, size, image::imageops::FilterType::Lanczos3) } else { cropped };
    let jpeg = control_core::covers::encode_jpeg(&scaled, 88);
    (!jpeg.is_empty()).then_some(jpeg)
}

/// Enregistre un JPEG dans `dir` sous une empreinte de 20 caractères ; renvoie l'empreinte.
pub fn save_by_hash(dir: &std::path::Path, jpeg: &[u8]) -> std::io::Result<String> {
    let hash = control_core::covers::hash_of(jpeg);
    std::fs::create_dir_all(dir)?;
    let file = dir.join(format!("{hash}.jpg"));
    if !file.exists() {
        std::fs::write(file, jpeg)?;
    }
    Ok(hash)
}

impl Social {
    pub fn new(db: ServerDb, accounts: Arc<Accounts>, avatars_dir: PathBuf) -> Social {
        Social { db, accounts, avatars_dir }
    }

    /// Ce que n'importe quel compte connecté voit d'un autre : nom, avatar, présentation.
    pub fn profile(user: &User) -> Value {
        json!({
            "id": user.id,
            "username": user.username,
            "displayName": user.display_name,
            "name": if user.display_name.is_empty() { &user.username } else { &user.display_name },
            "bio": user.bio,
            "avatar": user.avatar,
            "color": user.color,
            "role": user.role,
            "createdAt": user.created_at,
        })
    }

    pub fn set_profile(&self, id: &str, patch: &Value) -> HttpResult<Value> {
        let row = self.accounts.by_id(id).ok_or_else(|| http_error(404, "not_found", "Compte introuvable."))?;
        let (mut display_name, mut bio, mut color, mut avatar) = (row.display_name, row.bio, row.color, row.avatar);
        if let Some(v) = patch.get("displayName") {
            display_name = text(v).trim().chars().take(40).collect();
        }
        if let Some(v) = patch.get("bio") {
            bio = text(v).trim().chars().take(280).collect();
        }
        if let Some(v) = patch.get("color") {
            let c = text(v).trim().to_string();
            let valid = c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit());
            if !c.is_empty() && !valid {
                return Err(http_error(400, "invalid_color", "Couleur invalide."));
            }
            color = (!c.is_empty()).then_some(c);
        }
        match patch.get("avatar") {
            Some(Value::Null) => avatar = None,
            Some(src @ Value::Object(_)) => {
                let data = image_from_data_url(&src["data"]).ok_or_else(|| {
                    http_error(400, "invalid_image", "Image refusée (JPEG, PNG ou WebP, 12 Mo au plus).")
                })?;
                let jpeg = square_jpeg(&data, 320).ok_or_else(|| {
                    http_error(
                        400,
                        "invalid_image",
                        "Ce format d’image n’est pas pris en charge (utilisez JPEG ou PNG).",
                    )
                })?;
                avatar =
                    Some(save_by_hash(&self.avatars_dir, &jpeg).map_err(|e| http_error(500, "error", e.to_string()))?);
            }
            _ => {}
        }
        let _ = self.db.lock().execute(
            "UPDATE users SET display_name = ?, bio = ?, color = ?, avatar = ? WHERE id = ?",
            rusqlite::params![display_name, bio, color, avatar, id],
        );
        let row = self.accounts.by_id(id).ok_or_else(|| http_error(404, "not_found", "Compte introuvable."))?;
        Ok(Social::profile(&row))
    }

    // ─── Partages ──────────────────────────────────────────────────────────

    /// Les partages d'un compte : `[{ viewerId, browse, compare }]`.
    pub fn grants_of(&self, owner: &str) -> Value {
        let db = self.db.lock();
        let Ok(mut stmt) =
            db.prepare("SELECT viewer_id, browse, compare FROM shares WHERE owner_id = ? ORDER BY viewer_id")
        else {
            return json!([]);
        };
        let rows: Vec<Value> = stmt
            .query_map([owner], |r| {
                Ok(json!({
                    "viewerId": r.get::<_, String>(0)?, "browse": r.get::<_, i64>(1)? != 0,
                    "compare": r.get::<_, i64>(2)? != 0,
                }))
            })
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default();
        Value::Array(rows)
    }

    /// Remplace les partages d'un compte. Les entrées sans aucun droit, vers lui-même ou vers un compte inconnu sont
    /// ignorées.
    pub fn set_grants(&self, owner: &str, list: &Value) -> HttpResult<Value> {
        if self.accounts.by_id(owner).is_none() {
            return Err(http_error(404, "not_found", "Compte introuvable."));
        }
        let known = self.accounts.ids();
        let mut clean: BTreeMap<String, (bool, bool)> = BTreeMap::new();
        for item in list.as_array().map(|a| a.iter().take(500).collect::<Vec<_>>()).unwrap_or_default() {
            let viewer = match item.get("viewerId") {
                Some(Value::Null) | None => String::new(),
                Some(v) => text(v),
            };
            if viewer == owner || (viewer != EVERYONE && !known.contains(&viewer)) {
                continue;
            }
            let browse = control_core::core::truthy(&item["browse"]);
            let compare = control_core::core::truthy(&item["compare"]);
            if browse || compare {
                clean.insert(viewer, (browse, compare));
            }
        }
        {
            let mut db = self.db.lock();
            let tx = db.transaction().map_err(|e| http_error(500, "error", e.to_string()))?;
            let _ = tx.execute("DELETE FROM shares WHERE owner_id = ?", [owner]);
            for (viewer, (b, c)) in &clean {
                let _ = tx.execute(
                    "INSERT INTO shares (owner_id, viewer_id, browse, compare) VALUES (?, ?, ?, ?)",
                    rusqlite::params![owner, viewer, *b as i64, *c as i64],
                );
            }
            tx.commit().map_err(|e| http_error(500, "error", e.to_string()))?;
        }
        Ok(self.grants_of(owner))
    }

    /// Ce que `viewer` peut faire avec le compte `owner`.
    pub fn access(&self, viewer: &User, owner: &User) -> Access {
        if viewer.id == owner.id {
            return Access { browse: true, compare: true, own: true };
        }
        if owner.disabled {
            return Access::default();
        }
        // L'administrateur parcourt toutes les bibliothèques, pas les statistiques.
        let mut access = Access { browse: viewer.is_admin(), ..Access::default() };
        let db = self.db.lock();
        if let Ok(mut stmt) =
            db.prepare("SELECT browse, compare FROM shares WHERE owner_id = ? AND viewer_id IN (?, ?)")
            && let Ok(rows) = stmt.query_map([&owner.id, &viewer.id, &EVERYONE.to_string()], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })
        {
            for (b, c) in rows.flatten() {
                access.browse |= b != 0;
                access.compare |= c != 0;
            }
        }
        access
    }

    /// Les autres comptes actifs, par nom.
    pub fn other_users(&self, viewer: &str) -> Vec<User> {
        let db = self.db.lock();
        let Ok(mut stmt) =
            db.prepare("SELECT * FROM users WHERE disabled = 0 AND id <> ? ORDER BY username COLLATE NOCASE")
        else {
            return Vec::new();
        };
        stmt.query_map([viewer], User::from_row).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    /// Les autres comptes, avec ce que `viewer` peut en faire.
    pub fn others(&self, viewer: &User) -> Vec<(User, Value, Access)> {
        self.other_users(&viewer.id)
            .into_iter()
            .map(|u| {
                let access = self.access(viewer, &u);
                let mut p = Social::profile(&u);
                p["access"] = access.to_json();
                (u, p, access)
            })
            .collect()
    }

    /// Supprime tout ce qui concerne un compte supprimé (les partages vers lui).
    pub fn forget(&self, id: &str) {
        let _ = self.db.lock().execute("DELETE FROM shares WHERE viewer_id = ?", [id]);
    }
}

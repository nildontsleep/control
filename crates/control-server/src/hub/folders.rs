//! Dossiers de musique : communs à tous les comptes, propres à un compte, téléchargements ; analyse et ménage.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Entry, Hub};
use crate::auth::User;
use crate::util::{HttpResult, http_error, inside, path_cmp_key, resolve};

impl Hub {
    /// Dossiers communs : tous les comptes les voient dans leur bibliothèque.
    pub fn common_folders(&self) -> Vec<String> {
        self.setting("libraryFolders")
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    }

    /// Dossiers propres à un compte (attribués par l'administrateur).
    pub fn own_folders(&self, user_id: &str) -> Vec<String> {
        self.accounts.by_id(user_id).map(|u| parse_folders(&u.folders)).unwrap_or_default()
    }

    /// Dossier des téléchargements d'un compte : un sous-dossier à son nom.
    pub fn download_dir_for(&self, user: &User) -> PathBuf {
        PathBuf::from(self.setting_str("downloadDir")).join(&user.username)
    }

    pub fn download_dir_of(&self, user_id: &str) -> PathBuf {
        match self.accounts.by_id(user_id) {
            Some(u) => self.download_dir_for(&u),
            None => PathBuf::from(self.setting_str("downloadDir")).join(user_id),
        }
    }

    /// Dossiers analysés pour la bibliothèque d'un compte : les siens, ses téléchargements, et les dossiers communs.
    pub fn roots_for(&self, user_id: &str) -> Vec<String> {
        let Some(user) = self.accounts.by_id(user_id) else { return Vec::new() };
        let mut roots = self.common_folders();
        roots.extend(parse_folders(&user.folders));
        let dl = self.download_dir_for(&user);
        let dl_key = resolve(&dl).to_string_lossy().into_owned();
        if dl.exists() && !roots.iter().any(|r| dl_key.starts_with(&*resolve(r).to_string_lossy())) {
            roots.push(dl.to_string_lossy().into_owned());
        }
        roots
    }

    /// Dossiers qui n'appartiennent qu'à ce compte : ce que les autres voient de sa bibliothèque.
    pub fn owner_roots(&self, user: &User) -> Vec<String> {
        let mut roots = parse_folders(&user.folders);
        let dl = self.download_dir_for(user);
        if dl.exists() {
            roots.push(dl.to_string_lossy().into_owned());
        }
        roots
    }

    pub fn is_inside_roots(&self, user_id: &str, file: &str) -> bool {
        inside(&self.roots_for(user_id), file)
    }

    /// Reporte les dossiers dans les réglages d'un compte chargé (pour l'affichage) ; la source reste le serveur.
    pub fn mirror(&self, entry: &Entry) {
        let patch = json!({
            "libraryFolders": self.own_folders(&entry.id),
            "commonFolders": self.common_folders(),
            "downloadDir": self.download_dir_of(&entry.id).to_string_lossy(),
        });
        entry.core.store.patch(&patch);
        entry.emit("settings:changed", &patch);
    }

    pub(super) fn sync_folders(&self) {
        for e in self.loaded_entries() {
            self.mirror(&e);
        }
        self.watch();
        self.rescan_all();
    }

    pub fn check_folder(dir: &str) -> HttpResult<String> {
        let resolved = resolve(if dir.is_empty() { "." } else { dir });
        let meta = std::fs::metadata(&resolved)
            .map_err(|_| http_error(400, "not_found", "Ce dossier n’existe pas sur le serveur."))?;
        if !meta.is_dir() {
            return Err(http_error(400, "not_a_folder", "Ce n’est pas un dossier."));
        }
        if resolved.parent().is_none() {
            return Err(http_error(400, "too_broad", "Choisissez un dossier plutôt que la racine du disque."));
        }
        Ok(resolved.to_string_lossy().into_owned())
    }

    pub fn add_folder(&self, dir: &str) -> HttpResult<Value> {
        let resolved = Hub::check_folder(dir)?;
        let mut list = self.common_folders();
        if !list.contains(&resolved) {
            list.push(resolved);
            self.patch_settings(&json!({ "libraryFolders": list }))?;
        }
        Ok(json!(self.common_folders()))
    }

    pub fn remove_folder(&self, dir: &str) -> HttpResult<Value> {
        let list: Vec<String> = self.common_folders().into_iter().filter(|f| f != dir).collect();
        self.patch_settings(&json!({ "libraryFolders": list }))?;
        Ok(json!(self.common_folders()))
    }

    fn save_own_folders(&self, user_id: &str, list: Vec<String>) -> Value {
        let text = serde_json::to_string(&list).unwrap_or_else(|_| "[]".into());
        let _ = self.db.lock().execute("UPDATE users SET folders = ? WHERE id = ?", [&text, user_id]);
        if let Some(entry) = self.loaded(user_id) {
            self.mirror(&entry);
            self.refresh(&entry, self.roots_for(user_id));
        }
        self.watch();
        json!(list)
    }

    pub fn add_own_folder(&self, user_id: &str, dir: &str) -> HttpResult<Value> {
        if self.accounts.by_id(user_id).is_none() {
            return Err(http_error(404, "not_found", "Compte introuvable."));
        }
        let resolved = Hub::check_folder(dir)?;
        let mut list = self.own_folders(user_id);
        if list.contains(&resolved) {
            return Ok(json!(list));
        }
        list.push(resolved);
        Ok(self.save_own_folders(user_id, list))
    }

    pub fn remove_own_folder(&self, user_id: &str, dir: &str) -> Value {
        let list = self.own_folders(user_id).into_iter().filter(|f| f != dir).collect();
        self.save_own_folders(user_id, list)
    }

    /// Crée le dossier de musique d'un compte dans le dossier des comptes (si l'administrateur en a défini un).
    pub fn ensure_personal_folder(&self, user: &User) -> HttpResult<Value> {
        let root = self.setting_str("usersRoot");
        if root.is_empty() {
            return Err(http_error(
                400,
                "no_users_root",
                "Aucun dossier des comptes n’est défini dans les réglages du serveur.",
            ));
        }
        let dir = resolve(&root).join(&user.username);
        std::fs::create_dir_all(&dir).map_err(|e| {
            http_error(400, "mkdir_failed", format!("Impossible de créer {} : {}", dir.display(), e.kind()))
        })?;
        self.add_own_folder(&user.id, &dir.to_string_lossy())
    }

    /// Sous-dossiers d'un dossier du serveur, pour le sélecteur de l'administrateur.
    pub fn browse(&self, dir: &str) -> HttpResult<Value> {
        let target = if dir.is_empty() {
            std::env::current_dir()
                .ok()
                .and_then(|d| d.ancestors().last().map(Path::to_path_buf))
                .unwrap_or_else(|| "/".into())
        } else {
            resolve(dir)
        };
        let entries = std::fs::read_dir(&target).map_err(|_| http_error(400, "unreadable", "Dossier illisible."))?;
        let mut dirs: Vec<String> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .collect();
        dirs.sort_by_key(|a| a.to_lowercase());
        let parent = target.parent().map(|p| p.to_string_lossy().into_owned());
        Ok(json!({ "path": target.to_string_lossy(), "parent": parent, "dirs": dirs }))
    }

    // ─── Analyse ───────────────────────────────────────────────────────────

    /// Retire de la bibliothèque d'un compte les morceaux qui ne sont plus dans un dossier du serveur (les fichiers
    /// restent), puis analyse ses dossiers. En arrière-plan.
    pub fn refresh(&self, entry: &std::sync::Arc<Entry>, roots: Vec<String>) {
        let entry = entry.clone();
        let spawned = std::thread::Builder::new().name("control-scan".into()).spawn(move || {
            let library = &entry.core.library;
            let ids: Vec<i64> =
                library.tracks().into_iter().filter(|t| !inside_any(&roots, &t.path)).map(|t| t.id).collect();
            if !ids.is_empty() {
                library.remove_tracks(&ids, false);
            }
            library.scan(&roots);
        });
        if let Err(err) = spawned {
            log::error!("Analyse de la bibliothèque : {err}");
        }
    }

    pub fn rescan_all(&self) {
        for e in self.loaded_entries() {
            self.refresh(&e, self.roots_for(&e.id));
        }
    }

    /// Tous les dossiers surveillés : communs, propres à un compte, téléchargements.
    pub(super) fn all_roots(&self) -> Vec<String> {
        let mut roots = self.common_folders();
        let rows: Vec<(String, String, String)> = {
            let db = self.db.lock();
            db.prepare("SELECT id, username, folders FROM users WHERE disabled = 0")
                .and_then(|mut s| {
                    s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map(|r| r.flatten().collect())
                })
                .unwrap_or_default()
        };
        for (_, username, folders) in rows {
            roots.extend(parse_folders(&folders));
            let dl = PathBuf::from(self.setting_str("downloadDir")).join(username);
            if dl.exists() {
                roots.push(dl.to_string_lossy().into_owned());
            }
        }
        roots.sort();
        roots.dedup();
        roots
    }
}

/// Le morceau est-il sous un de ces dossiers (comparaison de préfixes, comme l'original) ?
fn inside_any(roots: &[String], file: &str) -> bool {
    let key = path_cmp_key(&resolve(file));
    let sep = std::path::MAIN_SEPARATOR;
    roots.iter().any(|r| key.starts_with(&format!("{}{sep}", path_cmp_key(&resolve(r)).trim_end_matches(sep))))
}

pub fn parse_folders(text: &str) -> Vec<String> {
    serde_json::from_str::<Vec<Value>>(text)
        .map(|v| v.into_iter().filter_map(|x| x.as_str().filter(|s| !s.is_empty()).map(str::to_string)).collect())
        .unwrap_or_default()
}

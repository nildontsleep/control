//! Envoi de musique depuis le navigateur : le fichier arrive en flux (jamais en mémoire), dans le dossier du compte
//! (`<téléchargements>/<nom>/Envois`), sous un nom nettoyé, sans écraser l'existant ; il n'est gardé que si c'est
//! bien un média. Repris de `server/upload.js`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use axum::body::Body;
use futures_util::StreamExt as _;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt as _;
use unicode_normalization::UnicodeNormalization;

use crate::hub::Entry;
use crate::util::{HttpResult, http_error};

/// De la place gardée libre sur le disque.
const RESERVE_BYTES: u64 = 200 * 1024 * 1024;
/// Envois simultanés par compte.
const MAX_PER_USER: usize = 3;
const MAX_DEPTH: usize = 4;

const RESERVED: &[&str] = &["con", "prn", "aux", "nul"];

/// Un nom de fichier sûr : ni chemin, ni caractère interdit, pas de point ou d'espace au bord, 180 caractères au plus.
pub fn clean_segment(raw: &str, keep_ext: bool) -> Option<String> {
    let replaced: String = raw
        .nfc()
        .map(|c| if "\\/:*?\"<>|".contains(c) || (c as u32) < 0x20 || c == '\x7f' { '_' } else { c })
        .collect();
    let edge = |c: char| c == '.' || c.is_whitespace();
    let mut name = replaced.trim_matches(edge).to_string();
    let stem = name.split('.').next().unwrap_or("").to_lowercase();
    let device = RESERVED.contains(&stem.as_str())
        || ((stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if name.is_empty() || device {
        return None;
    }
    // Longueur en unités UTF-16, comme l'original.
    if name.encode_utf16().count() > 180 {
        let ext = if keep_ext {
            Path::new(&name).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default()
        } else {
            String::new()
        };
        let keep = 180usize.saturating_sub(ext.encode_utf16().count());
        let mut out = String::new();
        let mut n = 0;
        for c in name.chars() {
            if n + c.len_utf16() > keep {
                break;
            }
            n += c.len_utf16();
            out.push(c);
        }
        name = out + &ext;
    }
    Some(name)
}

/// Le sous-dossier demandé (« Album/Disc 1 ») : segments nettoyés, quatre niveaux au plus ; vide si rien de valable.
pub fn clean_dir(raw: &str) -> Vec<String> {
    raw.split(['/', '\\']).filter(|p| !p.is_empty()).filter_map(|p| clean_segment(p, false)).take(MAX_DEPTH).collect()
}

/// Un nom libre dans ce dossier : « titre.mp3 », sinon « titre (2).mp3 »…
pub fn free_name(dir: &Path, name: &str) -> String {
    let ext = Path::new(name).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let base = &name[..name.len() - ext.len()];
    for i in 1..500 {
        let candidate = if i == 1 { name.to_string() } else { format!("{base} ({i}){ext}") };
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("{base}-{}{ext}", hex::encode(rand::random::<[u8; 3]>()))
}

static ACTIVE: LazyLock<Mutex<HashMap<String, usize>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Compte un envoi en cours tant qu'il vit.
struct Slot(String);

impl Drop for Slot {
    fn drop(&mut self) {
        let mut active = ACTIVE.lock();
        if let Some(n) = active.get_mut(&self.0) {
            *n -= 1;
            if *n == 0 {
                active.remove(&self.0);
            }
        }
    }
}

fn too_large(max: u64) -> crate::util::HttpError {
    http_error(413, "too_large", format!("Fichier trop volumineux ({} Mo au plus).", (max as f64 / 1048576.0).round()))
}

fn unreadable() -> crate::util::HttpError {
    http_error(415, "unreadable", "Ce fichier n’a pas pu être lu comme un morceau ou une vidéo.")
}

/// Place libre sur le disque qui contient `dir`, si on sait la mesurer.
fn free_space(dir: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        let out = std::process::Command::new("df").arg("-Pk").arg(dir).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().nth(1)?;
        line.split_whitespace().nth(3)?.parse::<u64>().ok().map(|k| k * 1024)
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        None
    }
}

/// Reçoit un fichier. Renvoie `{ name, size, trackIds }`.
pub async fn receive(
    body: Body,
    entry: Arc<Entry>,
    root: PathBuf,
    name: &str,
    subdir: &str,
    declared: Option<u64>,
    max: u64,
) -> HttpResult<Value> {
    let base =
        clean_segment(name, true).filter(|b| control_core::probe::is_media_file(Path::new(b))).ok_or_else(|| {
            http_error(
                415,
                "unsupported",
                "Seuls les fichiers audio et vidéo sont acceptés (mp3, flac, m4a, ogg, opus, wav, mp4…).",
            )
        })?;
    let _slot = {
        let mut active = ACTIVE.lock();
        let n = active.entry(entry.id.clone()).or_insert(0);
        if *n >= MAX_PER_USER {
            return Err(http_error(429, "too_many", "Trop d’envois en même temps : patientez un instant."));
        }
        *n += 1;
        Slot(entry.id.clone())
    };
    if declared.is_some_and(|d| d > max) {
        return Err(too_large(max));
    }
    let mut dir = root;
    for part in clean_dir(subdir) {
        dir.push(part);
    }
    tokio::fs::create_dir_all(&dir).await.map_err(|e| http_error(500, "error", e.to_string()))?;
    let probe_dir = dir.clone();
    if let Ok(Some(free)) = tokio::task::spawn_blocking(move || free_space(&probe_dir)).await
        && declared.unwrap_or(0) + RESERVE_BYTES > free
    {
        return Err(http_error(507, "no_space", "Plus assez de place sur le disque du serveur."));
    }
    let final_name = free_name(&dir, &base);
    let part = dir.join(format!(".{final_name}.{}.part", hex::encode(rand::random::<[u8; 4]>())));
    let res = async {
        let mut file = tokio::fs::File::create(&part).await.map_err(|e| http_error(500, "error", e.to_string()))?;
        let mut size = 0u64;
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| http_error(400, "aborted", "Envoi interrompu."))?;
            size += chunk.len() as u64;
            if size > max {
                return Err(too_large(max));
            }
            file.write_all(&chunk).await.map_err(|e| http_error(500, "error", e.to_string()))?;
        }
        file.flush().await.map_err(|e| http_error(500, "error", e.to_string()))?;
        drop(file);
        if size == 0 {
            return Err(http_error(400, "empty", "Fichier vide."));
        }
        let fin = dir.join(&final_name);
        tokio::fs::rename(&part, &fin).await.map_err(|e| http_error(500, "error", e.to_string()))?;
        // La bibliothèque garde les fichiers illisibles sous leur nom ; un envoi, lui, doit être un vrai média.
        let entry = entry.clone();
        let check = fin.clone();
        let ids = tokio::task::spawn_blocking(move || {
            let p = control_core::probe::probe(&check);
            if p.codec.is_none() && p.duration <= 0.0 {
                return Vec::new();
            }
            entry.core.library.add_paths(&[check.to_string_lossy().into_owned()], None)
        })
        .await
        .unwrap_or_default();
        if ids.is_empty() {
            let _ = tokio::fs::remove_file(&fin).await;
            return Err(unreadable());
        }
        Ok(json!({ "name": final_name, "size": size, "trackIds": ids }))
    }
    .await;
    if res.is_err() {
        let _ = tokio::fs::remove_file(&part).await;
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_folders_are_neutralised() {
        assert_eq!(clean_segment("01 - Titre: Remix?.mp3", true).as_deref(), Some("01 - Titre_ Remix_.mp3"));
        assert_eq!(clean_segment("../../etc/passwd.mp3", false).as_deref(), Some("_.._etc_passwd.mp3"));
        assert_eq!(clean_segment("  .caché.mp3  ", false).as_deref(), Some("caché.mp3"));
        assert_eq!(clean_segment("CON.mp3", false), None);
        assert_eq!(clean_segment("com1", false), None);
        assert_eq!(clean_segment("..", false), None);
        assert_eq!(clean_segment("", false), None);
        let long = clean_segment(&format!("{}.flac", "a".repeat(300)), true).unwrap();
        assert!(long.ends_with(".flac") && long.len() <= 180);
        assert_eq!(clean_dir("Album/../Disc 1/./x"), ["Album", "Disc 1", "x"]);
        assert_eq!(clean_dir("a/b/c/d/e/f"), ["a", "b", "c", "d"]);
        assert_eq!(clean_dir("../.."), Vec::<String>::new());
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(free_name(dir.path(), "a.mp3"), "a.mp3");
        std::fs::write(dir.path().join("a.mp3"), "").unwrap();
        assert_eq!(free_name(dir.path(), "a.mp3"), "a (2).mp3");
    }
}

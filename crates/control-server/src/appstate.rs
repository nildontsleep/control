//! Versions du serveur installées par la mise à jour automatique : où elles sont rangées et laquelle lancer.
//! Partagé par le lanceur (qui choisit la version et revient en arrière) et par le serveur (qui en télécharge de
//! nouvelles). Le format de `state.json` reste celui de l'original : champs inconnus conservés.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

/// Nom du binaire dans le dossier d'une version.
pub fn binary_name() -> String {
    control_core::process::exe_name("control-server")
}

/// Compare deux versions « 1.2.3 » (préfixe « v » et suffixe de préversion ignorés).
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |v: &str| -> Vec<i64> {
        let v = v.trim_start_matches(['v', 'V']);
        let v = v.split('-').next().unwrap_or("");
        v.split('.').map(|n| n.parse::<i64>().unwrap_or(0)).collect()
    };
    let (x, y) = (parts(a), parts(b));
    for i in 0..x.len().max(y.len()) {
        let d = x.get(i).copied().unwrap_or(0).cmp(&y.get(i).copied().unwrap_or(0));
        if d.is_ne() {
            return d;
        }
    }
    std::cmp::Ordering::Equal
}

/// « 1.2.3 » exactement.
pub fn is_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

pub struct Paths {
    pub root: PathBuf,
    pub versions: PathBuf,
    pub tmp: PathBuf,
    pub state: PathBuf,
}

pub fn paths(data_dir: &Path) -> Paths {
    let root = data_dir.join("app");
    Paths { versions: root.join("versions"), tmp: root.join("tmp"), state: root.join("state.json"), root }
}

/// L'état (`target`, `previous`, `bad`, `failure`…) ; `bad` ne garde que des versions valides.
pub fn read_state(data_dir: &Path) -> Map<String, Value> {
    let mut state = std::fs::read_to_string(paths(data_dir).state)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let bad: Vec<Value> = state
        .get("bad")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter(|v| v.as_str().is_some_and(is_version)).cloned().collect())
        .unwrap_or_default();
    state.insert("bad".into(), Value::Array(bad));
    state
}

pub fn bad_versions(state: &Map<String, Value>) -> Vec<String> {
    state
        .get("bad")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

pub fn str_of(state: &Map<String, Value>, key: &str) -> Option<String> {
    state.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Écriture atomique (un arrêt en plein milieu ne laisse jamais un fichier à moitié écrit).
pub fn write_state(data_dir: &Path, patch: Value) -> std::io::Result<Map<String, Value>> {
    let file = paths(data_dir).state;
    let mut next = read_state(data_dir);
    if let Value::Object(p) = patch {
        next.extend(p);
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(&Value::Object(next.clone())).unwrap_or_else(|_| "{}".into()))?;
    std::fs::rename(&tmp, &file)?;
    Ok(next)
}

/// Dossier d'une version installée si elle est complète (extraite et vérifiée), sinon `None`.
pub fn version_dir(data_dir: &Path, version: &str) -> Option<PathBuf> {
    if !is_version(version) {
        return None;
    }
    let dir = paths(data_dir).versions.join(version);
    (dir.join(".ok").exists() && dir.join(binary_name()).exists()).then_some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cmp::Ordering;

    #[test]
    fn versions_compare_and_state_roundtrips() {
        assert_eq!(compare_versions("0.10.0", "0.9.9"), Ordering::Greater);
        assert_eq!(compare_versions("v1.0.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("0.9.1", "0.9.2"), Ordering::Less);
        assert_eq!(compare_versions("1.0.0-beta", "1.0"), Ordering::Equal);
        assert!(is_version("1.2.3") && !is_version("1.2") && !is_version("1.2.x") && !is_version("../1.2.3"));

        let dir = tempfile::tempdir().unwrap();
        assert_eq!(bad_versions(&read_state(dir.path())), Vec::<String>::new());
        write_state(dir.path(), json!({ "target": "1.2.3", "bad": ["0.1.0", "n'importe quoi"], "autre": 1 })).unwrap();
        let st = read_state(dir.path());
        assert_eq!(str_of(&st, "target").as_deref(), Some("1.2.3"));
        assert_eq!(bad_versions(&st), ["0.1.0"]);
        write_state(dir.path(), json!({ "target": null })).unwrap();
        assert_eq!(read_state(dir.path())["autre"], 1, "champs inconnus conservés");
        assert_eq!(version_dir(dir.path(), "1.2.3"), None);
        let v = paths(dir.path()).versions.join("1.2.3");
        std::fs::create_dir_all(&v).unwrap();
        std::fs::write(v.join(".ok"), "").unwrap();
        std::fs::write(v.join(binary_name()), "").unwrap();
        assert_eq!(version_dir(dir.path(), "1.2.3"), Some(v));
    }
}

//! Configuration du serveur web : variables d'environnement (voir le README, section « Version web »).

use std::collections::HashMap;
use std::path::PathBuf;

/// Mêmes réglages que `server/config.js`.
#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub host: String,
    /// Tout ce que le serveur écrit : comptes, réglages, bibliothèques, pochettes, outils.
    pub data_dir: PathBuf,
    /// Dossiers de musique proposés au premier lancement (séparés par « : » sous Linux, « ; » sous Windows).
    pub library_folders: Vec<String>,
    /// Derrière un reverse proxy : fait confiance à X-Forwarded-For / X-Forwarded-Proto.
    pub trust_proxy: bool,
    /// « auto » : cookie Secure dès que la connexion est en HTTPS.
    pub cookie_secure: String,
    /// Minutes entre deux analyses complètes des dossiers.
    pub rescan_minutes: f64,
    /// Durée d'une session de connexion, en jours.
    pub session_days: f64,
    /// Clé TMDB fournie d'office aux comptes qui n'ont pas la leur.
    pub tmdb_key: String,
    /// Si défini, la création du premier compte exige ce code.
    pub setup_token: String,
    /// « off », « notify » ou « auto » force le réglage de mise à jour (vide : choisi dans /admin).
    pub update: String,
    /// Jeton GitHub en lecture, si le dépôt des releases est privé.
    pub update_token: String,
    /// Essais : toute adresse de webhook acceptée (sinon Discord seulement).
    pub backup_any_url: bool,
}

fn flag(v: Option<&String>) -> bool {
    v.is_some_and(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

/// Nombre positif (`Number(v) || fallback`).
fn positive(v: Option<&String>, fallback: f64) -> f64 {
    v.and_then(|s| s.trim().parse::<f64>().ok()).filter(|n| *n != 0.0 && n.is_finite()).unwrap_or(fallback)
}

impl Config {
    /// Depuis l'environnement du processus.
    pub fn from_env() -> Config {
        Config::from_map(&std::env::vars().collect())
    }

    pub fn from_map(env: &HashMap<String, String>) -> Config {
        let get = |k: &str| env.get(k).filter(|v| !v.is_empty());
        let port = get("PORT")
            .or_else(|| get("CONTROL_PORT"))
            .and_then(|p| p.trim().parse::<u16>().ok())
            .filter(|p| *p != 0)
            .unwrap_or(5180);
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let data = get("CONTROL_DATA").map(PathBuf::from).unwrap_or_else(|| cwd.join("data"));
        let data_dir = std::path::absolute(&data).unwrap_or(data);
        let sep = if cfg!(windows) { ';' } else { ':' };
        let update = get("CONTROL_UPDATE").map(|s| s.to_lowercase()).unwrap_or_default();
        Config {
            port,
            host: get("CONTROL_HOST").cloned().unwrap_or_else(|| "0.0.0.0".into()),
            data_dir,
            library_folders: get("CONTROL_LIBRARY")
                .map(|s| s.split(sep).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect())
                .unwrap_or_default(),
            trust_proxy: flag(get("CONTROL_TRUST_PROXY")),
            cookie_secure: get("CONTROL_COOKIE_SECURE").map(|s| s.to_lowercase()).unwrap_or_else(|| "auto".into()),
            rescan_minutes: positive(get("CONTROL_RESCAN_MINUTES"), 30.0),
            session_days: positive(get("CONTROL_SESSION_DAYS"), 30.0),
            tmdb_key: get("CONTROL_TMDB_KEY").cloned().unwrap_or_default(),
            setup_token: get("CONTROL_SETUP_TOKEN").cloned().unwrap_or_default(),
            update: if ["off", "notify", "auto"].contains(&update.as_str()) { update } else { String::new() },
            update_token: get("CONTROL_UPDATE_TOKEN").cloned().unwrap_or_default(),
            backup_any_url: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_original_variables() {
        let env: HashMap<String, String> = [
            ("CONTROL_PORT", "8080"),
            ("CONTROL_LIBRARY", "/a: /b :"),
            ("CONTROL_TRUST_PROXY", "yes"),
            ("CONTROL_UPDATE", "AUTO"),
            ("CONTROL_SESSION_DAYS", "abc"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let c = Config::from_map(&env);
        assert_eq!(c.port, 8080);
        assert_eq!(c.host, "0.0.0.0");
        if !cfg!(windows) {
            assert_eq!(c.library_folders, ["/a", "/b"]);
        }
        assert!(c.trust_proxy);
        assert_eq!(c.update, "auto");
        assert_eq!(c.session_days, 30.0);
        assert_eq!(c.cookie_secure, "auto");
        let d = Config::from_map(&HashMap::from([("PORT".to_string(), "0".to_string())]));
        assert_eq!(d.port, 5180);
        assert!(d.data_dir.ends_with("data"));
        assert_eq!(Config::from_map(&HashMap::from([("CONTROL_UPDATE".into(), "x".into())])).update, "");
    }
}

//! Adresse d'un serveur control saisie à la main : origines à essayer, dans l'ordre.

use std::sync::LazyLock;

use regex::Regex;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceUrlError {
    #[error("Saisissez l’adresse du serveur.")]
    Empty,
    #[error("Adresse invalide.")]
    Invalid,
    #[error("Seules les adresses http et https sont acceptées.")]
    Scheme,
}

static HAS_SCHEME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[a-z][a-z0-9+.-]*://").expect("regex constante"));

/// Nom d'hôte qui désigne vraisemblablement une machine du réseau local (http d'abord : ces serveurs n'ont
/// souvent pas de certificat).
static LOCAL_HOST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(localhost|\[.*\]|\d{1,3}(\.\d{1,3}){3}|[^.:]+|.*\.(local|lan|home|internal))(:\d+)?$")
        .expect("regex constante")
});

fn origin(text: &str) -> Result<String, SourceUrlError> {
    let url = Url::parse(text).map_err(|_| SourceUrlError::Invalid)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(SourceUrlError::Scheme);
    }
    if url.host().is_none() {
        return Err(SourceUrlError::Invalid);
    }
    Ok(url.origin().ascii_serialization())
}

/// Adresses à essayer pour ce que l'utilisateur a saisi : sans schéma, https d'abord (http d'abord pour une
/// adresse locale). Seule l'origine est gardée (ni chemin, ni identifiants).
pub fn candidates(input: &str) -> Result<Vec<String>, SourceUrlError> {
    let raw = input.trim().trim_end_matches('/');
    if raw.is_empty() {
        return Err(SourceUrlError::Empty);
    }
    if HAS_SCHEME.is_match(raw) {
        return Ok(vec![origin(raw)?]);
    }
    let host = raw.split(['/', '?', '#']).next().unwrap_or(raw);
    let schemes = if LOCAL_HOST.is_match(host) { ["http", "https"] } else { ["https", "http"] };
    schemes.iter().map(|scheme| origin(&format!("{scheme}://{raw}"))).collect()
}

/// Vérifie qu'une origine est bien un serveur control (`/api/health` répond `{ ok: true, name: "control" }`).
pub fn probe(origin: &str) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(6)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut res = agent.get(&format!("{origin}/api/health")).call().map_err(|e| {
        let text = e.to_string();
        if text.to_lowercase().contains("certificate") || text.contains("tls") {
            "Le certificat du serveur n’est pas accepté.".to_string()
        } else {
            "Impossible de joindre le serveur.".to_string()
        }
    })?;
    let body: serde_json::Value = res.body_mut().read_json().unwrap_or(serde_json::Value::Null);
    if res.status().is_success() && is_control_health(&body) {
        Ok(())
    } else {
        Err("Ce n’est pas un serveur control.".into())
    }
}

/// La réponse de `/api/health` est-elle celle d'un serveur control ?
pub fn is_control_health(body: &serde_json::Value) -> bool {
    body.get("ok").and_then(serde_json::Value::as_bool) == Some(true)
        && body.get("name").and_then(serde_json::Value::as_str) == Some("control")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> Vec<String> {
        candidates(s).unwrap()
    }

    #[test]
    fn full_addresses_keep_their_origin() {
        assert_eq!(c("https://music.example.com/"), ["https://music.example.com"]);
        assert_eq!(c("http://192.168.1.20:5180/admin?x=1#y"), ["http://192.168.1.20:5180"]);
        assert_eq!(c("http://moi:secret@nas.local:5180"), ["http://nas.local:5180"]);
        assert_eq!(c("HTTPS://Music.Example.com:443/x"), ["https://music.example.com"]);
    }

    #[test]
    fn without_scheme_order_depends_on_the_host() {
        assert_eq!(c("music.example.com"), ["https://music.example.com", "http://music.example.com"]);
        assert_eq!(c("192.168.1.20:5180"), ["http://192.168.1.20:5180", "https://192.168.1.20:5180"]);
        assert_eq!(c("localhost:5180"), ["http://localhost:5180", "https://localhost:5180"]);
        assert_eq!(c("nas"), ["http://nas", "https://nas"]);
        assert_eq!(c("maison.local"), ["http://maison.local", "https://maison.local"]);
        assert_eq!(c("[::1]:5180"), ["http://[::1]:5180", "https://[::1]:5180"]);
        assert_eq!(c("  music.example.com/admin//  "), ["https://music.example.com", "http://music.example.com"]);
    }

    #[test]
    fn refused() {
        assert_eq!(candidates(""), Err(SourceUrlError::Empty));
        assert_eq!(candidates("   "), Err(SourceUrlError::Empty));
        assert_eq!(candidates("///"), Err(SourceUrlError::Empty));
        assert_eq!(candidates("ftp://example.com"), Err(SourceUrlError::Scheme));
        assert_eq!(candidates("file:///etc/passwd"), Err(SourceUrlError::Scheme));
        assert_eq!(candidates("exa mple.com"), Err(SourceUrlError::Invalid));
        assert!(SourceUrlError::Scheme.to_string().contains("http"));
    }

    #[test]
    fn health_body_is_recognised() {
        assert!(is_control_health(&serde_json::json!({"ok": true, "name": "control", "version": "1.0.0"})));
        assert!(!is_control_health(&serde_json::json!({"ok": true, "name": "other"})));
        assert!(!is_control_health(&serde_json::Value::Null));
    }
}

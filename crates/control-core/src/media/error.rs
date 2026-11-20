//! Erreurs du service films et séries. Le code (`nokey`, `auth`, `network`…) est celui que l'interface d'origine
//! traduit ; il traverse l'IPC (et le serveur web) dans `{ ok: false, code, message }`.

use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorCode {
    /// Aucune clé TMDB configurée.
    NoKey,
    /// Clé refusée (401, 403).
    Auth,
    NotFound,
    /// Trop de requêtes (429).
    RateLimit,
    /// Autre réponse en erreur, source ou type inconnu.
    Http,
    /// Réseau indisponible.
    Network,
    Timeout,
    /// Erreur interne (base de données…).
    Error,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::NoKey => "nokey",
            ErrorCode::Auth => "auth",
            ErrorCode::NotFound => "notfound",
            ErrorCode::RateLimit => "ratelimit",
            ErrorCode::Http => "http",
            ErrorCode::Network => "network",
            ErrorCode::Timeout => "timeout",
            ErrorCode::Error => "error",
        }
    }

    /// Erreurs passagères : le cache périmé fait alors l'affaire.
    pub fn is_transient(self) -> bool {
        matches!(self, ErrorCode::Network | ErrorCode::Timeout | ErrorCode::RateLimit)
    }

    pub fn for_status(status: u16) -> ErrorCode {
        match status {
            401 | 403 => ErrorCode::Auth,
            404 => ErrorCode::NotFound,
            429 => ErrorCode::RateLimit,
            _ => ErrorCode::Http,
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct MediaError {
    pub code: ErrorCode,
    pub message: String,
    /// Statut HTTP (0 hors réponse HTTP).
    pub status: u16,
}

impl MediaError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> MediaError {
        MediaError { code, message: message.into(), status: 0 }
    }

    pub fn no_key() -> MediaError {
        MediaError::new(ErrorCode::NoKey, "Aucune clé TMDB")
    }

    pub fn unknown_source(source: &str) -> MediaError {
        MediaError::new(ErrorCode::Http, format!("Source inconnue : {source}"))
    }
}

impl From<rusqlite::Error> for MediaError {
    fn from(err: rusqlite::Error) -> MediaError {
        log::error!("[media] base : {err}");
        MediaError::new(ErrorCode::Error, err.to_string())
    }
}

/// Enveloppe d'une réponse, comme les gestionnaires IPC d'origine : `{ ok: true, data }` ou
/// `{ ok: false, code, message }`.
pub fn envelope<T: Serialize>(result: Result<T, MediaError>) -> Value {
    match result {
        Ok(data) => match serde_json::to_value(data) {
            Ok(data) => json!({ "ok": true, "data": data }),
            Err(err) => json!({ "ok": false, "code": "error", "message": err.to_string() }),
        },
        Err(err) => json!({ "ok": false, "code": err.code.as_str(), "message": err.message }),
    }
}

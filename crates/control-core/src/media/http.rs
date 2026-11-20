//! Requêtes JSON vers les catalogues (TMDB, AniList) : délai de 10 s, erreurs typées.
//!
//! Le transport est un trait ([`Fetch`]) : les tests branchent des réponses enregistrées, sans réseau.

use std::time::Duration;

use serde_json::Value;

use super::error::{ErrorCode, MediaError};

const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Option<String>,
}

/// Transport : envoie une requête et rend le corps JSON (ou `Null` si le corps n'est pas du JSON).
pub trait Fetch: Send + Sync {
    fn fetch(&self, req: &HttpRequest) -> Result<Value, MediaError>;
}

/// Transport réel, avec `ureq`.
pub struct UreqFetch {
    agent: ureq::Agent,
}

impl Default for UreqFetch {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .user_agent("control")
            .build()
            .into();
        UreqFetch { agent }
    }
}

impl Fetch for UreqFetch {
    fn fetch(&self, req: &HttpRequest) -> Result<Value, MediaError> {
        let response = match req.method {
            Method::Get => {
                let mut r = self.agent.get(&req.url);
                for (k, v) in &req.headers {
                    r = r.header(*k, v);
                }
                r.call()
            }
            Method::Post => {
                let mut r = self.agent.post(&req.url);
                for (k, v) in &req.headers {
                    r = r.header(*k, v);
                }
                r.send(req.body.as_deref().unwrap_or(""))
            }
        };
        let mut response = response.map_err(transport_error)?;
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().map_err(transport_error)?;
        interpret(status, &text)
    }
}

fn transport_error(err: ureq::Error) -> MediaError {
    match err {
        ureq::Error::Timeout(_) => MediaError::new(ErrorCode::Timeout, "Délai dépassé"),
        other => {
            let message = other.to_string();
            MediaError::new(ErrorCode::Network, if message.is_empty() { "Réseau indisponible".into() } else { message })
        }
    }
}

/// Analyse une réponse : JSON si possible, erreur typée si le statut n'est pas 2xx (avec le message de TMDB ou
/// d'AniList quand il y en a un).
pub fn interpret(status: u16, text: &str) -> Result<Value, MediaError> {
    let data: Value = if text.is_empty() { Value::Null } else { serde_json::from_str(text).unwrap_or(Value::Null) };
    if !(200..300).contains(&status) {
        let detail = data
            .get("status_message")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| data.pointer("/errors/0/message").and_then(Value::as_str).filter(|s| !s.is_empty()))
            .map(str::to_string)
            .unwrap_or_else(|| format!("HTTP {status}"));
        return Err(MediaError { code: ErrorCode::for_status(status), message: detail, status });
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_map_to_codes() {
        let e = interpret(401, r#"{"status_message":"Invalid API key"}"#).unwrap_err();
        assert_eq!((e.code, e.message.as_str(), e.status), (ErrorCode::Auth, "Invalid API key", 401));
        let e = interpret(404, "").unwrap_err();
        assert_eq!((e.code, e.message.as_str()), (ErrorCode::NotFound, "HTTP 404"));
        let e = interpret(429, r#"{"errors":[{"message":"Too Many Requests."}]}"#).unwrap_err();
        assert_eq!((e.code, e.message.as_str()), (ErrorCode::RateLimit, "Too Many Requests."));
        assert_eq!(interpret(500, "<html>").unwrap_err().code, ErrorCode::Http);
        assert_eq!(interpret(200, r#"{"a":1}"#).unwrap()["a"], 1);
        assert_eq!(interpret(200, "pas du json").unwrap(), Value::Null);
    }
}

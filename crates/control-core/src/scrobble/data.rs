//! Contenu de `scrobble.json` (même format que la version d'origine) et état envoyé à l'interface.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastfmApp {
    pub key: String,
    pub secret: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListenBrainzAccount {
    pub token: String,
    #[serde(default)]
    pub user: Option<String>,
    /// Serveur compatible (sinon celui de ListenBrainz).
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LastfmAccount {
    pub session: String,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Ce qu'on envoie d'un morceau.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Play {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Secondes (entières dans la file, comme l'original).
    #[serde(serialize_with = "whole_if_integral")]
    pub duration: f64,
    pub track_no: Option<i64>,
}

/// `215` plutôt que `215.0`, pour écrire le même fichier que la version d'origine.
fn whole_if_integral<S: serde::Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
    if v.fract() == 0.0 && v.abs() < 1e15 { s.serialize_i64(*v as i64) } else { s.serialize_f64(*v) }
}

/// Une écoute en attente d'envoi.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Queued {
    /// Début de l'écoute, en secondes Unix.
    pub at: i64,
    pub play: Play,
    /// Services auxquels il reste à l'envoyer (`listenbrainz`, `lastfm`).
    pub todo: Vec<String>,
    /// Identifiant en mémoire, pour retrouver l'écoute après un envoi fait sans verrou.
    #[serde(skip)]
    pub(super) seq: u64,
}

/// Contenu de `scrobble.json`.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrobbleData {
    pub listenbrainz: Option<ListenBrainzAccount>,
    pub lastfm: Option<LastfmAccount>,
    pub lastfm_app: Option<LastfmApp>,
    pub queue: Vec<Queued>,
}

impl ScrobbleData {
    /// Lecture tolérante : une partie abîmée est ignorée, le reste est gardé.
    pub fn parse(text: &str) -> ScrobbleData {
        let Ok(Value::Object(raw)) = serde_json::from_str::<Value>(text) else { return ScrobbleData::default() };
        let field = |k: &str| raw.get(k).cloned().unwrap_or(Value::Null);
        let queue = match raw.get("queue") {
            Some(Value::Array(items)) => {
                items.iter().filter_map(|q| serde_json::from_value::<Queued>(q.clone()).ok()).collect()
            }
            _ => Vec::new(),
        };
        ScrobbleData {
            listenbrainz: serde_json::from_value(field("listenbrainz")).ok().flatten(),
            lastfm: serde_json::from_value(field("lastfm")).ok().flatten(),
            lastfm_app: serde_json::from_value(field("lastfmApp")).ok().flatten(),
            queue,
        }
    }

    /// Services connectés, dans l'ordre d'envoi.
    pub fn services(&self) -> Vec<Service> {
        let mut out = Vec::new();
        if self.listenbrainz.as_ref().is_some_and(|a| !a.token.is_empty()) {
            out.push(Service::ListenBrainz);
        }
        if self.lastfm.as_ref().is_some_and(|a| !a.session.is_empty()) {
            out.push(Service::Lastfm);
        }
        out
    }

    pub(super) fn set_error(&mut self, service: Service, error: Option<String>) {
        match service {
            Service::ListenBrainz => {
                if let Some(a) = &mut self.listenbrainz {
                    a.error = error;
                }
            }
            Service::Lastfm => {
                if let Some(a) = &mut self.lastfm {
                    a.error = error;
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    ListenBrainz,
    Lastfm,
}

impl Service {
    pub fn as_str(self) -> &'static str {
        match self {
            Service::ListenBrainz => "listenbrainz",
            Service::Lastfm => "lastfm",
        }
    }

    pub fn parse(s: &str) -> Option<Service> {
        match s {
            "listenbrainz" => Some(Service::ListenBrainz),
            "lastfm" => Some(Service::Lastfm),
            _ => None,
        }
    }
}

// ─── État pour l'interface ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListenBrainzStatus {
    pub connected: bool,
    pub user: Option<String>,
    pub url: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastfmStatus {
    pub connected: bool,
    pub user: Option<String>,
    pub error: Option<String>,
    /// Une application Last.fm (clé et secret) est connue.
    pub configured: bool,
    /// Ses clés viennent du serveur (réglées par l'administrateur).
    pub keys_from_server: bool,
}

/// Payload de `scrobble:changed`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    pub listenbrainz: ListenBrainzStatus,
    pub lastfm: LastfmStatus,
    pub pending: usize,
}

/// Première étape de la connexion Last.fm : le jeton, et l'adresse où l'utilisateur autorise control.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LastfmAuth {
    pub token: String,
    pub url: String,
}

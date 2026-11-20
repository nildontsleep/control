//! Encodage et décodage des échanges avec ListenBrainz et Last.fm (fonctions pures).

use std::collections::BTreeMap;

use anyhow::bail;

use serde_json::{Map, Value, json};

use md5::{Digest, Md5};

use super::data::{LastfmApp, Play, Queued};

/// Une écoute compte : morceau de 30 s au moins, écouté pendant la moitié de sa durée ou 4 minutes.
pub fn counts(duration: f64, seconds: f64) -> bool {
    duration >= 30.0 && seconds >= (duration / 2.0).min(240.0)
}

/// Signature d'un appel Last.fm : noms et valeurs triés par nom, mis bout à bout, suivis du secret, en MD5.
/// (`format` et `callback` n'en font jamais partie : on les ajoute après.)
pub fn lastfm_signature(params: &BTreeMap<String, String>, secret: &str) -> String {
    let mut h = Md5::new();
    for (k, v) in params {
        h.update(k.as_bytes());
        h.update(v.as_bytes());
    }
    h.update(secret.as_bytes());
    hex::encode(h.finalize())
}

/// Formulaire complet d'un appel Last.fm : paramètres, clé d'API, signature (si demandée) et `format=json`.
pub fn lastfm_form(params: &BTreeMap<String, String>, app: &LastfmApp, sign: bool) -> Vec<(String, String)> {
    let mut all = params.clone();
    all.insert("api_key".into(), app.key.clone());
    if sign {
        let sig = lastfm_signature(&all, &app.secret);
        all.insert("api_sig".into(), sig);
    }
    let mut out: Vec<(String, String)> = all.into_iter().collect();
    out.push(("format".into(), "json".into()));
    out
}

/// Échec d'un envoi. `permanent` : inutile de réessayer (jeton ou session refusés…).
#[derive(Debug, Clone, PartialEq)]
pub struct SendError {
    pub message: String,
    pub permanent: bool,
    /// Secondes à attendre avant de réessayer (limite de ListenBrainz).
    pub retry_after: Option<u64>,
}

impl SendError {
    pub(super) fn temporary(message: impl Into<String>) -> SendError {
        SendError { message: message.into(), permanent: false, retry_after: None }
    }

    pub(super) fn permanent(message: impl Into<String>) -> SendError {
        SendError { message: message.into(), permanent: true, retry_after: None }
    }
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SendError {}

/// Jeton ListenBrainz plausible (`^[\w-]{8,100}$`).
pub fn valid_lb_token(token: &str) -> bool {
    (8..=100).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Métadonnées d'un morceau pour ListenBrainz.
pub fn lb_meta(play: &Play) -> Value {
    let mut info = Map::new();
    info.insert("media_player".into(), json!("control"));
    info.insert("submission_client".into(), json!("control"));
    if play.duration > 0.0 {
        info.insert("duration_ms".into(), json!((play.duration * 1000.0).round() as i64));
    }
    if let Some(n) = play.track_no.filter(|n| *n != 0) {
        info.insert("tracknumber".into(), json!(n));
    }
    let mut meta = Map::new();
    meta.insert("artist_name".into(), json!(play.artist));
    meta.insert("track_name".into(), json!(play.title));
    if !play.album.is_empty() {
        meta.insert("release_name".into(), json!(play.album));
    }
    meta.insert("additional_info".into(), Value::Object(info));
    Value::Object(meta)
}

/// Corps d'un envoi ListenBrainz : `playing_now` (sans date), `single` (une écoute) ou `import` (plusieurs).
pub fn lb_body(items: &[(Option<i64>, &Play)]) -> Value {
    let listen_type = match items {
        [(None, _)] => "playing_now",
        [_] => "single",
        _ => "import",
    };
    let payload: Vec<Value> = items
        .iter()
        .map(|(at, p)| match at {
            Some(at) => json!({ "listened_at": at, "track_metadata": lb_meta(p) }),
            None => json!({ "track_metadata": lb_meta(p) }),
        })
        .collect();
    json!({ "listen_type": listen_type, "payload": payload })
}

/// Réponse de ListenBrainz à un envoi.
pub fn lb_result(status: u16, rate_reset_in: Option<&str>) -> Result<(), SendError> {
    match status {
        200..=299 => Ok(()),
        401 => Err(SendError::permanent("Jeton ListenBrainz refusé")),
        429 => Err(SendError {
            message: "ListenBrainz limite les envois".into(),
            permanent: false,
            retry_after: Some(rate_reset_in.and_then(|s| s.trim().parse().ok()).filter(|n| *n > 0).unwrap_or(30)),
        }),
        400..=499 => Err(SendError::permanent(format!("ListenBrainz : {status}"))),
        _ => Err(SendError::temporary(format!("ListenBrainz : {status}"))),
    }
}

/// Réponse de `validate-token` : le nom de l'utilisateur si le jeton est valide.
pub fn lb_validate(status: u16, body: &str) -> anyhow::Result<Option<String>> {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    if !(200..300).contains(&status) || v.get("valid").and_then(Value::as_bool) != Some(true) {
        bail!("Jeton refusé par ListenBrainz.");
    }
    Ok(v.get("user_name").and_then(Value::as_str).map(str::to_string))
}

/// Réponse de Last.fm. Codes : 9 session invalide ; 4, 10, 26 clés ou autorisation refusées (définitifs) ;
/// 11, 16 service indisponible (à réessayer).
pub fn lastfm_result(status: u16, body: &str) -> Result<Value, SendError> {
    let v: Value = serde_json::from_str(body).unwrap_or_else(|_| json!({}));
    if let Some(code) = v.get("error").filter(|e| !e.is_null() && *e != &json!(0) && *e != &json!(false)) {
        let code_n = code.as_i64().or_else(|| code.as_str().and_then(|s| s.parse().ok()));
        let message = v
            .get("message")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Last.fm : erreur {}", code_n.map(|n| n.to_string()).unwrap_or_default()));
        let permanent = matches!(code_n, Some(4 | 9 | 10 | 26));
        return Err(SendError { message, permanent, retry_after: None });
    }
    if !(200..300).contains(&status) {
        return Err(SendError::temporary(format!("Last.fm : {status}")));
    }
    Ok(v)
}

pub(super) fn params(pairs: &[(&str, String)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect()
}

/// Paramètres de `track.updateNowPlaying`.
pub fn lastfm_now_playing_params(play: &Play, session: &str) -> BTreeMap<String, String> {
    let mut p = params(&[
        ("method", "track.updateNowPlaying".into()),
        ("artist", play.artist.clone()),
        ("track", play.title.clone()),
        ("sk", session.into()),
    ]);
    if !play.album.is_empty() {
        p.insert("album".into(), play.album.clone());
    }
    if play.duration > 0.0 {
        p.insert("duration".into(), (play.duration.round() as i64).to_string());
    }
    p
}

/// Paramètres de `track.scrobble` pour un lot (50 au plus).
pub fn lastfm_scrobble_params(items: &[Queued], session: &str) -> BTreeMap<String, String> {
    let mut p = params(&[("method", "track.scrobble".into()), ("sk", session.into())]);
    for (i, q) in items.iter().enumerate() {
        p.insert(format!("artist[{i}]"), q.play.artist.clone());
        p.insert(format!("track[{i}]"), q.play.title.clone());
        p.insert(format!("timestamp[{i}]"), q.at.to_string());
        if !q.play.album.is_empty() {
            p.insert(format!("album[{i}]"), q.play.album.clone());
        }
        if q.play.duration > 0.0 {
            p.insert(format!("duration[{i}]"), (q.play.duration.round() as i64).to_string());
        }
    }
    p
}

/// Adresse où l'utilisateur autorise control sur le site de Last.fm.
pub fn lastfm_auth_url(key: &str, token: &str) -> String {
    let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>().replace('+', "%20");
    format!("https://www.last.fm/api/auth/?api_key={}&token={}", enc(key), enc(token))
}

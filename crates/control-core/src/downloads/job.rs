//! Un téléchargement de la file : forme JSON d'origine (`downloads.json`, `dl:update`) et ses changements d'état.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use super::info::title_from_url;
use super::ytdlp::{DownloadSpec, Progress};
use crate::model::Kind;

pub type JobId = i64;

/// Message des téléchargements coupés par la fermeture de l'application.
pub const INTERRUPTED: &str = "Interrompu par la fermeture de l’application.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    #[default]
    Queued,
    Info,
    Downloading,
    Processing,
    Done,
    Canceled,
    /// Aussi pour un état inconnu lu dans le fichier.
    #[serde(other)]
    Error,
}

impl JobStatus {
    /// En file ou en cours.
    pub fn is_active(self) -> bool {
        matches!(self, JobStatus::Queued | JobStatus::Info | JobStatus::Downloading | JobStatus::Processing)
    }

    /// yt-dlp tourne pour ce téléchargement.
    pub fn is_running(self) -> bool {
        matches!(self, JobStatus::Downloading | JobStatus::Processing)
    }
}

/// Entier tolérant : un nombre décimal est tronqué, tout autre type vaut `None`.
fn lenient_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(v.as_i64().or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f as i64)))
}

fn lenient_f64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    Ok(Value::deserialize(d)?.as_f64().filter(|f| f.is_finite()))
}

fn lenient_ids<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<i64>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(v.as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default())
}

fn lenient_strings<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default())
}

/// Un téléchargement. Les champs inconnus d'un fichier plus récent sont gardés (`extra`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Job {
    pub id: JobId,
    pub url: String,
    pub title: String,
    pub thumbnail: Option<String>,
    pub is_playlist: bool,
    pub kind: Kind,
    /// Format audio (`mp3`, `m4a`, `opus`, `flac`, `best`).
    pub format: String,
    /// Débit audio en kb/s, ou hauteur maximale de la vidéo (`best` : sans limite).
    pub quality: String,
    /// Séparer « Artiste - Titre » et nettoyer le titre (audio).
    pub parse_title: bool,
    /// Playlist de l'application à créer à la fin.
    pub playlist_name: Option<String>,
    pub status: JobStatus,
    pub progress: f64,
    #[serde(deserialize_with = "lenient_f64")]
    pub speed: Option<f64>,
    #[serde(deserialize_with = "lenient_f64")]
    pub eta: Option<f64>,
    /// Titre de l'élément en cours.
    pub current: Option<String>,
    #[serde(deserialize_with = "lenient_i64")]
    pub index: Option<i64>,
    #[serde(deserialize_with = "lenient_i64")]
    pub count: Option<i64>,
    #[serde(deserialize_with = "lenient_strings")]
    pub files: Vec<String>,
    #[serde(deserialize_with = "lenient_ids")]
    pub track_ids: Vec<i64>,
    pub error: Option<String>,
    pub created_at: i64,
    pub canceled: bool,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient_i64")]
    pub playlist_id: Option<i64>,
    /// Éléments d'une playlist que yt-dlp n'a pas pu télécharger.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient_i64")]
    pub skipped: Option<i64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Job {
    pub fn spec(&self) -> DownloadSpec<'_> {
        DownloadSpec {
            url: &self.url,
            is_playlist: self.is_playlist,
            kind: self.kind,
            format: &self.format,
            quality: &self.quality,
            parse_title: self.parse_title,
        }
    }

    /// Remise à zéro avant un (nouveau) lancement.
    pub fn reset_run(&mut self) {
        self.progress = 0.0;
        self.error = None;
        self.speed = None;
        self.eta = None;
        self.canceled = false;
        self.files.clear();
        self.track_ids.clear();
    }

    /// Progression lue sur une ligne `CTRLPROG` (l'avancement ne recule jamais).
    pub fn apply_progress(&mut self, p: &Progress) {
        let n = p.count.filter(|n| *n > 0).or(self.count);
        self.status = JobStatus::Downloading;
        self.progress = self.progress.max(p.overall(self.count));
        self.speed = p.speed;
        self.eta = p.eta;
        self.index = p.index.filter(|i| *i > 0);
        self.count = n;
        if p.title.is_some() {
            self.current = p.title.clone();
        }
    }

    /// Post-traitement (conversion, pochette) d'un élément.
    pub fn apply_post(&mut self, title: Option<String>) {
        self.status = JobStatus::Processing;
        if title.is_some() {
            self.current = title;
        }
        self.speed = None;
        self.eta = None;
    }

    /// Fin de yt-dlp : code de sortie (-1 s'il n'a pas pu démarrer ou a été tué), dernier message d'erreur, playlist
    /// créée.
    pub fn finish(&mut self, code: i32, last_error: Option<String>, playlist_id: Option<i64>) {
        self.speed = None;
        self.eta = None;
        if self.canceled {
            self.status = JobStatus::Canceled;
        } else if !self.files.is_empty() {
            let got = self.files.len() as i64;
            let skipped = if code != 0 { (self.count.unwrap_or(got) - got).max(0) } else { 0 };
            self.status = JobStatus::Done;
            self.progress = 1.0;
            self.playlist_id = playlist_id;
            self.skipped = Some(skipped);
            self.error = if skipped > 0 { last_error } else { None };
        } else {
            self.status = JobStatus::Error;
            self.error = Some(last_error.unwrap_or_else(|| {
                if code == 0 { "yt-dlp:NOFILES".to_string() } else { format!("yt-dlp:EXIT:{code}") }
            }));
        }
    }
}

/// Ce que l'interface envoie pour ajouter un téléchargement (`dl:add`). Lu sans exiger de types exacts.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AddOptions {
    pub url: String,
    pub title: Option<String>,
    pub thumbnail: Option<String>,
    pub is_playlist: bool,
    pub kind: Kind,
    pub format: Option<String>,
    pub quality: Option<String>,
    pub parse_title: bool,
    /// `None` : réglage `dl.createPlaylist` (pour une playlist seulement).
    pub create_playlist: Option<bool>,
    pub playlist_name: Option<String>,
    pub count: Option<i64>,
}

impl AddOptions {
    pub fn from_value(v: &Value) -> AddOptions {
        let s = |k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
        let truthy = |k: &str| match v.get(k) {
            Some(Value::Bool(b)) => *b,
            Some(Value::Null) | None => false,
            Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
            Some(Value::String(s)) => !s.is_empty(),
            Some(_) => true,
        };
        AddOptions {
            url: s("url").unwrap_or_default(),
            title: s("title"),
            thumbnail: s("thumbnail"),
            is_playlist: truthy("isPlaylist"),
            kind: if v.get("kind").and_then(Value::as_str) == Some("video") { Kind::Video } else { Kind::Audio },
            format: s("format"),
            quality: s("quality").or_else(|| v.get("quality").and_then(Value::as_i64).map(|n| n.to_string())),
            parse_title: v.get("parseTitle") != Some(&Value::Bool(false)),
            create_playlist: v.get("createPlaylist").filter(|x| !x.is_null()).map(|_| truthy("createPlaylist")),
            playlist_name: s("playlistName"),
            count: v.get("count").and_then(Value::as_f64).filter(|n| *n > 0.0).map(|n| n as i64),
        }
    }

    /// Nouveau téléchargement en file. `default_create_playlist` : réglage `dl.createPlaylist`.
    pub fn into_job(self, id: JobId, created_at: i64, default_create_playlist: bool) -> Job {
        let create = self.create_playlist.unwrap_or(default_create_playlist && self.is_playlist);
        let playlist_name = create.then(|| {
            self.playlist_name.clone().or_else(|| self.title.clone()).unwrap_or_else(|| "Téléchargements".into())
        });
        let title = self.title.or_else(|| title_from_url(&self.url)).unwrap_or_else(|| self.url.clone());
        Job {
            id,
            url: self.url,
            title,
            thumbnail: self.thumbnail,
            is_playlist: self.is_playlist,
            kind: self.kind,
            format: self.format.unwrap_or_else(|| "mp3".into()),
            quality: self.quality.unwrap_or_else(|| "320".into()),
            parse_title: self.parse_title,
            playlist_name,
            status: JobStatus::Queued,
            count: self.count,
            created_at,
            ..Job::default()
        }
    }
}

/// Lit `downloads.json` (`{ jobs: [...] }`). Les téléchargements en cours à la fermeture passent en erreur ; une
/// entrée illisible est ignorée.
pub fn load_jobs(text: &str) -> Vec<Job> {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    let Some(list) = v.get("jobs").and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .filter_map(|j| serde_json::from_value::<Job>(j.clone()).ok())
        .map(|mut job| {
            if job.status.is_active() {
                job.status = JobStatus::Error;
                job.error = Some(INTERRUPTED.into());
                job.speed = None;
                job.eta = None;
            }
            job
        })
        .collect()
}

/// Contenu de `downloads.json` : les 80 derniers.
pub fn save_jobs(jobs: &[Job]) -> String {
    let start = jobs.len().saturating_sub(80);
    serde_json::to_string(&serde_json::json!({ "jobs": &jobs[start..] })).unwrap_or_else(|_| "{\"jobs\":[]}".into())
}

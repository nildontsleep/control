//! Données de la bibliothèque. Les noms JSON (camelCase) sont ceux de la version d'origine : le serveur web les
//! envoie tels quels.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type TrackId = i64;
pub type PlaylistId = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Audio,
    Video,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Audio => "audio",
            Kind::Video => "video",
        }
    }

    pub fn parse(s: &str) -> Kind {
        if s == "video" { Kind::Video } else { Kind::Audio }
    }
}

/// Un morceau ou une vidéo de la bibliothèque.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: TrackId,
    pub path: String,
    pub kind: Kind,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: Option<i64>,
    pub track_no: Option<i64>,
    pub disc_no: Option<i64>,
    /// Secondes.
    pub duration: f64,
    /// kb/s.
    pub bitrate: Option<i64>,
    pub codec: Option<String>,
    pub size: i64,
    /// Empreinte de la pochette (fichiers `<hash>.jpg` et `<hash>-s.jpg` du dossier des pochettes).
    pub cover: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    /// Millisecondes depuis l'époque Unix.
    pub added_at: i64,
    pub source_url: Option<String>,
    pub play_count: i64,
    pub last_played: Option<i64>,
    /// LUFS intégrés (EBU R128).
    pub loudness: Option<f64>,
    /// dBTP.
    pub true_peak: Option<f64>,
    #[serde(skip)]
    pub mtime: i64,
}

impl Track {
    /// Lit une ligne sélectionnée avec [`crate::db::TRACK_COLUMNS`].
    pub fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
        Ok(Track {
            id: r.get(0)?,
            path: r.get(1)?,
            kind: Kind::parse(&r.get::<_, String>(2)?),
            title: r.get(3)?,
            artist: r.get(4)?,
            album: r.get(5)?,
            album_artist: r.get(6)?,
            genre: r.get(7)?,
            year: r.get(8)?,
            track_no: r.get(9)?,
            disc_no: r.get(10)?,
            duration: r.get(11)?,
            bitrate: r.get(12)?,
            codec: r.get(13)?,
            size: r.get(14)?,
            cover: r.get(15)?,
            width: r.get(16)?,
            height: r.get(17)?,
            added_at: r.get(18)?,
            source_url: r.get(19)?,
            play_count: r.get(20)?,
            last_played: r.get(21)?,
            loudness: r.get(22)?,
            true_peak: r.get(23)?,
            mtime: r.get(24)?,
        })
    }

    pub fn is_video(&self) -> bool {
        self.kind == Kind::Video
    }

    /// Nom du fichier, sans dossier.
    pub fn file_name(&self) -> &str {
        self.path.rsplit(['/', '\\']).next().unwrap_or(&self.path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PlaylistKind {
    #[default]
    Manual,
    Smart,
}

impl PlaylistKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PlaylistKind::Manual => "manual",
            PlaylistKind::Smart => "smart",
        }
    }

    pub fn parse(s: &str) -> PlaylistKind {
        if s == "smart" { PlaylistKind::Smart } else { PlaylistKind::Manual }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: PlaylistId,
    pub name: String,
    pub kind: PlaylistKind,
    /// Règles d'une playlist intelligente (JSON d'origine).
    pub rules: Option<SmartSpec>,
    pub description: String,
    pub cover: Option<String>,
    pub shared: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Une entrée de playlist classique (un même morceau peut y figurer plusieurs fois).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistItem {
    pub id: i64,
    pub playlist_id: PlaylistId,
    pub track_id: TrackId,
    pub position: i64,
}

/// Règles d'une playlist intelligente.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SmartSpec {
    /// `all` ou `any`.
    #[serde(rename = "match")]
    pub match_: String,
    pub rules: Vec<SmartRule>,
    pub sort: SmartSort,
    /// 0 : sans limite.
    pub limit: i64,
}

impl Default for SmartSpec {
    fn default() -> Self {
        SmartSpec { match_: "all".into(), rules: Vec::new(), sort: SmartSort::default(), limit: 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SmartRule {
    pub field: String,
    pub op: String,
    /// Texte, nombre ou nombre de jours selon le champ.
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SmartSort {
    pub key: String,
    pub dir: String,
}

impl Default for SmartSort {
    fn default() -> Self {
        SmartSort { key: "title".into(), dir: "asc".into() }
    }
}

/// Événements que le noyau envoie à l'interface (ou aux clients du serveur web).
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// La bibliothèque a changé : `tracks` ou `playlists`.
    LibraryChanged(&'static str),
    ScanProgress(ScanProgress),
    /// Autres événements, sous forme JSON : (canal, contenu), avec les noms de canaux d'origine.
    Json(&'static str, Value),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    /// `discover`, `read` ou `done`.
    pub phase: &'static str,
    pub found: usize,
    pub done: usize,
    pub total: usize,
    pub added: usize,
    pub removed: usize,
}

/// Fonction qui reçoit les événements (appelée depuis n'importe quel thread).
pub type Emit = std::sync::Arc<dyn Fn(Event) + Send + Sync>;

/// Émetteur qui ignore tout (tests).
pub fn emit_nothing() -> Emit {
    std::sync::Arc::new(|_| {})
}

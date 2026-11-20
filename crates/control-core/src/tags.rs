//! Éditeur de tags : lecture et écriture dans les fichiers (MP3, FLAC, M4A, OGG, Opus, WAV…), pochettes (fichier,
//! adresse, recherche en ligne), renommage et annulation du dernier lot.
//!
//! L'écriture passe par lofty, format par format ([`media`]) : seules les étiquettes touchées changent, les autres
//! (et l'audio) restent tels quels. Les fichiers sont modifiés pendant [`Library::exclusive`], puis relus avec
//! [`Library::refresh_track`] : le morceau garde son identifiant, donc ses écoutes et ses playlists.

mod cover;
mod media;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub use cover::{
    CoverHit, CoverSource, PreparedImage, StagedCover, merge_hits, parse_deezer, parse_itunes, prepare_image,
};

use crate::library::Library;
use crate::model::{Emit, Event, TrackId};

/// Au-delà, le lot annulable garde les tags mais pas les anciennes pochettes.
const UNDO_BYTES: usize = 40 * 1024 * 1024;
/// Pochettes préparées gardées en mémoire (les plus anciennes partent d'abord).
const STAGED_MAX: usize = 24;

/// Conteneurs que l'on sait modifier (Matroska et WebM ne le sont pas).
const WRITABLE_EXT: &[&str] = &["mp3", "flac", "m4a", "m4b", "mp4", "m4v", "aac", "ogg", "oga", "opus", "wav"];

/// Le fichier est-il d'un format dont on sait écrire les tags ?
pub fn is_writable(path: &Path) -> bool {
    WRITABLE_EXT.contains(&crate::probe::ext_of(path).as_str())
}

/// Champs de l'éditeur. Les nombres valent 0 quand ils sont absents ; les valeurs multiples (plusieurs artistes,
/// plusieurs genres) sont jointes par « ; ».
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TagFields {
    pub title: String,
    pub artist: String,
    pub album_artist: String,
    pub album: String,
    pub genre: String,
    pub composer: String,
    pub comment: String,
    pub lyrics: String,
    pub year: u32,
    pub track_no: u32,
    pub track_total: u32,
    pub disc_no: u32,
    pub disc_total: u32,
}

/// Modifications d'un morceau : seuls les champs présents sont écrits. C'est ce qui donne « Plusieurs valeurs » :
/// un champ qui diffère d'un morceau à l'autre n'est envoyé que si on l'a modifié.
///
/// Texte vide : le champ est effacé. Nombre à 0 : effacé aussi. En JSON, un nombre peut venir comme dans la version
/// d'origine : nombre, chaîne (`"5"`, `""` pour effacer) ou `null` (effacer) ; une valeur invalide est ignorée.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TagSet {
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient_text")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient_text")]
    pub artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient_text")]
    pub album_artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient_text")]
    pub album: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient_text")]
    pub genre: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient_text")]
    pub composer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient_text")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient_text")]
    pub lyrics: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient_number")]
    pub year: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient_number")]
    pub track_no: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient_number")]
    pub track_total: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient_number")]
    pub disc_no: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient_number")]
    pub disc_total: Option<u32>,
}

/// Seules les chaînes comptent (comme `typeof v === 'string'`).
fn lenient_text<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => Some(s),
        _ => None,
    })
}

/// `null` et `""` effacent (0) ; un nombre ou une chaîne numérique est tronqué ; le reste est ignoré.
fn lenient_number<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    let n = match Value::deserialize(d)? {
        Value::Null => return Ok(Some(0)),
        Value::Number(n) => n.as_f64(),
        Value::String(s) if s.trim().is_empty() => return Ok(Some(0)),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    Ok(n.map(f64::trunc).filter(|n| n.is_finite() && *n >= 0.0).map(|n| n.min(u32::MAX as f64) as u32))
}

impl TagSet {
    /// Au moins un champ à écrire ?
    pub fn is_empty(&self) -> bool {
        *self == TagSet::default()
    }

    /// Les valeurs de `fields` pour les champs présents dans `self` (de quoi revenir en arrière).
    fn restoring(&self, fields: &TagFields) -> TagSet {
        let text = |edited: &Option<String>, old: &str| edited.as_ref().map(|_| old.to_string());
        let number = |edited: Option<u32>, old: u32| edited.map(|_| old);
        TagSet {
            title: text(&self.title, &fields.title),
            artist: text(&self.artist, &fields.artist),
            album_artist: text(&self.album_artist, &fields.album_artist),
            album: text(&self.album, &fields.album),
            genre: text(&self.genre, &fields.genre),
            composer: text(&self.composer, &fields.composer),
            comment: text(&self.comment, &fields.comment),
            lyrics: text(&self.lyrics, &fields.lyrics),
            year: number(self.year, fields.year),
            track_no: number(self.track_no, fields.track_no),
            track_total: number(self.track_total, fields.track_total),
            disc_no: number(self.disc_no, fields.disc_no),
            disc_total: number(self.disc_total, fields.disc_total),
        }
    }

    /// Les valeurs finales (texte sans espaces aux bords, nombres bornés) par-dessus les champs actuels.
    fn merged(&self, current: &TagFields) -> TagFields {
        let text = |v: &Option<String>, cur: &str| v.as_ref().map_or_else(|| cur.to_string(), |s| s.trim().to_string());
        let number = |v: Option<u32>, cur: u32, max: u32| v.map_or(cur, |n| n.min(max));
        TagFields {
            title: text(&self.title, &current.title),
            artist: text(&self.artist, &current.artist),
            album_artist: text(&self.album_artist, &current.album_artist),
            album: text(&self.album, &current.album),
            genre: text(&self.genre, &current.genre),
            composer: text(&self.composer, &current.composer),
            comment: text(&self.comment, &current.comment),
            lyrics: text(&self.lyrics, &current.lyrics),
            year: number(self.year, current.year, 9999),
            track_no: number(self.track_no, current.track_no, 9999),
            track_total: number(self.track_total, current.track_total, 9999),
            disc_no: number(self.disc_no, current.disc_no, 999),
            disc_total: number(self.disc_total, current.disc_total, 999),
        }
    }
}

/// Changement de pochette : `{ "token": "…" }` (pochette préparée avec [`Tags::stage_cover`]) ou
/// `{ "remove": true }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CoverEdit {
    Token { token: String },
    Remove { remove: bool },
}

/// Une modification de morceau.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TagEdit {
    pub id: TrackId,
    pub set: TagSet,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover: Option<CoverEdit>,
    /// Nouveau nom du fichier, sans extension.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rename: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorCode {
    Unsupported,
    Missing,
    Locked,
    Readonly,
    Other,
}

/// Échec sur un morceau, avec un message pour l'utilisateur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct TagError {
    pub code: ErrorCode,
    pub message: String,
}

impl TagError {
    fn new(code: ErrorCode, message: &str) -> TagError {
        TagError { code, message: message.to_string() }
    }

    pub fn unsupported() -> TagError {
        TagError::new(ErrorCode::Unsupported, "Format non modifiable (MKV, WebM…).")
    }

    fn missing() -> TagError {
        TagError::new(ErrorCode::Missing, "Fichier introuvable.")
    }

    fn other(message: impl Into<String>) -> TagError {
        TagError { code: ErrorCode::Other, message: message.into() }
    }

    /// Erreur du système de fichiers, traduite pour l'utilisateur.
    fn io(err: &std::io::Error) -> TagError {
        use std::io::ErrorKind as K;
        // 32 et 33 : ERROR_SHARING_VIOLATION et ERROR_LOCK_VIOLATION de Windows.
        if matches!(err.raw_os_error(), Some(32 | 33) if cfg!(windows)) || err.kind() == K::ResourceBusy {
            return TagError::new(ErrorCode::Locked, "Fichier utilisé par un autre programme.");
        }
        match err.kind() {
            K::NotFound => TagError::missing(),
            K::PermissionDenied | K::ReadOnlyFilesystem => {
                TagError::new(ErrorCode::Readonly, "Fichier en lecture seule ou verrouillé.")
            }
            _ => TagError::other(err.to_string()),
        }
    }

    /// Faut-il réessayer ? Windows verrouille un fichier en cours de lecture pendant un court instant.
    fn transient(&self) -> bool {
        matches!(self.code, ErrorCode::Locked | ErrorCode::Readonly)
    }
}

/// Lecture d'un morceau : `fields` et `embedded` (nombre d'images intégrées), ou `error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadResult {
    pub id: TrackId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<TagFields>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedded: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<TagError>,
}

/// Résultat de l'écriture d'un morceau : `{ id, ok: true, path, renamed }` ou `{ id, ok: false, code, message }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteResult {
    pub id: TrackId,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renamed: Option<bool>,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub error: Option<TagError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteOutcome {
    pub results: Vec<WriteResult>,
    /// Le lot peut être annulé avec [`Tags::undo`].
    pub undoable: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct UndoOutcome {
    pub restored: usize,
    pub failed: usize,
}

/// Une image intégrée, gardée pour l'annulation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SavedPicture {
    pub data: Vec<u8>,
    pub mime: Option<String>,
    pub pic_type: u8,
    pub description: Option<String>,
}

/// Ce qu'il faut pour remettre un morceau dans son état d'avant.
#[derive(Debug, Clone)]
struct Snapshot {
    id: TrackId,
    path: String,
    /// Anciennes valeurs des seuls champs modifiés.
    restore: Option<TagSet>,
    /// Anciennes images, si la pochette a changé (et si le lot n'était pas trop lourd).
    pictures: Option<Vec<SavedPicture>>,
    renamed_to: Option<String>,
}

/// Image prête à être intégrée.
#[derive(Debug, Clone)]
struct Staged {
    data: Vec<u8>,
    mime: &'static str,
}

/// Service d'édition des tags. Un par bibliothèque.
pub struct Tags {
    library: Arc<Library>,
    emit: Emit,
    staged: Mutex<VecDeque<(String, Staged)>>,
    last_batch: Mutex<Option<Vec<Snapshot>>>,
}

/// Réessaie un peu une opération sur un fichier verrouillé avant d'abandonner.
fn retrying<T>(mut f: impl FnMut() -> Result<T, TagError>) -> Result<T, TagError> {
    let pause = if cfg!(test) { 5 } else { 250 };
    let mut attempt = 0;
    loop {
        match f() {
            Err(err) if err.transient() && attempt < 3 => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(pause * attempt));
            }
            other => return other,
        }
    }
}

/// Nom de fichier valable sous Windows, macOS et Linux (chaîne vide s'il n'en reste rien).
pub fn safe_name(name: &str) -> String {
    let replaced: String =
        name.chars().map(|c| if (c as u32) < 0x20 || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let spaced = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let stripped = spaced.trim_matches(|c: char| c.is_whitespace() || c == '.');
    let cleaned = crate::util::clip(stripped, 180).trim().to_string();
    let lower = cleaned.to_ascii_lowercase();
    let reserved = ["con", "prn", "aux", "nul"].contains(&lower.as_str())
        || ((lower.starts_with("com") || lower.starts_with("lpt"))
            && lower.len() == 4
            && lower.as_bytes()[3].is_ascii_digit());
    if reserved { String::new() } else { cleaned }
}

/// Renomme un fichier (même dossier, même extension). Ajoute « (2) », « (3) »… si le nom est pris, sauf pour un
/// simple changement de casse.
fn rename_file(file: &Path, base: &str) -> Result<PathBuf, TagError> {
    let dir = file.parent().unwrap_or(Path::new(""));
    let ext = file.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let name = safe_name(base);
    if name.is_empty() {
        return Err(TagError::other("Nom de fichier invalide."));
    }
    let mut target = dir.join(format!("{name}{ext}"));
    if target == file {
        return Ok(target);
    }
    let same = target.to_string_lossy().to_lowercase() == file.to_string_lossy().to_lowercase();
    if !same {
        let mut n = 2;
        while target.exists() {
            target = dir.join(format!("{name} ({n}){ext}"));
            n += 1;
        }
    }
    retrying(|| std::fs::rename(file, &target).map_err(|e| TagError::io(&e)))?;
    Ok(target)
}

/// Paroles intégrées au fichier (étiquette USLT, LYRICS…), ou chaîne vide.
pub fn embedded_lyrics(path: &Path) -> String {
    if !is_writable(path) {
        return String::new();
    }
    media::Media::open(path, false).map(|(_, m)| m.fields().lyrics).unwrap_or_default()
}

impl Tags {
    pub fn new(library: Arc<Library>, emit: Emit) -> Tags {
        Tags { library, emit, staged: Mutex::new(VecDeque::new()), last_batch: Mutex::new(None) }
    }

    // ─── Lecture ─────────────────────────────────────────────────────────────

    pub fn read(&self, ids: &[TrackId]) -> Vec<ReadResult> {
        ids.iter()
            .map(|&id| {
                let mut out = ReadResult { id, path: None, fields: None, embedded: None, error: None };
                let Some(path) = self.library.track_path(id) else {
                    out.error = Some(TagError::new(ErrorCode::Missing, "Morceau introuvable."));
                    return out;
                };
                out.path = Some(path.clone());
                if !is_writable(Path::new(&path)) {
                    out.error = Some(TagError::unsupported());
                    return out;
                }
                match media::Media::open(Path::new(&path), false) {
                    Ok((_, m)) => {
                        out.fields = Some(m.fields());
                        out.embedded = Some(m.pictures().len());
                    }
                    Err(err) => out.error = Some(err),
                }
                out
            })
            .collect()
    }

    // ─── Pochettes ───────────────────────────────────────────────────────────

    /// Prépare une pochette (fichier, adresse ou image envoyée) : réduite, aperçu prêt à afficher. Le jeton rendu
    /// sert ensuite dans [`CoverEdit::Token`].
    pub fn stage_cover(&self, src: &CoverSource) -> anyhow::Result<StagedCover> {
        let bytes = cover::load(src)?;
        let image = prepare_image(&bytes)?;
        let token = hex::encode(rand::random::<[u8; 8]>());
        let mut staged = self.staged.lock();
        staged.push_back((token.clone(), Staged { data: image.data, mime: image.mime }));
        while staged.len() > STAGED_MAX {
            staged.pop_front();
        }
        Ok(StagedCover { token, preview: image.preview, width: image.width, height: image.height })
    }

    /// Pochettes trouvées sur Apple Music (iTunes) et Deezer, en alternance. Erreur seulement si les deux services
    /// sont injoignables.
    pub fn search_covers(&self, artist: &str, album: &str) -> anyhow::Result<Vec<CoverHit>> {
        cover::search(artist, album)
    }

    fn staged(&self, token: &str) -> Option<Staged> {
        self.staged.lock().iter().find(|(t, _)| t == token).map(|(_, s)| s.clone())
    }

    // ─── Écriture ────────────────────────────────────────────────────────────

    /// Écrit un lot de modifications. Un résultat par morceau ; une erreur n'interrompt pas le lot.
    pub fn write(&self, edits: &[TagEdit]) -> WriteOutcome {
        let outcome = self.library.exclusive(|| {
            let mut results = Vec::with_capacity(edits.len());
            let mut snapshots = Vec::new();
            for edit in edits {
                match self.write_one(edit) {
                    Ok((result, snapshot)) => {
                        results.push(result);
                        snapshots.push(snapshot);
                    }
                    Err(err) => results.push(WriteResult {
                        id: edit.id,
                        ok: false,
                        path: None,
                        renamed: None,
                        error: Some(err),
                    }),
                }
            }
            if !snapshots.is_empty() {
                let bytes: usize =
                    snapshots.iter().flat_map(|s| s.pictures.iter().flatten()).map(|p| p.data.len()).sum();
                if bytes > UNDO_BYTES {
                    for s in &mut snapshots {
                        s.pictures = None;
                    }
                }
            }
            let undoable = !snapshots.is_empty();
            if undoable {
                *self.last_batch.lock() = Some(snapshots);
            }
            WriteOutcome { results, undoable }
        });
        if outcome.undoable {
            (self.emit)(Event::LibraryChanged("tracks"));
        }
        outcome
    }

    fn write_one(&self, edit: &TagEdit) -> Result<(WriteResult, Snapshot), TagError> {
        let path = self.library.track_path(edit.id).ok_or_else(TagError::missing)?;
        let file = PathBuf::from(&path);
        if !is_writable(&file) {
            return Err(TagError::unsupported());
        }
        let mut snapshot =
            Snapshot { id: edit.id, path: path.clone(), restore: None, pictures: None, renamed_to: None };
        let wants_tags = !edit.set.is_empty();
        if wants_tags || edit.cover.is_some() {
            let cover = match &edit.cover {
                Some(CoverEdit::Token { token }) => Some(Some(
                    self.staged(token)
                        .ok_or_else(|| TagError::other("Pochette expirée : choisissez-la de nouveau."))?,
                )),
                Some(CoverEdit::Remove { .. }) => Some(None),
                None => None,
            };
            retrying(|| {
                let (mut handle, mut m) = media::Media::open(&file, true)?;
                let before = m.fields();
                if edit.cover.is_some() {
                    snapshot.pictures = Some(m.pictures());
                }
                if wants_tags {
                    snapshot.restore = Some(edit.set.restoring(&before));
                    m.apply(&edit.set.merged(&before), &edit.set);
                }
                if let Some(cover) = &cover {
                    let pictures = cover
                        .iter()
                        .map(|s| SavedPicture {
                            data: s.data.clone(),
                            mime: Some(s.mime.to_string()),
                            pic_type: 3,
                            description: None,
                        })
                        .collect();
                    m.set_pictures(pictures);
                }
                m.save(&mut handle)
            })?;
        }
        let mut new_path = None;
        if let Some(base) = edit.rename.as_deref().filter(|b| !b.is_empty()) {
            let target = rename_file(&file, base)?;
            if target != file {
                new_path = Some(target.to_string_lossy().into_owned());
            }
        }
        let row = self.library.refresh_track(edit.id, new_path.as_deref());
        snapshot.renamed_to.clone_from(&new_path);
        let result = WriteResult {
            id: edit.id,
            ok: true,
            path: Some(row.map(|t| t.path).or_else(|| new_path.clone()).unwrap_or(path)),
            renamed: Some(new_path.is_some()),
            error: None,
        };
        Ok((result, snapshot))
    }

    /// Le dernier lot peut-il être annulé ?
    pub fn can_undo(&self) -> bool {
        self.last_batch.lock().as_ref().is_some_and(|b| !b.is_empty())
    }

    /// Remet les fichiers du dernier lot dans leur état d'avant (tags, pochette, nom).
    pub fn undo(&self) -> UndoOutcome {
        let Some(batch) = self.last_batch.lock().take().filter(|b| !b.is_empty()) else {
            return UndoOutcome::default();
        };
        let outcome = self.library.exclusive(|| {
            let mut out = UndoOutcome::default();
            for s in batch.iter().rev() {
                match self.undo_one(s) {
                    Ok(()) => out.restored += 1,
                    Err(err) => {
                        log::warn!("Annulation impossible pour {} : {err}", s.path);
                        out.failed += 1;
                    }
                }
            }
            out
        });
        (self.emit)(Event::LibraryChanged("tracks"));
        outcome
    }

    fn undo_one(&self, s: &Snapshot) -> Result<(), TagError> {
        let file = PathBuf::from(&s.path);
        if let Some(renamed) = &s.renamed_to {
            retrying(|| std::fs::rename(renamed, &file).map_err(|e| TagError::io(&e)))?;
        }
        if s.restore.is_some() || s.pictures.is_some() {
            retrying(|| {
                let (mut handle, mut m) = media::Media::open(&file, true)?;
                if let Some(restore) = &s.restore {
                    let current = m.fields();
                    m.apply(&restore.merged(&current), restore);
                }
                if let Some(pictures) = &s.pictures {
                    m.set_pictures(pictures.clone());
                }
                m.save(&mut handle)
            })?;
        }
        // Comme l'original : le fichier est restauré même si le morceau a quitté la bibliothèque entre-temps.
        let moved = self.library.track_path(s.id).is_some_and(|p| p != s.path);
        self.library.refresh_track(s.id, moved.then_some(s.path.as_str()));
        Ok(())
    }
}

#[cfg(test)]
mod tests;

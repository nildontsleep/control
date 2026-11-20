//! Accès aux tags d'un fichier, format par format. On garde la structure propre à chaque format (ID3v2,
//! commentaires Vorbis, atomes MP4) au lieu du `Tag` générique de lofty : la conversion générique perd des cadres,
//! alors qu'ici seuls les champs de l'éditeur changent et tout le reste est réécrit tel quel.

use std::borrow::Cow;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, Seek};
use std::path::Path;

use lofty::TextEncoding;
use lofty::aac::AacFile;
use lofty::config::{ParseOptions, WriteOptions};
use lofty::error::{ErrorKind, LoftyError};
use lofty::file::{AudioFile, FileType};
use lofty::flac::FlacFile;
use lofty::id3::v1::Id3v1Tag;
use lofty::id3::v2::{
    AttachedPictureFrame, CommentFrame, Frame, FrameId, Id3v2Tag, Id3v2Version, TextInformationFrame, TimestampFrame,
    UnsynchronizedTextFrame,
};
use lofty::iff::wav::{RiffInfoList, WavFile};
use lofty::mp4::{Atom, AtomData, AtomIdent, Ilst, Mp4File};
use lofty::mpeg::MpegFile;
use lofty::ogg::{OggPictureStorage, OpusFile, SpeexFile, VorbisComments, VorbisFile};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::Accessor;
use lofty::tag::items::{ENGLISH, Timestamp};

use super::{SavedPicture, TagError, TagFields, TagSet};

/// Le tag ID3v2 d'un fichier, créé s'il n'existe pas.
macro_rules! id3_of {
    ($f:expr) => {{
        if $f.id3v2().is_none() {
            $f.set_id3v2(Id3v2Tag::new());
        }
        $f.id3v2_mut().expect("tag créé juste avant")
    }};
}

/// Fichier ouvert, avec ses tags.
pub(crate) struct Media {
    file: Kind,
    /// Le fichier avait déjà un tag ID3v2 (sinon on en crée un en version 2.3).
    id3_existed: bool,
}

enum Kind {
    Mpeg(MpegFile),
    Aac(AacFile),
    Wav(WavFile),
    Flac(FlacFile),
    Vorbis(VorbisFile),
    Opus(OpusFile),
    Speex(SpeexFile),
    Mp4(Mp4File),
}

fn lofty_error(err: LoftyError) -> TagError {
    match err.kind() {
        ErrorKind::Io(e) => TagError::io(e),
        ErrorKind::UnknownFormat | ErrorKind::UnsupportedTag => {
            TagError::new(super::ErrorCode::Unsupported, "Format non pris en charge.")
        }
        _ => TagError::other(err.to_string()),
    }
}

fn read<T: AudioFile>(file: &mut File) -> Result<T, TagError> {
    file.rewind().map_err(|e| TagError::io(&e))?;
    // Les propriétés audio ne servent pas ici : ne pas les lire rend la lecture plus rapide et plus tolérante.
    let options = ParseOptions::new().read_properties(false);
    T::read_from(&mut BufReader::new(file), options).map_err(lofty_error)
}

impl Media {
    /// Ouvre un fichier (en écriture si `write`), et reconnaît son format d'après son contenu.
    pub fn open(path: &Path, write: bool) -> Result<(File, Media), TagError> {
        let mut file = OpenOptions::new().read(true).write(write).open(path).map_err(|e| TagError::io(&e))?;
        let ty = lofty::probe::Probe::new(BufReader::new(&mut file))
            .guess_file_type()
            .map_err(|e| TagError::io(&e))?
            .file_type();
        let kind = match ty {
            Some(FileType::Mpeg) => Kind::Mpeg(read(&mut file)?),
            Some(FileType::Aac) => Kind::Aac(read(&mut file)?),
            Some(FileType::Wav) => Kind::Wav(read(&mut file)?),
            Some(FileType::Flac) => Kind::Flac(read(&mut file)?),
            Some(FileType::Vorbis) => Kind::Vorbis(read(&mut file)?),
            Some(FileType::Opus) => Kind::Opus(read(&mut file)?),
            Some(FileType::Speex) => Kind::Speex(read(&mut file)?),
            Some(FileType::Mp4) => Kind::Mp4(read(&mut file)?),
            _ => return Err(TagError::new(super::ErrorCode::Unsupported, "Format non pris en charge.")),
        };
        let id3_existed = match &kind {
            Kind::Mpeg(f) => f.id3v2().is_some(),
            Kind::Aac(f) => f.id3v2().is_some(),
            Kind::Wav(f) => f.id3v2().is_some(),
            _ => false,
        };
        Ok((file, Media { file: kind, id3_existed }))
    }

    pub fn fields(&self) -> TagFields {
        match &self.file {
            Kind::Mpeg(f) => id3_or_v1(f.id3v2(), f.id3v1()),
            Kind::Aac(f) => id3_or_v1(f.id3v2(), f.id3v1()),
            Kind::Wav(f) => match (f.id3v2(), f.riff_info()) {
                (Some(t), _) => id3::fields(t),
                (None, Some(info)) => basic_fields(info),
                (None, None) => TagFields::default(),
            },
            Kind::Flac(f) => f.vorbis_comments().map(vorbis::fields).unwrap_or_default(),
            Kind::Vorbis(f) => vorbis::fields(f.vorbis_comments()),
            Kind::Opus(f) => vorbis::fields(f.vorbis_comments()),
            Kind::Speex(f) => vorbis::fields(f.vorbis_comments()),
            Kind::Mp4(f) => f.ilst().map(mp4::fields).unwrap_or_default(),
        }
    }

    pub fn pictures(&self) -> Vec<SavedPicture> {
        let ogg = |pics: &[(Picture, lofty::picture::PictureInformation)]| pics.iter().map(|(p, _)| saved(p)).collect();
        match &self.file {
            Kind::Mpeg(f) => f.id3v2().map(id3::pictures).unwrap_or_default(),
            Kind::Aac(f) => f.id3v2().map(id3::pictures).unwrap_or_default(),
            Kind::Wav(f) => f.id3v2().map(id3::pictures).unwrap_or_default(),
            Kind::Flac(f) => ogg(OggPictureStorage::pictures(f)),
            Kind::Vorbis(f) => ogg(OggPictureStorage::pictures(f.vorbis_comments())),
            Kind::Opus(f) => ogg(OggPictureStorage::pictures(f.vorbis_comments())),
            Kind::Speex(f) => ogg(OggPictureStorage::pictures(f.vorbis_comments())),
            Kind::Mp4(f) => f.ilst().and_then(|t| t.pictures()).map(|p| p.map(saved).collect()).unwrap_or_default(),
        }
    }

    /// Écrit les champs présents dans `set`, avec les valeurs de `target` (déjà nettoyées et bornées).
    pub fn apply(&mut self, target: &TagFields, set: &TagSet) {
        match &mut self.file {
            Kind::Mpeg(f) => {
                id3::apply(id3_of!(f), target, set);
                if let Some(v1) = f.id3v1_mut() {
                    apply_basic(v1, target, set);
                }
            }
            Kind::Aac(f) => {
                id3::apply(id3_of!(f), target, set);
                if let Some(v1) = f.id3v1_mut() {
                    apply_basic(v1, target, set);
                }
            }
            Kind::Wav(f) => {
                id3::apply(id3_of!(f), target, set);
                // La liste RIFF INFO, si elle existe, suit (d'autres lecteurs ne lisent qu'elle).
                if let Some(info) = f.riff_info_mut() {
                    apply_basic::<RiffInfoList>(info, target, set);
                }
            }
            Kind::Flac(f) => vorbis::apply(flac_comments(f), target, set),
            Kind::Vorbis(f) => vorbis::apply(f.vorbis_comments_mut(), target, set),
            Kind::Opus(f) => vorbis::apply(f.vorbis_comments_mut(), target, set),
            Kind::Speex(f) => vorbis::apply(f.vorbis_comments_mut(), target, set),
            Kind::Mp4(f) => {
                if f.ilst().is_none() {
                    f.set_ilst(Ilst::new());
                }
                if let Some(ilst) = f.ilst_mut() {
                    mp4::apply(ilst, target, set);
                }
            }
        }
    }

    /// Remplace toutes les images intégrées.
    pub fn set_pictures(&mut self, pictures: Vec<SavedPicture>) {
        let pictures: Vec<Picture> = pictures.into_iter().map(picture).collect();
        let ogg = |store: &mut dyn FnMut(Picture)| {
            for p in &pictures {
                store(p.clone());
            }
        };
        match &mut self.file {
            Kind::Mpeg(f) => id3::set_pictures(id3_of!(f), &pictures),
            Kind::Aac(f) => id3::set_pictures(id3_of!(f), &pictures),
            Kind::Wav(f) => id3::set_pictures(id3_of!(f), &pictures),
            Kind::Flac(f) => {
                // Sans bloc de commentaires, lofty n'écrirait rien et les anciennes images resteraient.
                flac_comments(f);
                f.remove_pictures();
                ogg(&mut |p| insert_ogg_picture(f, p));
            }
            Kind::Vorbis(f) => {
                let vc = f.vorbis_comments_mut();
                vc.remove_pictures();
                ogg(&mut |p| insert_ogg_picture(vc, p));
            }
            Kind::Opus(f) => {
                let vc = f.vorbis_comments_mut();
                vc.remove_pictures();
                ogg(&mut |p| insert_ogg_picture(vc, p));
            }
            Kind::Speex(f) => {
                let vc = f.vorbis_comments_mut();
                vc.remove_pictures();
                ogg(&mut |p| insert_ogg_picture(vc, p));
            }
            Kind::Mp4(f) => {
                if f.ilst().is_none() {
                    f.set_ilst(Ilst::new());
                }
                if let Some(ilst) = f.ilst_mut() {
                    ilst.remove_pictures();
                    for p in pictures {
                        ilst.insert_picture(p);
                    }
                }
            }
        }
    }

    /// Réécrit les tags dans le fichier (l'audio n'est pas touché).
    pub fn save(&self, file: &mut File) -> Result<(), TagError> {
        file.rewind().map_err(|e| TagError::io(&e))?;
        // Les anciens lecteurs (Explorateur Windows, autoradios) lisent mieux l'ID3v2.3 que la 2.4 : un nouveau tag
        // est écrit en 2.3, un tag existant garde sa version.
        let v4 = self.id3_existed
            && match &self.file {
                Kind::Mpeg(f) => f.id3v2().is_some_and(|t| t.original_version() == Id3v2Version::V4),
                Kind::Aac(f) => f.id3v2().is_some_and(|t| t.original_version() == Id3v2Version::V4),
                Kind::Wav(f) => f.id3v2().is_some_and(|t| t.original_version() == Id3v2Version::V4),
                _ => false,
            };
        let options = WriteOptions::new().use_id3v23(!v4);
        if let Kind::Flac(_) = self.file {
            ensure_flac_padding(file).map_err(|e| TagError::io(&e))?;
            file.rewind().map_err(|e| TagError::io(&e))?;
        }
        let res = match &self.file {
            Kind::Mpeg(f) => f.save_to(file, options),
            Kind::Aac(f) => f.save_to(file, options),
            Kind::Wav(f) => f.save_to(file, options),
            Kind::Flac(f) => f.save_to(file, options),
            Kind::Vorbis(f) => f.save_to(file, options),
            Kind::Opus(f) => f.save_to(file, options),
            Kind::Speex(f) => f.save_to(file, options),
            Kind::Mp4(f) => f.save_to(file, options),
        };
        res.map_err(lofty_error)?;
        file.sync_all().map_err(|e| TagError::io(&e))
    }
}

/// Ajoute un bloc de bourrage en dernier bloc de métadonnées d'un FLAC qui n'en a pas.
///
/// lofty 0.22 se trompe d'octet quand il doit lui-même ajouter ce bloc (il écrit à un indice égal à la longueur du
/// dernier bloc) : selon le fichier, il panique ou abîme un octet. Avec un bourrage déjà en place, il ne fait que le
/// redimensionner. Seules les métadonnées bougent ; l'audio est recopié tel quel.
fn ensure_flac_padding(file: &mut File) -> std::io::Result<()> {
    use std::io::{Read, Write};
    const PADDING: u8 = 1;
    let mut bytes = Vec::new();
    file.rewind()?;
    file.read_to_end(&mut bytes)?;
    // Un tag ID3v2 peut précéder le FLAC (déconseillé, mais ça existe).
    let mut at = 0usize;
    if bytes.len() >= 10 && &bytes[..3] == b"ID3" {
        let size = bytes[6..10].iter().fold(0usize, |acc, b| (acc << 7) | (*b as usize & 0x7F));
        at = 10 + size + if bytes[5] & 0x10 != 0 { 10 } else { 0 };
    }
    if bytes.get(at..at + 4) != Some(b"fLaC") {
        return Ok(());
    }
    at += 4;
    loop {
        let Some(head) = bytes.get(at..at + 4) else { return Ok(()) };
        let last = head[0] & 0x80 != 0;
        let ty = head[0] & 0x7F;
        let len = ((head[1] as usize) << 16) | ((head[2] as usize) << 8) | head[3] as usize;
        if !last {
            at += 4 + len;
            continue;
        }
        if ty == PADDING {
            return Ok(());
        }
        let end = at + 4 + len;
        if end > bytes.len() {
            return Ok(());
        }
        bytes[at] &= 0x7F;
        let size = WriteOptions::DEFAULT_PREFERRED_PADDING as usize;
        let mut block = vec![0x80 | PADDING, (size >> 16) as u8, (size >> 8) as u8, size as u8];
        block.resize(4 + size, 0);
        bytes.splice(end..end, block);
        file.rewind()?;
        file.write_all(&bytes)?;
        file.set_len(bytes.len() as u64)?;
        return Ok(());
    }
}

/// Les commentaires Vorbis d'un FLAC, créés s'il n'y en a pas.
fn flac_comments(f: &mut FlacFile) -> &mut VorbisComments {
    if f.vorbis_comments().is_none() {
        f.set_vorbis_comments(VorbisComments::default());
    }
    f.vorbis_comments_mut().expect("commentaires créés juste avant")
}

fn insert_ogg_picture(store: &mut impl OggPictureStorage, p: Picture) {
    // Les informations (taille, profondeur) sont lues dans l'image ; à défaut, des zéros conviennent.
    let info = lofty::picture::PictureInformation::from_picture(&p).unwrap_or_default();
    if let Err(err) = store.insert_picture(p, Some(info)) {
        log::warn!("Image non intégrée : {err}");
    }
}

fn saved(p: &Picture) -> SavedPicture {
    SavedPicture {
        data: p.data().to_vec(),
        mime: p.mime_type().map(|m| m.as_str().to_string()),
        pic_type: p.pic_type().as_u8(),
        description: p.description().map(str::to_string),
    }
}

fn picture(p: SavedPicture) -> Picture {
    Picture::new_unchecked(
        PictureType::from_u8(p.pic_type),
        p.mime.as_deref().map(MimeType::from_str),
        p.description,
        p.data,
    )
}

fn id3_or_v1(v2: Option<&Id3v2Tag>, v1: Option<&Id3v1Tag>) -> TagFields {
    match (v2, v1) {
        (Some(t), _) => id3::fields(t),
        (None, Some(t)) => basic_fields(t),
        (None, None) => TagFields::default(),
    }
}

fn trimmed(s: Option<Cow<'_, str>>) -> String {
    s.map(|v| v.trim().to_string()).unwrap_or_default()
}

/// Champs communs à tous les formats (ID3v1, RIFF INFO).
fn basic_fields(t: &impl Accessor) -> TagFields {
    TagFields {
        title: trimmed(t.title()),
        artist: trimmed(t.artist()),
        album: trimmed(t.album()),
        genre: trimmed(t.genre()),
        comment: trimmed(t.comment()),
        year: t.year().unwrap_or(0),
        track_no: t.track().unwrap_or(0),
        track_total: t.track_total().unwrap_or(0),
        ..TagFields::default()
    }
}

/// Écrit les champs communs dans un tag secondaire (ID3v1, RIFF INFO) qui existe déjà.
fn apply_basic<T: Accessor>(t: &mut T, target: &TagFields, set: &TagSet) {
    fn text<T: Accessor>(t: &mut T, value: &str, put: fn(&mut T, String), clear: fn(&mut T)) {
        if value.is_empty() { clear(t) } else { put(t, value.to_string()) }
    }
    if set.title.is_some() {
        text(t, &target.title, T::set_title, T::remove_title);
    }
    if set.artist.is_some() {
        text(t, &target.artist, T::set_artist, T::remove_artist);
    }
    if set.album.is_some() {
        text(t, &target.album, T::set_album, T::remove_album);
    }
    if set.genre.is_some() {
        text(t, &target.genre, T::set_genre, T::remove_genre);
    }
    if set.comment.is_some() {
        text(t, &target.comment, T::set_comment, T::remove_comment);
    }
    if set.year.is_some() {
        if target.year > 0 { t.set_year(target.year) } else { t.remove_year() }
    }
    if set.track_no.is_some() {
        if target.track_no > 0 { t.set_track(target.track_no) } else { t.remove_track() }
    }
}

/// « 3/12 » → (3, 12) ; « 3 » → (3, 0).
fn number_pair(s: &str) -> (u32, u32) {
    let mut parts = s.split(['/', '\0']).map(|p| {
        let digits: String = p.trim().chars().take_while(char::is_ascii_digit).collect();
        digits.parse::<u32>().unwrap_or(0)
    });
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

fn join<'a>(values: impl Iterator<Item = &'a str>) -> String {
    values.map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("; ")
}

mod id3 {
    use super::*;

    fn id(name: &'static str) -> FrameId<'static> {
        FrameId::Valid(Cow::Borrowed(name))
    }

    fn texts(t: &Id3v2Tag, name: &'static str) -> String {
        t.get_texts(&id(name)).map(join).unwrap_or_default()
    }

    fn set_text(t: &mut Id3v2Tag, name: &'static str, value: &str) {
        let _ = t.remove(&id(name));
        if !value.is_empty() {
            t.insert(Frame::Text(TextInformationFrame::new(id(name), TextEncoding::UTF16, value.to_string())));
        }
    }

    fn pair(t: &Id3v2Tag, name: &'static str) -> (u32, u32) {
        t.get_text(&id(name)).map(number_pair).unwrap_or((0, 0))
    }

    /// Même écriture que TagLib : « n/total », « n », ou rien si les deux valent 0.
    fn set_pair(t: &mut Id3v2Tag, name: &'static str, n: u32, total: u32) {
        let value = match (n, total) {
            (0, 0) => String::new(),
            (n, 0) => n.to_string(),
            (n, total) => format!("{n}/{total}"),
        };
        set_text(t, name, &value);
    }

    pub fn fields(t: &Id3v2Tag) -> TagFields {
        let (track_no, track_total) = pair(t, "TRCK");
        let (disc_no, disc_total) = pair(t, "TPOS");
        let year = t
            .year()
            .or_else(|| t.get_text(&id("TDRC")).and_then(|s| s.get(..4)).and_then(|s| s.parse().ok()))
            .unwrap_or(0);
        TagFields {
            title: texts(t, "TIT2"),
            artist: texts(t, "TPE1"),
            album_artist: texts(t, "TPE2"),
            album: texts(t, "TALB"),
            genre: t.genres().map(join).unwrap_or_default(),
            composer: texts(t, "TCOM"),
            comment: trimmed(t.comment()),
            lyrics: t.unsync_text().next().map(|f| f.content.trim().to_string()).unwrap_or_default(),
            year,
            track_no,
            track_total,
            disc_no,
            disc_total,
        }
    }

    pub fn apply(t: &mut Id3v2Tag, target: &TagFields, set: &TagSet) {
        let texts: [(&Option<String>, &'static str, &str); 6] = [
            (&set.title, "TIT2", &target.title),
            (&set.artist, "TPE1", &target.artist),
            (&set.album_artist, "TPE2", &target.album_artist),
            (&set.album, "TALB", &target.album),
            (&set.genre, "TCON", &target.genre),
            (&set.composer, "TCOM", &target.composer),
        ];
        for (edited, name, value) in texts {
            if edited.is_some() {
                set_text(t, name, value);
            }
        }
        if set.comment.is_some() {
            // Le commentaire « principal » (sans description) ; les autres restent.
            let lang = t.comments().next().map_or(ENGLISH, |c| c.language);
            t.retain(|f| !matches!(f, Frame::Comment(c) if c.description.is_empty()));
            if !target.comment.is_empty() {
                let frame = CommentFrame::new(TextEncoding::UTF16, lang, String::new(), target.comment.clone());
                t.insert(Frame::Comment(frame));
            }
        }
        if set.lyrics.is_some() {
            let lang = t.unsync_text().next().map_or(ENGLISH, |f| f.language);
            t.retain(|f| !matches!(f, Frame::UnsynchronizedText(_)));
            if !target.lyrics.is_empty() {
                let frame =
                    UnsynchronizedTextFrame::new(TextEncoding::UTF16, lang, String::new(), target.lyrics.clone());
                t.insert(Frame::UnsynchronizedText(frame));
            }
        }
        if set.year.is_some() {
            let _ = t.remove(&id("TDRC"));
            if target.year > 0 {
                // Un vrai cadre d'horodatage : lofty le convertit en TYER pour l'ID3v2.3.
                let stamp = Timestamp { year: target.year as u16, ..Timestamp::default() };
                t.insert(Frame::Timestamp(TimestampFrame::new(id("TDRC"), TextEncoding::Latin1, stamp)));
            }
        }
        if set.track_no.is_some() || set.track_total.is_some() {
            set_pair(t, "TRCK", target.track_no, target.track_total);
        }
        if set.disc_no.is_some() || set.disc_total.is_some() {
            set_pair(t, "TPOS", target.disc_no, target.disc_total);
        }
    }

    pub fn pictures(t: &Id3v2Tag) -> Vec<SavedPicture> {
        t.into_iter()
            .filter_map(|f| match f {
                Frame::Picture(AttachedPictureFrame { picture, .. }) => Some(saved(picture)),
                _ => None,
            })
            .collect()
    }

    pub fn set_pictures(t: &mut Id3v2Tag, pictures: &[Picture]) {
        t.retain(|f| !matches!(f, Frame::Picture(_)));
        for p in pictures {
            // lofty écrit une description absente sur un seul octet, ce qui casse l'UTF-16 : en Latin-1, c'est juste.
            let latin1 = p.description().is_none_or(|d| d.chars().all(|c| (c as u32) <= 0xFF));
            let encoding = if latin1 { TextEncoding::Latin1 } else { TextEncoding::UTF16 };
            t.insert(Frame::Picture(AttachedPictureFrame::new(encoding, p.clone())));
        }
    }
}

mod vorbis {
    use super::*;

    fn all(vc: &VorbisComments, keys: &[&str]) -> String {
        keys.iter().map(|k| join(vc.get_all(k))).find(|s| !s.is_empty()).unwrap_or_default()
    }

    /// Remplace un champ (et ses variantes de nom) par une seule valeur, ou l'efface.
    fn set(vc: &mut VorbisComments, keys: &[&str], value: &str) {
        for k in keys {
            let _ = vc.remove(k);
        }
        if !value.is_empty() {
            vc.push(keys[0].to_string(), value.to_string());
        }
    }

    /// Numéro et total : `TRACKNUMBER=3` et `TRACKTOTAL=12`, ou l'ancienne forme `TRACKNUMBER=3/12`.
    fn pair(vc: &VorbisComments, number: &[&str], total: &[&str]) -> (u32, u32) {
        let (n, inline_total) = number.iter().find_map(|k| vc.get(k)).map(number_pair).unwrap_or((0, 0));
        let t = total.iter().find_map(|k| vc.get(k)).map(|s| number_pair(s).0).unwrap_or(0);
        (n, if t > 0 { t } else { inline_total })
    }

    const TRACK: &[&str] = &["TRACKNUMBER", "TRACKNUM"];
    const TRACK_TOTAL: &[&str] = &["TRACKTOTAL", "TOTALTRACKS"];
    const DISC: &[&str] = &["DISCNUMBER"];
    const DISC_TOTAL: &[&str] = &["DISCTOTAL", "TOTALDISCS"];

    pub fn fields(vc: &VorbisComments) -> TagFields {
        let (track_no, track_total) = pair(vc, TRACK, TRACK_TOTAL);
        let (disc_no, disc_total) = pair(vc, DISC, DISC_TOTAL);
        TagFields {
            title: all(vc, &["TITLE"]),
            artist: all(vc, &["ARTIST"]),
            album_artist: all(vc, &["ALBUMARTIST", "ALBUM ARTIST"]),
            album: all(vc, &["ALBUM"]),
            genre: all(vc, &["GENRE"]),
            composer: all(vc, &["COMPOSER"]),
            comment: all(vc, &["COMMENT", "DESCRIPTION"]),
            lyrics: all(vc, &["LYRICS", "UNSYNCEDLYRICS"]),
            year: vc.year().unwrap_or(0),
            track_no,
            track_total,
            disc_no,
            disc_total,
        }
    }

    pub fn apply(vc: &mut VorbisComments, target: &TagFields, s: &TagSet) {
        let texts: [(&Option<String>, &[&str], &str); 8] = [
            (&s.title, &["TITLE"], &target.title),
            (&s.artist, &["ARTIST"], &target.artist),
            (&s.album_artist, &["ALBUMARTIST", "ALBUM ARTIST"], &target.album_artist),
            (&s.album, &["ALBUM"], &target.album),
            (&s.genre, &["GENRE"], &target.genre),
            (&s.composer, &["COMPOSER"], &target.composer),
            (&s.comment, &["COMMENT", "DESCRIPTION"], &target.comment),
            (&s.lyrics, &["LYRICS", "UNSYNCEDLYRICS"], &target.lyrics),
        ];
        for (edited, keys, value) in texts {
            if edited.is_some() {
                set(vc, keys, value);
            }
        }
        let number = |n: u32| if n > 0 { n.to_string() } else { String::new() };
        if s.year.is_some() {
            set(vc, &["DATE", "YEAR"], &number(target.year));
        }
        if s.track_no.is_some() || s.track_total.is_some() {
            set(vc, TRACK, &number(target.track_no));
            set(vc, TRACK_TOTAL, &number(target.track_total));
        }
        if s.disc_no.is_some() || s.disc_total.is_some() {
            set(vc, DISC, &number(target.disc_no));
            set(vc, DISC_TOTAL, &number(target.disc_total));
        }
    }
}

mod mp4 {
    use super::*;

    const ALBUM_ARTIST: AtomIdent<'static> = AtomIdent::Fourcc(*b"aART");
    const COMPOSER: AtomIdent<'static> = AtomIdent::Fourcc(*b"\xa9wrt");
    const LYRICS: AtomIdent<'static> = AtomIdent::Fourcc(*b"\xa9lyr");
    const GENRE_ID3: AtomIdent<'static> = AtomIdent::Fourcc(*b"gnre");
    const YEAR: AtomIdent<'static> = AtomIdent::Fourcc(*b"\xa9day");

    fn text(t: &Ilst, ident: &AtomIdent<'_>) -> String {
        t.get(ident)
            .map(|a| {
                join(a.data().filter_map(|d| match d {
                    AtomData::UTF8(s) | AtomData::UTF16(s) => Some(s.as_str()),
                    _ => None,
                }))
            })
            .unwrap_or_default()
    }

    fn set_text(t: &mut Ilst, ident: AtomIdent<'static>, value: &str) {
        let _ = t.remove(&ident);
        if !value.is_empty() {
            t.replace_atom(Atom::new(ident, AtomData::UTF8(value.to_string())));
        }
    }

    pub fn fields(t: &Ilst) -> TagFields {
        TagFields {
            album_artist: text(t, &ALBUM_ARTIST),
            composer: text(t, &COMPOSER),
            lyrics: text(t, &LYRICS),
            disc_no: t.disk().unwrap_or(0),
            disc_total: t.disk_total().unwrap_or(0),
            ..basic_fields(t)
        }
    }

    pub fn apply(t: &mut Ilst, target: &TagFields, s: &TagSet) {
        if s.genre.is_some() {
            // L'ancien genre numérique (`gnre`) laisserait deux valeurs.
            let _ = t.remove(&GENRE_ID3);
        }
        apply_basic(t, target, s);
        if s.year.is_some() && target.year == 0 {
            // `Ilst::remove_year` de lofty 0.22 retire un atome `Year` au lieu de `©day`.
            let _ = t.remove(&YEAR);
        }
        for (edited, ident, value) in [
            (&s.album_artist, ALBUM_ARTIST, &target.album_artist),
            (&s.composer, COMPOSER, &target.composer),
            (&s.lyrics, LYRICS, &target.lyrics),
        ] {
            if edited.is_some() {
                set_text(t, ident, value);
            }
        }
        // `trkn` et `disk` portent le numéro et le total ensemble : on réécrit la paire.
        if s.track_no.is_some() || s.track_total.is_some() {
            t.remove_track();
            if target.track_total > 0 {
                t.set_track_total(target.track_total);
                t.set_track(target.track_no);
            } else if target.track_no > 0 {
                t.set_track(target.track_no);
            }
        }
        if s.disc_no.is_some() || s.disc_total.is_some() {
            t.remove_disk();
            if target.disc_total > 0 {
                t.set_disk_total(target.disc_total);
                t.set_disk(target.disc_no);
            } else if target.disc_no > 0 {
                t.set_disk(target.disc_no);
            }
        }
    }
}

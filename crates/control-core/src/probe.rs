//! Lecture des informations d'un fichier : tags et pochette (lofty), et pour les conteneurs vidéo (MP4/MOV,
//! Matroska/WebM) la présence d'une piste vidéo, sa taille et la durée, lues directement dans le conteneur.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use lofty::file::{AudioFile, FileType, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::prelude::{Accessor, ItemKey};
use lofty::probe::Probe as LoftyProbe;

use crate::model::Kind;

pub const AUDIO_EXT: &[&str] = &["mp3", "flac", "m4a", "m4b", "aac", "ogg", "oga", "opus", "wav"];
pub const VIDEO_EXT: &[&str] = &["mp4", "m4v", "mov", "mkv", "webm"];

/// Extension en minuscules, sans le point.
pub fn ext_of(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default()
}

pub fn is_media_file(path: &Path) -> bool {
    let e = ext_of(path);
    AUDIO_EXT.contains(&e.as_str()) || VIDEO_EXT.contains(&e.as_str())
}

/// Ce que l'on sait d'un fichier après lecture.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Probe {
    pub kind: Kind,
    pub title: Option<String>,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: Option<i64>,
    pub track_no: Option<i64>,
    pub disc_no: Option<i64>,
    pub duration: f64,
    pub bitrate: Option<i64>,
    pub codec: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    /// Image de la pochette intégrée (la face avant de préférence).
    pub picture: Option<Vec<u8>>,
}

/// Lit un fichier. Ne renvoie jamais d'erreur : un fichier illisible garde au moins son genre d'après l'extension.
pub fn probe(path: &Path) -> Probe {
    let ext = ext_of(path);
    let mut out = Probe {
        kind: if VIDEO_EXT.contains(&ext.as_str()) { Kind::Video } else { Kind::Audio },
        ..Probe::default()
    };
    let container = match ext.as_str() {
        "mp4" | "m4v" | "mov" | "m4a" | "m4b" => File::open(path).ok().and_then(|f| mp4::probe(BufReader::new(f))),
        "mkv" | "webm" => File::open(path).ok().and_then(|f| matroska::probe(BufReader::new(f))),
        _ => None,
    };
    if let Some(c) = &container {
        out.kind = if c.video.is_some() { Kind::Video } else { Kind::Audio };
        if let Some((w, h)) = c.video.filter(|(w, h)| *w > 0 && *h > 0) {
            out.width = Some(w as i64);
            out.height = Some(h as i64);
        }
        out.duration = c.duration;
        if c.title.is_some() {
            out.title.clone_from(&c.title);
        }
        out.codec.clone_from(&c.codec);
    }
    read_tags(path, &mut out);
    out
}

fn clean(s: Option<std::borrow::Cow<'_, str>>) -> String {
    s.map(|v| v.trim().to_string()).unwrap_or_default()
}

fn read_tags(path: &Path, out: &mut Probe) {
    let Ok(probe) = LoftyProbe::open(path).and_then(|p| Ok(p.guess_file_type()?)) else { return };
    let Ok(tagged) = probe.read() else { return };
    let props = tagged.properties();
    let d = props.duration().as_secs_f64();
    if d > 0.0 {
        out.duration = d;
    }
    if let Some(b) = props.audio_bitrate().or(props.overall_bitrate()).filter(|b| *b > 0) {
        out.bitrate = Some(b as i64);
    }
    if out.codec.is_none() {
        out.codec = Some(
            match tagged.file_type() {
                FileType::Mpeg => "MPEG 1 Layer 3",
                FileType::Flac => "FLAC",
                FileType::Aac => "AAC",
                FileType::Mp4 => "AAC",
                FileType::Opus => "Opus",
                FileType::Vorbis => "Vorbis I",
                FileType::Wav => "PCM",
                FileType::Aiff => "PCM",
                FileType::Speex => "Speex",
                FileType::Ape => "Monkey's Audio",
                FileType::Mpc => "Musepack",
                FileType::WavPack => "WavPack",
                _ => "",
            }
            .to_string(),
        )
        .filter(|s| !s.is_empty());
    }
    let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) else { return };
    if let Some(t) = tag.title().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
        out.title = Some(t);
    }
    out.artist = clean(tag.artist());
    if out.artist.is_empty() {
        // Plusieurs champs « artiste » : joints par une virgule, comme l'original.
        let all: Vec<String> = tag.get_strings(&ItemKey::TrackArtist).map(|s| s.trim().to_string()).collect();
        out.artist = all.join(", ");
    }
    out.album = clean(tag.album());
    out.album_artist = tag.get_string(&ItemKey::AlbumArtist).map(|s| s.trim().to_string()).unwrap_or_default();
    let genres: Vec<String> =
        tag.get_strings(&ItemKey::Genre).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    out.genre = genres.join(", ");
    out.year = tag.year().map(i64::from).filter(|y| *y > 0);
    out.track_no = tag.track().map(i64::from).filter(|n| *n > 0);
    out.disc_no = tag.disk().map(i64::from).filter(|n| *n > 0);
    let pics = tag.pictures();
    let front = pics.iter().find(|p| p.pic_type() == PictureType::CoverFront).or_else(|| pics.first());
    out.picture = front.map(|p| p.data().to_vec()).filter(|d| !d.is_empty());
}

/// Informations lues dans un conteneur vidéo.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Container {
    pub video: Option<(u32, u32)>,
    pub duration: f64,
    pub title: Option<String>,
    pub codec: Option<String>,
}

/// MP4 / QuickTime : boîtes `moov/mvhd` (durée) et `moov/trak/{tkhd,mdia/hdlr}` (pistes).
pub mod mp4 {
    use super::*;

    fn header<R: Read>(r: &mut R) -> Option<(u64, [u8; 4], u64)> {
        let mut h = [0u8; 8];
        r.read_exact(&mut h).ok()?;
        let size32 = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as u64;
        let kind = [h[4], h[5], h[6], h[7]];
        let (size, head) = if size32 == 1 {
            let mut b = [0u8; 8];
            r.read_exact(&mut b).ok()?;
            (u64::from_be_bytes(b), 16)
        } else {
            (size32, 8)
        };
        Some((size, kind, head))
    }

    /// Lit les boîtes enfants d'un contenu en mémoire.
    fn children(buf: &[u8]) -> Vec<([u8; 4], &[u8])> {
        let mut out = Vec::new();
        let mut i = 0usize;
        while i + 8 <= buf.len() {
            let size32 = u32::from_be_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]) as usize;
            let kind = [buf[i + 4], buf[i + 5], buf[i + 6], buf[i + 7]];
            let (size, head) = if size32 == 1 {
                if i + 16 > buf.len() {
                    break;
                }
                let mut b = [0u8; 8];
                b.copy_from_slice(&buf[i + 8..i + 16]);
                (u64::from_be_bytes(b) as usize, 16)
            } else if size32 == 0 {
                (buf.len() - i, 8)
            } else {
                (size32, 8)
            };
            if size < head || i + size > buf.len() {
                break;
            }
            out.push((kind, &buf[i + head..i + size]));
            i += size;
        }
        out
    }

    fn be32(b: &[u8], at: usize) -> Option<u32> {
        b.get(at..at + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn be64(b: &[u8], at: usize) -> Option<u64> {
        b.get(at..at + 8).map(|s| u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
    }

    pub fn probe<R: Read + Seek>(mut r: R) -> Option<Container> {
        // Trouve `moov` parmi les boîtes de premier niveau (il peut être après `mdat`).
        let moov = loop {
            let (size, kind, head) = header(&mut r)?;
            if &kind == b"moov" {
                if size < head || size > 256 * 1024 * 1024 {
                    return None;
                }
                let mut buf = vec![0u8; (size - head) as usize];
                r.read_exact(&mut buf).ok()?;
                break buf;
            }
            if size == 0 {
                return None;
            }
            if size < head {
                return None;
            }
            r.seek(SeekFrom::Current((size - head) as i64)).ok()?;
        };
        let mut out = Container::default();
        for (kind, body) in children(&moov) {
            match &kind {
                b"mvhd" => {
                    let version = *body.first()?;
                    let (scale, dur) = if version == 1 {
                        (be32(body, 20)?, be64(body, 24)?)
                    } else {
                        (be32(body, 12)?, be32(body, 16)? as u64)
                    };
                    if scale > 0 {
                        out.duration = dur as f64 / scale as f64;
                    }
                }
                b"trak" => {
                    let parts = children(body);
                    let mut handler = None;
                    let mut size = None;
                    let mut codec = None;
                    for (k, b) in &parts {
                        if k == b"tkhd" && b.len() >= 8 {
                            // Largeur et hauteur : les 8 derniers octets, en virgule fixe 16.16.
                            let n = b.len();
                            let w = be32(b, n - 8)? >> 16;
                            let h = be32(b, n - 4)? >> 16;
                            size = Some((w, h));
                        }
                        if k == b"mdia" {
                            for (mk, mb) in children(b) {
                                if &mk == b"hdlr" && mb.len() >= 12 {
                                    handler = Some([mb[8], mb[9], mb[10], mb[11]]);
                                }
                                if &mk == b"minf" {
                                    codec = codec.or_else(|| sample_entry(mb));
                                }
                            }
                        }
                    }
                    match handler.as_ref() {
                        Some(b"vide") => {
                            out.video.get_or_insert(size.unwrap_or((0, 0)));
                            if codec.is_some() {
                                out.codec = codec;
                            }
                        }
                        Some(b"soun") if out.codec.is_none() => out.codec = codec,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        Some(out)
    }

    /// Codec de la première entrée de `minf/stbl/stsd`.
    fn sample_entry(minf: &[u8]) -> Option<String> {
        for (k, b) in children(minf) {
            if &k == b"stbl" {
                for (sk, sb) in children(b) {
                    if &sk == b"stsd" && sb.len() >= 16 {
                        let fourcc = &sb[12..16];
                        let name = match fourcc {
                            b"avc1" | b"avc3" => "H.264",
                            b"hvc1" | b"hev1" => "HEVC",
                            b"av01" => "AV1",
                            b"vp09" => "VP9",
                            b"mp4a" => "AAC",
                            b"alac" => "ALAC",
                            b"Opus" => "Opus",
                            b"fLaC" => "FLAC",
                            b"ac-3" => "AC-3",
                            b"ec-3" => "E-AC-3",
                            other => return std::str::from_utf8(other).ok().map(|s| s.trim().to_string()),
                        };
                        return Some(name.to_string());
                    }
                }
            }
        }
        None
    }
}

/// Matroska / WebM (EBML) : `Segment/Info` (durée, titre) et `Segment/Tracks` (pistes).
pub mod matroska {
    use super::*;

    const SEGMENT: u64 = 0x1853_8067;
    const INFO: u64 = 0x1549_A966;
    const TRACKS: u64 = 0x1654_AE6B;
    const CLUSTER: u64 = 0x1F43_B675;
    const TIMECODE_SCALE: u64 = 0x2AD7B1;
    const DURATION: u64 = 0x4489;
    const TITLE: u64 = 0x7BA9;
    const TRACK_ENTRY: u64 = 0xAE;
    const TRACK_TYPE: u64 = 0x83;
    const CODEC_ID: u64 = 0x86;
    const VIDEO: u64 = 0xE0;
    const PIXEL_WIDTH: u64 = 0xB0;
    const PIXEL_HEIGHT: u64 = 0xBA;

    /// Entier à longueur variable d'EBML. `keep_marker` garde le bit de longueur (identifiants).
    fn vint<R: Read>(r: &mut R, keep_marker: bool) -> Option<(u64, usize, bool)> {
        let mut first = [0u8; 1];
        r.read_exact(&mut first).ok()?;
        let len = first[0].leading_zeros() as usize + 1;
        if len > 8 {
            return None;
        }
        // Bits de valeur du premier octet (aucun quand la longueur est 8).
        let mask = (0xFFu32 >> len) as u64;
        let mut value = if keep_marker { first[0] as u64 } else { first[0] as u64 & mask };
        let mut all_ones = value == mask;
        for _ in 1..len {
            let mut b = [0u8; 1];
            r.read_exact(&mut b).ok()?;
            value = (value << 8) | b[0] as u64;
            all_ones &= b[0] == 0xFF;
        }
        Some((value, len, all_ones && !keep_marker))
    }

    fn element<R: Read>(r: &mut R) -> Option<(u64, Option<u64>)> {
        let (id, _, _) = vint(r, true)?;
        let (size, _, unknown) = vint(r, false)?;
        Some((id, if unknown { None } else { Some(size) }))
    }

    fn read_body<R: Read>(r: &mut R, size: u64) -> Option<Vec<u8>> {
        if size > 64 * 1024 * 1024 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        r.read_exact(&mut buf).ok()?;
        Some(buf)
    }

    fn uint(b: &[u8]) -> u64 {
        b.iter().take(8).fold(0u64, |acc, x| (acc << 8) | *x as u64)
    }

    fn float(b: &[u8]) -> f64 {
        match b.len() {
            4 => f32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64,
            8 => f64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
            _ => 0.0,
        }
    }

    /// Éléments enfants d'un contenu en mémoire.
    fn children(buf: &[u8]) -> Vec<(u64, &[u8])> {
        let mut out = Vec::new();
        let mut cur = std::io::Cursor::new(buf);
        while (cur.position() as usize) < buf.len() {
            let Some((id, size)) = element(&mut cur) else { break };
            let start = cur.position() as usize;
            let end = match size {
                Some(s) => start.saturating_add(s as usize),
                None => buf.len(),
            };
            if end > buf.len() {
                break;
            }
            out.push((id, &buf[start..end]));
            cur.set_position(end as u64);
        }
        out
    }

    pub fn probe<R: Read + Seek>(mut r: R) -> Option<Container> {
        // En-tête EBML.
        let (id, size) = element(&mut r)?;
        if id != 0x1A45_DFA3 {
            return None;
        }
        r.seek(SeekFrom::Current(size? as i64)).ok()?;
        let (id, _) = element(&mut r)?;
        if id != SEGMENT {
            return None;
        }
        let mut out = Container::default();
        let mut scale = 1_000_000u64;
        let mut raw_duration = None;
        let mut seen_tracks = false;
        while let Some((id, size)) = element(&mut r) {
            match id {
                INFO => {
                    let body = read_body(&mut r, size?)?;
                    for (cid, cb) in children(&body) {
                        match cid {
                            TIMECODE_SCALE => scale = uint(cb).max(1),
                            DURATION => raw_duration = Some(float(cb)),
                            TITLE => {
                                let t = String::from_utf8_lossy(cb).trim_end_matches('\0').trim().to_string();
                                if !t.is_empty() {
                                    out.title = Some(t);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                TRACKS => {
                    seen_tracks = true;
                    let body = read_body(&mut r, size?)?;
                    for (cid, entry) in children(&body) {
                        if cid != TRACK_ENTRY {
                            continue;
                        }
                        let mut kind = 0;
                        let mut codec = None;
                        let mut dims = None;
                        for (eid, eb) in children(entry) {
                            match eid {
                                TRACK_TYPE => kind = uint(eb),
                                CODEC_ID => codec = Some(String::from_utf8_lossy(eb).trim_end_matches('\0').to_string()),
                                VIDEO => {
                                    let mut w = 0;
                                    let mut h = 0;
                                    for (vid, vb) in children(eb) {
                                        match vid {
                                            PIXEL_WIDTH => w = uint(vb) as u32,
                                            PIXEL_HEIGHT => h = uint(vb) as u32,
                                            _ => {}
                                        }
                                    }
                                    dims = Some((w, h));
                                }
                                _ => {}
                            }
                        }
                        let codec = codec.map(|c| codec_name(&c));
                        if kind == 1 && out.video.is_none() {
                            out.video = Some(dims.unwrap_or((0, 0)));
                            out.codec = codec;
                        } else if kind == 2 && out.codec.is_none() {
                            out.codec = codec;
                        }
                    }
                }
                CLUSTER => break,
                _ => match size {
                    Some(s) => {
                        r.seek(SeekFrom::Current(s as i64)).ok()?;
                    }
                    None => break,
                },
            }
            if seen_tracks && raw_duration.is_some() {
                break;
            }
        }
        if let Some(d) = raw_duration {
            out.duration = d * scale as f64 / 1e9;
        }
        Some(out)
    }

    fn codec_name(id: &str) -> String {
        match id {
            "V_MPEG4/ISO/AVC" => "H.264",
            "V_MPEGH/ISO/HEVC" => "HEVC",
            "V_VP8" => "VP8",
            "V_VP9" => "VP9",
            "V_AV1" => "AV1",
            "A_OPUS" => "Opus",
            "A_VORBIS" => "Vorbis",
            "A_AAC" => "AAC",
            "A_FLAC" => "FLAC",
            "A_MPEG/L3" => "MPEG 1 Layer 3",
            other => other,
        }
        .to_string()
    }
}

#[cfg(test)]
mod tests;

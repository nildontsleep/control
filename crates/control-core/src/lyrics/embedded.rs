//! Paroles intégrées au fichier : USLT (ID3v2), `©lyr` (MP4), `LYRICS` / `UNSYNCEDLYRICS` (Vorbis, APE), et les
//! paroles synchronisées SYLT de l'ID3v2, converties en LRC. Lecture seule, avec lofty.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use lofty::config::ParseOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::id3::v2::{Frame, FrameFlags, Id3v2Tag, SyncTextContentType, SynchronizedTextFrame, TimestampFormat};
use lofty::prelude::ItemKey;
use lofty::probe::Probe;

use crate::lrc::{format_stamp, looks_synced};
use crate::probe::ext_of;

fn options() -> ParseOptions {
    ParseOptions::new().read_properties(false).read_cover_art(false)
}

/// Paroles intégrées au fichier, ou `None`. Des paroles minutées l'emportent sur du texte brut.
pub fn embedded_lyrics(path: &Path) -> Option<String> {
    let plain = generic_lyrics(path);
    if plain.as_deref().is_some_and(looks_synced) {
        return plain;
    }
    sylt_lyrics(path).or(plain)
}

/// Paroles vues par la couche commune de lofty (USLT, `©lyr`, `LYRICS`…), étiquette principale d'abord.
fn generic_lyrics(path: &Path) -> Option<String> {
    let tagged = Probe::open(path).ok()?.options(options()).guess_file_type().ok()?.read().ok()?;
    let primary = tagged.primary_tag_type();
    let mut tags: Vec<_> = tagged.tags().iter().collect();
    tags.sort_by_key(|t| t.tag_type() != primary);
    let unsynced = ItemKey::Unknown("UNSYNCEDLYRICS".into());
    tags.iter()
        .flat_map(|t| t.get_strings(&ItemKey::Lyrics).chain(t.get_strings(&unsynced)))
        .map(str::trim)
        .find(|s| !s.is_empty())
        .map(str::to_string)
}

/// Étiquette ID3v2 des formats qui en portent (lofty ne passe pas SYLT à sa couche commune).
fn id3v2_of(path: &Path) -> Option<Id3v2Tag> {
    let mut r = BufReader::new(File::open(path).ok()?);
    match ext_of(path).as_str() {
        "mp3" => lofty::mpeg::MpegFile::read_from(&mut r, options()).ok()?.remove_id3v2(),
        "wav" => lofty::iff::wav::WavFile::read_from(&mut r, options()).ok()?.remove_id3v2(),
        "aif" | "aiff" => lofty::iff::aiff::AiffFile::read_from(&mut r, options()).ok()?.remove_id3v2(),
        "aac" => lofty::aac::AacFile::read_from(&mut r, options()).ok()?.remove_id3v2(),
        _ => None,
    }
}

fn sylt_lyrics(path: &Path) -> Option<String> {
    let tag = id3v2_of(path)?;
    tag.into_iter().find_map(|frame| {
        let Frame::Binary(bin) = frame else { return None };
        if bin.id().as_str() != "SYLT" {
            return None;
        }
        let sylt = SynchronizedTextFrame::parse(&bin.data, FrameFlags::default()).ok()?;
        // Les repères en trames MPEG demanderaient le débit du fichier : rares, on les laisse.
        let lyrics = matches!(
            sylt.content_type,
            SyncTextContentType::Lyrics | SyncTextContentType::TextTranscription | SyncTextContentType::Other
        );
        if sylt.timestamp_format != TimestampFormat::MS || !lyrics {
            return None;
        }
        let lrc = sylt_to_lrc(&sylt.content);
        looks_synced(&lrc).then_some(lrc)
    })
}

/// Convertit le contenu d'une trame SYLT (millisecondes, texte) en LRC.
///
/// Deux usages existent : une entrée par ligne, ou une entrée par mot ou syllabe, où une entrée qui commence par
/// un saut de ligne ouvre une nouvelle ligne. Dans ce second cas, les mots gardent leur minutage (`<mm:ss.xx>`).
pub fn sylt_to_lrc(content: &[(u32, String)]) -> String {
    let by_syllable = content.iter().skip(1).any(|(_, s)| s.starts_with(['\n', '\r']));
    let mut out: Vec<String> = Vec::new();
    if !by_syllable {
        for (ms, text) in content {
            // Une entrée vide reste : c'est un interlude.
            if text.trim().is_empty() {
                out.push(format_stamp(u64::from(*ms)));
                continue;
            }
            for (k, line) in text.lines().enumerate() {
                // Une entrée sur plusieurs lignes : les suivantes au même instant.
                let line = line.trim();
                if k == 0 || !line.is_empty() {
                    out.push(format!("{}{line}", format_stamp(u64::from(*ms))));
                }
            }
        }
        return out.join("\n");
    }
    let mut current: Option<String> = None;
    for (ms, text) in content {
        let stamp = format_stamp(u64::from(*ms));
        let starts_line = current.is_none() || text.starts_with(['\n', '\r']);
        let word = text.trim_start_matches(['\n', '\r']);
        if starts_line {
            out.extend(current.take());
            current = Some(format!("{stamp}<{}>{word}", &stamp[1..stamp.len() - 1]));
        } else if let Some(line) = current.as_mut() {
            line.push('<');
            line.push_str(&stamp[1..stamp.len() - 1]);
            line.push('>');
            line.push_str(word);
        }
    }
    out.extend(current);
    out.join("\n")
}

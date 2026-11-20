use std::io::Cursor;

use image::DynamicImage;
use lofty::file::AudioFile;
use serde_json::json;

use super::*;
use crate::covers::Covers;
use crate::model::emit_nothing;

// ─── Fichiers de test, construits octet par octet ────────────────────────────

/// Un WAV muet valide d'une seconde (8 kHz, mono, 16 bits).
fn wav() -> Vec<u8> {
    let rate = 8000u32;
    let data_len = rate * 2;
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    v.resize(v.len() + data_len as usize, 0);
    v
}

/// Tag ID3v2.3 : cadres (identifiant, contenu).
fn id3v23(frames: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (id, data) in frames {
        body.extend_from_slice(id.as_bytes());
        body.extend_from_slice(&(data.len() as u32).to_be_bytes());
        body.extend_from_slice(&[0, 0]);
        body.extend_from_slice(data);
    }
    let n = body.len() as u32;
    let mut out = b"ID3\x03\x00\x00".to_vec();
    out.extend_from_slice(&[(n >> 21) as u8 & 0x7F, (n >> 14) as u8 & 0x7F, (n >> 7) as u8 & 0x7F, n as u8 & 0x7F]);
    out.extend(body);
    out
}

fn latin1(s: &str) -> Vec<u8> {
    let mut v = vec![0u8];
    v.extend_from_slice(s.as_bytes());
    v
}

/// MP3 : vingt trames MPEG-1 Layer III (128 kb/s, 44,1 kHz) de silence, précédées d'un tag éventuel.
fn mp3(tag: Vec<u8>) -> Vec<u8> {
    let mut v = tag;
    for _ in 0..20 {
        let mut frame = vec![0u8; 417];
        frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
        v.extend(frame);
    }
    v
}

/// FLAC : STREAMINFO puis un bloc de commentaires Vorbis, sans trame audio (lofty n'en a pas besoin).
fn flac(comments: &[&str]) -> Vec<u8> {
    let mut v = b"fLaC".to_vec();
    v.extend_from_slice(&[0x00, 0, 0, 34]);
    v.extend_from_slice(&4096u16.to_be_bytes());
    v.extend_from_slice(&4096u16.to_be_bytes());
    v.extend_from_slice(&[0; 6]);
    let packed: u64 = (44100u64 << 44) | (1 << 41) | (15 << 36) | 44100;
    v.extend_from_slice(&packed.to_be_bytes());
    v.extend_from_slice(&[0; 16]);
    let block = vorbis_block(comments);
    let len = block.len() as u32;
    v.extend_from_slice(&[0x84, (len >> 16) as u8, (len >> 8) as u8, len as u8]);
    v.extend(block);
    v.extend_from_slice(FLAC_AUDIO);
    v
}

/// Octets « audio » d'un FLAC de test : ils doivent ressortir intacts.
const FLAC_AUDIO: &[u8] = &[0xFF, 0xF8, 0x69, 0x08, 0x00, 0x00, 0xAA, 0x55, 0x12, 0x34];

fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

/// M4A minimal : `ftyp`, `moov` (avec `mvhd` et un titre dans `udta/meta/ilst`), puis `mdat`.
fn m4a(title: &str) -> Vec<u8> {
    let mut data = vec![0, 0, 0, 1, 0, 0, 0, 0];
    data.extend_from_slice(title.as_bytes());
    let ilst = mp4_box(b"ilst", &mp4_box(b"\xa9nam", &mp4_box(b"data", &data)));
    let mut hdlr = vec![0u8; 8];
    hdlr.extend_from_slice(b"mdir");
    hdlr.extend_from_slice(&[0; 13]);
    let mut meta = vec![0u8; 4];
    meta.extend(mp4_box(b"hdlr", &hdlr));
    meta.extend(ilst);
    let mut mvhd = vec![0u8; 100];
    mvhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
    mvhd[16..20].copy_from_slice(&1000u32.to_be_bytes());
    let mut moov = mp4_box(b"mvhd", &mvhd);
    moov.extend(mp4_box(b"udta", &mp4_box(b"meta", &meta)));
    let mut v = mp4_box(b"ftyp", b"M4A \0\0\0\0M4A isom");
    v.extend(mp4_box(b"moov", &moov));
    v.extend(mp4_box(b"mdat", &[0x21; 64]));
    v
}

fn vorbis_block(comments: &[&str]) -> Vec<u8> {
    let vendor = b"test";
    let mut v = (vendor.len() as u32).to_le_bytes().to_vec();
    v.extend_from_slice(vendor);
    v.extend_from_slice(&(comments.len() as u32).to_le_bytes());
    for c in comments {
        v.extend_from_slice(&(c.len() as u32).to_le_bytes());
        v.extend_from_slice(c.as_bytes());
    }
    v
}

fn ogg_crc(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &b in data {
        crc ^= (b as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04C1_1DB7 } else { crc << 1 };
        }
    }
    crc
}

fn ogg_page(kind: u8, granule: u64, seq: u32, packet: &[u8]) -> Vec<u8> {
    let mut segments = vec![255u8; packet.len() / 255];
    segments.push((packet.len() % 255) as u8);
    let mut page = b"OggS\x00".to_vec();
    page.push(kind);
    page.extend_from_slice(&granule.to_le_bytes());
    page.extend_from_slice(&0x1234u32.to_le_bytes());
    page.extend_from_slice(&seq.to_le_bytes());
    page.extend_from_slice(&[0; 4]);
    page.push(segments.len() as u8);
    page.extend(segments);
    page.extend_from_slice(packet);
    let crc = ogg_crc(&page);
    page[22..26].copy_from_slice(&crc.to_le_bytes());
    page
}

/// Opus : en-tête, commentaires, puis un paquet audio d'une seconde.
fn opus(comments: &[&str]) -> Vec<u8> {
    let mut head = b"OpusHead\x01\x02".to_vec();
    head.extend_from_slice(&312u16.to_le_bytes());
    head.extend_from_slice(&48000u32.to_le_bytes());
    head.extend_from_slice(&[0, 0, 0]);
    let mut tags = b"OpusTags".to_vec();
    tags.extend(vorbis_block(comments));
    let mut v = ogg_page(0x02, 0, 0, &head);
    v.extend(ogg_page(0x00, 0, 1, &tags));
    v.extend(ogg_page(0x04, 48312, 2, &[0xFC, 0xFF, 0xFE]));
    v
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    DynamicImage::new_rgb8(w, h).write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn jpeg(w: u32, h: u32) -> Vec<u8> {
    crate::covers::encode_jpeg(&DynamicImage::new_rgb8(w, h), 90)
}

struct Fixture {
    dir: tempfile::TempDir,
    lib: Arc<Library>,
    tags: Tags,
}

fn setup() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::db::open(&dir.path().join("library.db")).unwrap();
    let lib = Arc::new(Library::new(
        Arc::new(Mutex::new(db)),
        Covers::new(dir.path().join("covers")),
        dir.path().join("thumbs"),
        emit_nothing(),
    ));
    let tags = Tags::new(lib.clone(), emit_nothing());
    Fixture { dir, lib, tags }
}

impl Fixture {
    /// Écrit un fichier dans le dossier de musique et l'ajoute à la bibliothèque.
    fn add(&self, name: &str, bytes: &[u8]) -> (TrackId, PathBuf) {
        let music = self.dir.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        let path = music.join(name);
        std::fs::write(&path, bytes).unwrap();
        let ids = self.lib.add_paths(&[path.to_string_lossy().into_owned()], None);
        assert_eq!(ids.len(), 1, "{name} non ajouté");
        (ids[0], path)
    }

    fn fields(&self, id: TrackId) -> TagFields {
        let r = self.tags.read(&[id]).remove(0);
        assert!(r.error.is_none(), "{:?}", r.error);
        r.fields.unwrap()
    }

    fn embedded(&self, id: TrackId) -> usize {
        self.tags.read(&[id]).remove(0).embedded.unwrap()
    }

    fn write(&self, edit: TagEdit) -> WriteResult {
        let out = self.tags.write(&[edit]);
        assert!(out.undoable);
        out.results.into_iter().next().unwrap()
    }
}

fn set(json: Value) -> TagSet {
    serde_json::from_value(json).unwrap()
}

fn edit(id: TrackId, json: Value) -> TagEdit {
    TagEdit { id, set: set(json), cover: None, rename: None }
}

// ─── Aller-retour par format ─────────────────────────────────────────────────

#[test]
fn wav_round_trip_with_cover_rename_and_undo() {
    let fx = setup();
    let (id, path) = fx.add("Song.wav", &wav());
    assert_eq!(fx.fields(id), TagFields::default());
    assert_eq!(fx.embedded(id), 0);

    let staged =
        fx.tags.stage_cover(&CoverSource { data: Some(base64_png(2000, 1000)), ..Default::default() }).unwrap();
    assert_eq!((staged.width, staged.height), (2000, 1000));
    assert!(staged.preview.starts_with("data:image/jpeg;base64,"));

    let mut e = edit(
        id,
        json!({
            "title": " Titre ", "artist": "Artiste", "albumArtist": "Groupe", "album": "Album", "genre": "Rock",
            "composer": "Auteur", "comment": "Note", "lyrics": "la la la", "year": "2020", "trackNo": 3,
            "trackTotal": "10", "discNo": 1, "discTotal": 2
        }),
    );
    e.cover = Some(CoverEdit::Token { token: staged.token.clone() });
    e.rename = Some("03 - Titre".into());
    let res = fx.write(e);
    assert!(res.ok, "{res:?}");
    assert_eq!(res.renamed, Some(true));
    let renamed = path.with_file_name("03 - Titre.wav");
    assert_eq!(res.path.as_deref(), Some(renamed.to_string_lossy().as_ref()));
    assert!(renamed.exists() && !path.exists());

    let f = fx.fields(id);
    assert_eq!(f.title, "Titre");
    assert_eq!((f.artist.as_str(), f.album_artist.as_str(), f.album.as_str()), ("Artiste", "Groupe", "Album"));
    assert_eq!((f.genre.as_str(), f.composer.as_str()), ("Rock", "Auteur"));
    assert_eq!((f.comment.as_str(), f.lyrics.as_str()), ("Note", "la la la"));
    assert_eq!((f.year, f.track_no, f.track_total, f.disc_no, f.disc_total), (2020, 3, 10, 1, 2));
    assert_eq!(fx.embedded(id), 1);
    assert_eq!(embedded_lyrics(&renamed), "la la la");

    // La bibliothèque suit : même identifiant, nouveau chemin, nouveaux tags, pochette.
    let t = fx.lib.track(id).unwrap();
    assert_eq!(t.path, renamed.to_string_lossy());
    assert_eq!((t.title.as_str(), t.artist.as_str(), t.year, t.track_no), ("Titre", "Artiste", Some(2020), Some(3)));
    assert!(t.cover.is_some());
    // La pochette intégrée a été réduite à 1400 px.
    let (_, m) = media::Media::open(&renamed, false).unwrap();
    let pic = image::load_from_memory(&m.pictures()[0].data).unwrap();
    assert_eq!((pic.width(), pic.height()), (1400, 700));

    let undo = fx.tags.undo();
    assert_eq!(undo, UndoOutcome { restored: 1, failed: 0 });
    assert!(path.exists() && !renamed.exists());
    assert_eq!(fx.fields(id), TagFields::default());
    assert_eq!(fx.embedded(id), 0);
    let t = fx.lib.track(id).unwrap();
    assert_eq!((t.path.as_str(), t.title.as_str()), (path.to_string_lossy().as_ref(), "Song"));
    assert!(!fx.tags.can_undo());
    assert_eq!(fx.tags.undo(), UndoOutcome::default());
}

fn base64_png(w: u32, h: u32) -> String {
    use base64::Engine;
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png(w, h)))
}

#[test]
fn mp3_keeps_other_frames_and_id3v23() {
    let fx = setup();
    let mut private = b"owner@example\0".to_vec();
    private.extend_from_slice(&[1, 2, 3, 4]);
    let tag = id3v23(&[
        ("TIT2", latin1("Old")),
        ("TPE1", latin1("Someone")),
        ("TYER", latin1("1999")),
        ("PRIV", private.clone()),
    ]);
    let (id, path) = fx.add("a.mp3", &mp3(tag));
    let before = fx.fields(id);
    assert_eq!((before.title.as_str(), before.artist.as_str(), before.year), ("Old", "Someone", 1999));

    let res = fx.write(edit(id, json!({ "title": "Nouveau titre é", "trackNo": "4", "discNo": "1" })));
    assert!(res.ok, "{res:?}");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], b"ID3\x03", "la version 2.3 est gardée");
    let f = fx.fields(id);
    assert_eq!(
        (f.title.as_str(), f.artist.as_str(), f.year, f.track_no, f.disc_no),
        ("Nouveau titre é", "Someone", 1999, 4, 1)
    );
    // Le cadre PRIV, inconnu de l'éditeur, est toujours là.
    let file = lofty::mpeg::MpegFile::read_from(&mut std::fs::File::open(&path).unwrap(), Default::default()).unwrap();
    let id3 = file.id3v2().unwrap();
    assert!(id3.into_iter().any(|f| f.id_str() == "PRIV"));
    // L'audio n'a pas bougé : les trames sont à la fin du fichier, intactes.
    assert_eq!(&bytes[bytes.len() - 20 * 417..][..4], &[0xFF, 0xFB, 0x90, 0x64]);
    assert_eq!(fx.lib.track(id).unwrap().title, "Nouveau titre é");

    assert_eq!(fx.tags.undo().restored, 1);
    let f = fx.fields(id);
    assert_eq!((f.title.as_str(), f.track_no, f.disc_no, f.year), ("Old", 0, 0, 1999));
}

#[test]
fn mp3_without_tag_gets_id3v23_and_v24_stays() {
    let fx = setup();
    let (id, path) = fx.add("b.mp3", &mp3(Vec::new()));
    assert!(fx.write(edit(id, json!({ "title": "T", "year": 2001 }))).ok);
    assert_eq!(&std::fs::read(&path).unwrap()[..4], b"ID3\x03");
    assert_eq!(fx.fields(id).year, 2001);

    let mut v24 = id3v23(&[("TIT2", latin1("Four"))]);
    v24[3] = 4;
    let (id, path) = fx.add("c.mp3", &mp3(v24));
    assert!(fx.write(edit(id, json!({ "artist": "X" }))).ok);
    assert_eq!(&std::fs::read(&path).unwrap()[..4], b"ID3\x04");
    let f = fx.fields(id);
    assert_eq!((f.title.as_str(), f.artist.as_str()), ("Four", "X"));
}

#[test]
fn flac_round_trip_keeps_unknown_comments_and_multiple_values() {
    let fx = setup();
    let (id, path) = fx.add(
        "a.flac",
        &flac(&["TITLE=Old", "ARTIST=One", "ARTIST=Two", "TRACKNUMBER=2/9", "REPLAYGAIN_TRACK_GAIN=-6.5 dB"]),
    );
    let f = fx.fields(id);
    assert_eq!((f.title.as_str(), f.artist.as_str(), f.track_no, f.track_total), ("Old", "One; Two", 2, 9));

    let staged = fx.tags.stage_cover(&CoverSource { data: Some(base64_png(64, 64)), ..Default::default() }).unwrap();
    let mut e = edit(id, json!({ "title": "New", "trackTotal": 12, "genre": "Jazz" }));
    e.cover = Some(CoverEdit::Token { token: staged.token });
    assert!(fx.write(e).ok);
    let f = fx.fields(id);
    // L'artiste n'a pas été touché : ses deux valeurs restent.
    assert_eq!((f.title.as_str(), f.artist.as_str(), f.genre.as_str()), ("New", "One; Two", "Jazz"));
    assert_eq!((f.track_no, f.track_total), (2, 12));
    assert_eq!(fx.embedded(id), 1);
    let file = lofty::flac::FlacFile::read_from(&mut std::fs::File::open(&path).unwrap(), Default::default()).unwrap();
    let vc = file.vorbis_comments().unwrap();
    assert_eq!(vc.get("REPLAYGAIN_TRACK_GAIN"), Some("-6.5 dB"));
    assert_eq!(vc.get_all("ARTIST").count(), 2);
    assert_eq!(vc.vendor(), "test");
    assert!(std::fs::read(&path).unwrap().ends_with(FLAC_AUDIO), "l'audio est recopié tel quel");

    // Retirer la pochette, puis annuler : elle revient.
    let mut e = edit(id, json!({}));
    e.cover = Some(CoverEdit::Remove { remove: true });
    assert!(fx.write(e).ok);
    assert_eq!(fx.embedded(id), 0);
    assert_eq!(fx.tags.undo().restored, 1);
    assert_eq!(fx.embedded(id), 1);
    assert_eq!(fx.fields(id).title, "New");
}

#[test]
fn opus_round_trip() {
    let fx = setup();
    let (id, _) = fx.add("a.opus", &opus(&["TITLE=Old", "ENCODER=test"]));
    assert_eq!(fx.fields(id).title, "Old");
    assert!(fx.write(edit(id, json!({ "title": "New", "artist": "A", "year": 1987, "lyrics": "paroles" }))).ok);
    let f = fx.fields(id);
    assert_eq!((f.title.as_str(), f.artist.as_str(), f.year, f.lyrics.as_str()), ("New", "A", 1987, "paroles"));
    assert_eq!(fx.tags.undo().restored, 1);
    let f = fx.fields(id);
    assert_eq!((f.title.as_str(), f.artist.as_str(), f.year), ("Old", "", 0));
}

#[test]
fn m4a_round_trip() {
    let fx = setup();
    let (id, path) = fx.add("a.m4a", &m4a("Old"));
    assert_eq!(fx.fields(id).title, "Old");
    let staged = fx.tags.stage_cover(&CoverSource { data: Some(base64_png(32, 32)), ..Default::default() }).unwrap();
    let mut e = edit(
        id,
        json!({ "title": "New", "albumArtist": "AA", "composer": "C", "lyrics": "L", "trackNo": 2, "trackTotal": 8,
                "discNo": 1, "discTotal": 2, "year": 2010 }),
    );
    e.cover = Some(CoverEdit::Token { token: staged.token });
    assert!(fx.write(e).ok);
    let f = fx.fields(id);
    assert_eq!(
        (f.title.as_str(), f.album_artist.as_str(), f.composer.as_str(), f.lyrics.as_str()),
        ("New", "AA", "C", "L")
    );
    assert_eq!((f.track_no, f.track_total, f.disc_no, f.disc_total, f.year), (2, 8, 1, 2, 2010));
    assert_eq!(fx.embedded(id), 1);
    assert!(std::fs::read(&path).unwrap().ends_with(&[0x21; 64]));
    assert_eq!(fx.tags.undo().restored, 1);
    let f = fx.fields(id);
    assert_eq!((f.title.as_str(), f.track_no, f.year, f.album_artist.as_str()), ("Old", 0, 0, ""));
    assert_eq!(fx.embedded(id), 0);
}

// ─── Lots, erreurs, sémantique des champs ────────────────────────────────────

#[test]
fn batch_writes_only_given_fields() {
    let fx = setup();
    let (a, _) = fx.add("a.flac", &flac(&["TITLE=A", "ARTIST=First"]));
    let (b, _) = fx.add("b.flac", &flac(&["TITLE=B", "ARTIST=Second"]));
    // « Plusieurs valeurs » : seul l'album, modifié, est envoyé pour les deux morceaux.
    let out = fx.tags.write(&[edit(a, json!({ "album": "Commun" })), edit(b, json!({ "album": "Commun" }))]);
    assert!(out.results.iter().all(|r| r.ok));
    assert_eq!((fx.fields(a).artist, fx.fields(a).album), ("First".to_string(), "Commun".to_string()));
    assert_eq!((fx.fields(b).artist, fx.fields(b).album), ("Second".to_string(), "Commun".to_string()));
    assert_eq!(fx.tags.undo(), UndoOutcome { restored: 2, failed: 0 });
    assert_eq!(fx.fields(a).album, "");
    assert_eq!(fx.fields(b).title, "B");
}

#[test]
fn clearing_and_bounds() {
    let fx = setup();
    let (id, _) = fx.add("a.flac", &flac(&["TITLE=T", "DATE=2004-05-06", "TRACKNUMBER=3", "COMMENT=c"]));
    assert_eq!(fx.fields(id).year, 2004);
    assert!(fx.write(edit(id, json!({ "comment": "  ", "trackNo": "", "discNo": 5000, "year": null }))).ok);
    let f = fx.fields(id);
    assert_eq!((f.comment.as_str(), f.track_no, f.disc_no, f.year, f.title.as_str()), ("", 0, 999, 0, "T"));
}

#[test]
fn errors_do_not_stop_the_batch() {
    let fx = setup();
    let (ok, _) = fx.add("a.wav", &wav());
    let (mkv, _) = fx.add("v.mkv", b"not really");
    let out = fx.tags.write(&[
        edit(mkv, json!({ "title": "x" })),
        edit(9999, json!({ "title": "x" })),
        TagEdit { cover: Some(CoverEdit::Token { token: "nope".into() }), ..edit(ok, json!({})) },
        edit(ok, json!({ "title": "Fine" })),
    ]);
    let codes: Vec<_> = out.results.iter().map(|r| r.error.as_ref().map(|e| e.code)).collect();
    assert_eq!(codes, [Some(ErrorCode::Unsupported), Some(ErrorCode::Missing), Some(ErrorCode::Other), None]);
    assert_eq!(out.results[2].error.as_ref().unwrap().message, "Pochette expirée : choisissez-la de nouveau.");
    assert!(out.undoable);
    assert_eq!(fx.fields(ok).title, "Fine");

    let read = fx.tags.read(&[mkv, 9999]);
    assert_eq!(read[0].error.as_ref().unwrap().code, ErrorCode::Unsupported);
    assert_eq!(read[1].error.as_ref().unwrap().message, "Morceau introuvable.");

    // Un fichier corrompu : erreur propre, pas de panique.
    let (bad, _) = fx.add("bad.mp3", b"garbage garbage");
    let res = fx.tags.write(&[edit(bad, json!({ "title": "x" }))]);
    assert!(!res.results[0].ok);
    assert!(!res.undoable);
}

#[test]
fn rename_avoids_collisions_and_rejects_bad_names() {
    let fx = setup();
    let (a, pa) = fx.add("a.wav", &wav());
    let (b, _) = fx.add("b.wav", &wav());
    let res = fx.write(TagEdit { rename: Some("b".into()), ..edit(a, json!({})) });
    assert_eq!(res.path.as_deref(), Some(pa.with_file_name("b (2).wav").to_string_lossy().as_ref()));
    let res = fx.tags.write(&[TagEdit { rename: Some(" .. ".into()), ..edit(b, json!({})) }]);
    assert_eq!(res.results[0].error.as_ref().unwrap().message, "Nom de fichier invalide.");
    // Même nom : rien n'est renommé.
    let res = fx.write(TagEdit { rename: Some("b".into()), ..edit(b, json!({})) });
    assert_eq!(res.renamed, Some(false));
}

#[test]
fn json_shapes() {
    let s = set(json!({
        "title": "x", "artist": 5, "year": "", "trackNo": "7", "trackTotal": 3.9, "discNo": null, "discTotal": "abc",
        "unknown": true
    }));
    assert_eq!(s.title.as_deref(), Some("x"));
    assert_eq!(s.artist, None);
    assert_eq!(
        (s.year, s.track_no, s.track_total, s.disc_no, s.disc_total),
        (Some(0), Some(7), Some(3), Some(0), None)
    );
    assert!(set(json!({})).is_empty());

    let e: TagEdit =
        serde_json::from_value(json!({ "id": 4, "set": { "album": "A" }, "cover": { "remove": true } })).unwrap();
    assert_eq!(e.cover, Some(CoverEdit::Remove { remove: true }));
    let e: TagEdit = serde_json::from_value(json!({ "id": 4, "cover": { "token": "ab" }, "rename": "n" })).unwrap();
    assert_eq!(e.cover, Some(CoverEdit::Token { token: "ab".into() }));

    let ok = WriteResult { id: 1, ok: true, path: Some("/a".into()), renamed: Some(false), error: None };
    assert_eq!(serde_json::to_value(&ok).unwrap(), json!({ "id": 1, "ok": true, "path": "/a", "renamed": false }));
    let ko = WriteResult { id: 2, ok: false, path: None, renamed: None, error: Some(TagError::unsupported()) };
    assert_eq!(
        serde_json::to_value(&ko).unwrap(),
        json!({ "id": 2, "ok": false, "code": "unsupported", "message": "Format non modifiable (MKV, WebM…)." })
    );
    let fields = serde_json::to_value(TagFields::default()).unwrap();
    assert_eq!(fields["albumArtist"], "");
    assert_eq!(fields["trackTotal"], 0);
}

// ─── Pochettes ───────────────────────────────────────────────────────────────

#[test]
fn prepares_images() {
    let small = jpeg(300, 200);
    let p = prepare_image(&small).unwrap();
    assert_eq!(p.data, small, "un petit JPEG est gardé tel quel");
    assert_eq!((p.width, p.height, p.mime), (300, 200, "image/jpeg"));
    let p = prepare_image(&png(500, 3000)).unwrap();
    assert!(p.data.starts_with(&[0xFF, 0xD8]));
    let img = image::load_from_memory(&p.data).unwrap();
    assert_eq!((img.width(), img.height()), (233, 1400));
    assert!(prepare_image(b"not an image").is_err());
    assert!(prepare_image(&[]).is_err());
}

#[test]
fn staged_covers_expire() {
    let fx = setup();
    let file = fx.dir.path().join("cover.png");
    std::fs::write(&file, png(10, 10)).unwrap();
    let src = CoverSource { path: Some(file.to_string_lossy().into_owned()), ..Default::default() };
    let first = fx.tags.stage_cover(&src).unwrap();
    for _ in 0..STAGED_MAX {
        fx.tags.stage_cover(&src).unwrap();
    }
    assert!(fx.tags.staged(&first.token).is_none());
    assert!(fx.tags.stage_cover(&CoverSource::default()).is_err());
    assert!(fx.tags.stage_cover(&CoverSource { url: Some("http://insecure".into()), ..Default::default() }).is_err());
}

#[test]
fn parses_cover_search_results() {
    let itunes = json!({ "resultCount": 3, "results": [
        { "collectionName": "Discovery", "artistName": "Daft Punk", "releaseDate": "2001-03-12T08:00:00Z",
          "primaryGenreName": "Electronic", "trackCount": 14,
          "artworkUrl100": "https://is1.mzstatic.com/image/thumb/x/source/100x100bb.jpg" },
        { "collectionName": "No art", "artistName": "X" },
        { "collectionName": "Homework", "artistName": "Daft Punk",
          "artworkUrl100": "https://is1.mzstatic.com/image/thumb/y/source/100x100bb.jpg" }
    ]});
    let a = parse_itunes(&itunes);
    assert_eq!(a.len(), 2);
    assert_eq!(a[0].preview, "https://is1.mzstatic.com/image/thumb/x/source/300x300bb.jpg");
    assert_eq!(a[0].full, "https://is1.mzstatic.com/image/thumb/x/source/1200x1200bb.jpg");
    assert_eq!((a[0].year, a[0].tracks, a[0].genre.as_str()), (Some(2001), Some(14), "Electronic"));
    assert_eq!((a[1].year, a[1].tracks), (None, None));

    let deezer = json!({ "data": [
        { "title": "Discovery", "artist": { "name": "Daft Punk" }, "nb_tracks": 14,
          "cover_medium": "https://e-cdns/m.jpg", "cover_big": "https://e-cdns/b.jpg", "cover_xl": "https://e-cdns/xl.jpg" },
        { "title": "Alive 2007", "artist": { "name": "Daft Punk" }, "cover_big": "https://e-cdns/b2.jpg" },
        { "title": "Nothing" }
    ]});
    let d = parse_deezer(&deezer);
    assert_eq!(d.len(), 2);
    assert_eq!((d[0].preview.as_str(), d[0].full.as_str()), ("https://e-cdns/m.jpg", "https://e-cdns/xl.jpg"));
    assert_eq!((d[1].preview.as_str(), d[1].full.as_str()), ("https://e-cdns/b2.jpg", "https://e-cdns/b2.jpg"));
    assert_eq!(d[1].tracks, None);
    assert!(parse_deezer(&json!({ "error": {} })).is_empty());

    // En alternance, « Discovery » de Deezer écarté comme doublon.
    let merged = merge_hits(a, d);
    let names: Vec<_> = merged.iter().map(|h| (h.source.as_str(), h.title.as_str())).collect();
    assert_eq!(names, [("Apple Music", "Discovery"), ("Apple Music", "Homework"), ("Deezer", "Alive 2007")]);
    let json = serde_json::to_value(&merged[2]).unwrap();
    assert_eq!(json["year"], Value::Null);
    assert_eq!(json["source"], "Deezer");

    let many: Vec<_> = (0..20)
        .map(|i| CoverHit {
            source: "Deezer".into(),
            title: format!("T{i}"),
            artist: String::new(),
            year: None,
            genre: String::new(),
            tracks: None,
            preview: String::new(),
            full: String::new(),
        })
        .collect();
    assert_eq!(merge_hits(Vec::new(), many).len(), 16);
}

#[test]
fn safe_names() {
    assert_eq!(safe_name("AC/DC: Back in Black?"), "AC_DC_ Back in Black_");
    assert_eq!(safe_name("  ..Hello   world.. "), "Hello world");
    assert_eq!(safe_name("con"), "");
    assert_eq!(safe_name("COM1"), "");
    assert_eq!(safe_name("Com1x"), "Com1x");
    assert_eq!(safe_name("a\u{1}b"), "a_b");
    assert_eq!(safe_name(&"é".repeat(300)).chars().count(), 180);
}

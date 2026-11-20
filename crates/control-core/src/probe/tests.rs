use std::io::Cursor;

use super::*;

fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

fn tkhd(w: u32, h: u32) -> Vec<u8> {
    let mut b = vec![0u8; 84 - 8];
    let n = b.len();
    b[n - 8..n - 4].copy_from_slice(&(w << 16).to_be_bytes());
    b[n - 4..].copy_from_slice(&(h << 16).to_be_bytes());
    mp4_box(b"tkhd", &b)
}

fn hdlr(kind: &[u8; 4]) -> Vec<u8> {
    let mut b = vec![0u8; 24];
    b[8..12].copy_from_slice(kind);
    mp4_box(b"hdlr", &b)
}

fn stsd(fourcc: &[u8; 4]) -> Vec<u8> {
    let mut b = vec![0u8; 8];
    b[7] = 1;
    b.extend_from_slice(&16u32.to_be_bytes());
    b.extend_from_slice(fourcc);
    b.extend_from_slice(&[0u8; 8]);
    mp4_box(b"minf", &mp4_box(b"stbl", &mp4_box(b"stsd", &b)))
}

fn trak(kind: &[u8; 4], w: u32, h: u32, fourcc: &[u8; 4]) -> Vec<u8> {
    let mut mdia = hdlr(kind);
    mdia.extend(stsd(fourcc));
    let mut body = tkhd(w, h);
    body.extend(mp4_box(b"mdia", &mdia));
    mp4_box(b"trak", &body)
}

fn mvhd(scale: u32, dur: u32) -> Vec<u8> {
    let mut b = vec![0u8; 100];
    b[12..16].copy_from_slice(&scale.to_be_bytes());
    b[16..20].copy_from_slice(&dur.to_be_bytes());
    mp4_box(b"mvhd", &b)
}

#[test]
fn mp4_with_video_after_mdat() {
    let mut file = mp4_box(b"ftyp", b"isom\0\0\0\0");
    file.extend(mp4_box(b"mdat", &[0u8; 1000]));
    let mut moov = mvhd(1000, 125_500);
    moov.extend(trak(b"soun", 0, 0, b"mp4a"));
    moov.extend(trak(b"vide", 1920, 1080, b"avc1"));
    file.extend(mp4_box(b"moov", &moov));
    let c = mp4::probe(Cursor::new(file)).unwrap();
    assert_eq!(c.video, Some((1920, 1080)));
    assert!((c.duration - 125.5).abs() < 1e-9);
    assert_eq!(c.codec.as_deref(), Some("H.264"));
}

#[test]
fn mp4_audio_only() {
    let mut moov = mvhd(44100, 44100 * 3);
    moov.extend(trak(b"soun", 0, 0, b"mp4a"));
    let file = mp4_box(b"moov", &moov);
    let c = mp4::probe(Cursor::new(file)).unwrap();
    assert_eq!(c.video, None);
    assert_eq!(c.codec.as_deref(), Some("AAC"));
    assert!((c.duration - 3.0).abs() < 1e-9);
}

fn ebml(id: u64, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let idb = id.to_be_bytes();
    let first = idb.iter().position(|b| *b != 0).unwrap();
    out.extend_from_slice(&idb[first..]);
    // Taille sur 8 octets (0x01 + 7 octets).
    out.push(0x01);
    out.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
    out.extend_from_slice(body);
    out
}

#[test]
fn matroska_tracks_and_duration() {
    let mut file = ebml(0x1A45_DFA3, &ebml(0x4282, b"webm"));
    let mut info = ebml(0x2AD7B1, &1_000_000u32.to_be_bytes());
    info.extend(ebml(0x4489, &90_000.0f64.to_be_bytes()));
    info.extend(ebml(0x7BA9, b"Clip"));
    let mut video = ebml(0xB0, &[0x05, 0x00]);
    video.extend(ebml(0xBA, &[0x02, 0xD0]));
    let mut entry = ebml(0x83, &[1]);
    entry.extend(ebml(0x86, b"V_VP9"));
    entry.extend(ebml(0xE0, &video));
    let mut audio = ebml(0x83, &[2]);
    audio.extend(ebml(0x86, b"A_OPUS"));
    let mut tracks = ebml(0xAE, &audio);
    tracks.extend(ebml(0xAE, &entry));
    let mut segment = ebml(0x1549_A966, &info);
    segment.extend(ebml(0x1654_AE6B, &tracks));
    segment.extend(ebml(0x1F43_B675, &[0u8; 16]));
    file.extend(ebml(0x1853_8067, &segment));
    let c = matroska::probe(Cursor::new(file)).unwrap();
    assert_eq!(c.video, Some((1280, 720)));
    assert_eq!(c.codec.as_deref(), Some("VP9"));
    assert_eq!(c.title.as_deref(), Some("Clip"));
    assert!((c.duration - 90.0).abs() < 1e-9);
}

#[test]
fn unreadable_file_keeps_kind_from_extension() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("x.mkv");
    std::fs::write(&f, b"garbage").unwrap();
    let p = probe(&f);
    assert_eq!(p.kind, Kind::Video);
    assert_eq!(p.title, None);
    let f = dir.path().join("y.mp3");
    std::fs::write(&f, b"garbage").unwrap();
    assert_eq!(probe(&f).kind, Kind::Audio);
}

use super::*;
use crate::model::emit_nothing;

fn setup() -> (tempfile::TempDir, Library) {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::db::open(&dir.path().join("library.db")).unwrap();
    let lib = Library::new(
        Arc::new(Mutex::new(db)),
        Covers::new(dir.path().join("covers")),
        dir.path().join("thumbs"),
        emit_nothing(),
    );
    (dir, lib)
}

/// Un WAV muet valide de `secs` secondes (8 kHz, mono, 16 bits).
fn wav(secs: u32) -> Vec<u8> {
    let rate = 8000u32;
    let data_len = rate * secs * 2;
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

#[test]
fn scan_adds_updates_and_removes() {
    let (dir, lib) = setup();
    let music = dir.path().join("music");
    std::fs::create_dir_all(music.join("Album")).unwrap();
    std::fs::create_dir_all(music.join(".hidden")).unwrap();
    std::fs::write(music.join("Album/01 Song.wav"), wav(2)).unwrap();
    std::fs::write(music.join("Album/02 Other.wav"), wav(1)).unwrap();
    std::fs::write(music.join(".hidden/x.wav"), wav(1)).unwrap();
    std::fs::write(music.join("notes.txt"), b"x").unwrap();
    let root = music.to_string_lossy().into_owned();

    let r = lib.scan(std::slice::from_ref(&root));
    assert_eq!(r, ScanResult { added: 2, removed: 0, total: 2 });
    let tracks = lib.tracks();
    assert_eq!(tracks.len(), 2);
    let song = tracks.iter().find(|t| t.title == "01 Song").unwrap();
    assert!((song.duration - 2.0).abs() < 0.01, "durée {}", song.duration);
    assert_eq!(song.kind, Kind::Audio);

    // Rien n'a changé : rien n'est relu.
    assert_eq!(lib.scan(std::slice::from_ref(&root)), ScanResult { added: 0, removed: 0, total: 2 });

    std::fs::remove_file(music.join("Album/02 Other.wav")).unwrap();
    assert_eq!(lib.scan(std::slice::from_ref(&root)), ScanResult { added: 0, removed: 1, total: 1 });
    assert_eq!(lib.tracks().len(), 1);
    assert_eq!(lib.tracks()[0].id, song.id, "l'identifiant reste le même");
}

#[test]
fn empty_or_missing_roots_never_wipe() {
    let (dir, lib) = setup();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    std::fs::write(music.join("a.wav"), wav(1)).unwrap();
    let root = music.to_string_lossy().into_owned();
    lib.scan(std::slice::from_ref(&root));
    std::fs::remove_file(music.join("a.wav")).unwrap();
    // Dossier lisible mais vide : on suppose un disque mal monté, on garde.
    assert_eq!(lib.scan(std::slice::from_ref(&root)).removed, 0);
    std::fs::remove_dir_all(&music).unwrap();
    assert_eq!(lib.scan(&[root]).removed, 0);
    assert_eq!(lib.tracks().len(), 1);
}

#[test]
fn folder_cover_is_used() {
    let (dir, lib) = setup();
    let music = dir.path().join("m");
    std::fs::create_dir_all(&music).unwrap();
    std::fs::write(music.join("a.wav"), wav(1)).unwrap();
    let img = image::DynamicImage::new_rgb8(50, 50);
    img.save(music.join("Folder.JPG")).unwrap();
    lib.add_paths(&[music.to_string_lossy().into_owned()], Some("https://x"));
    let t = &lib.tracks()[0];
    assert!(t.cover.is_some());
    assert_eq!(t.source_url.as_deref(), Some("https://x"));
}

#[test]
fn playlists_and_m3u() {
    let (dir, lib) = setup();
    let music = dir.path().join("m");
    std::fs::create_dir_all(&music).unwrap();
    for n in ["a", "b", "c"] {
        std::fs::write(music.join(format!("{n}.wav")), wav(1)).unwrap();
    }
    let ids = lib.add_paths(&[music.to_string_lossy().into_owned()], None);
    assert_eq!(ids.len(), 3);
    let pl = lib.create_playlist("  ", PlaylistKind::Manual, None).unwrap();
    lib.add_to_playlist(pl, &[ids[2], ids[0], ids[2]]);
    let (pls, items) = lib.playlists();
    assert_eq!(pls[0].name, "Sans titre");
    assert_eq!(items.iter().map(|i| i.track_id).collect::<Vec<_>>(), vec![ids[2], ids[0], ids[2]]);
    let entries: Vec<i64> = items.iter().rev().map(|i| i.id).collect();
    lib.reorder_playlist(pl, &entries);
    let (_, items) = lib.playlists();
    assert_eq!(items.iter().map(|i| i.track_id).collect::<Vec<_>>(), vec![ids[2], ids[0], ids[2]].into_iter().rev().collect::<Vec<_>>());

    let m3u = dir.path().join("list.m3u8");
    assert_eq!(lib.export_m3u(&m3u, &ids).unwrap(), 3);
    let text = std::fs::read_to_string(&m3u).unwrap();
    assert!(text.starts_with("#EXTM3U\r\n#EXTINF:1,a\r\n"));
    // Chemins relatifs.
    std::fs::write(dir.path().join("rel.m3u"), "#EXTM3U\nm/b.wav\n./m/../m/c.wav\nmissing.mp3\n").unwrap();
    let imp = lib.import_m3u(&dir.path().join("rel.m3u")).unwrap();
    assert_eq!((imp.matched, imp.total), (2, 3));
    assert!(lib.update_playlist(imp.id, None, Some(" desc "), Some(true)));
    let (pls, _) = lib.playlists();
    let p = pls.iter().find(|p| p.id == imp.id).unwrap();
    assert_eq!((p.name.as_str(), p.description.as_str(), p.shared), ("rel", "desc", true));
}

#[test]
fn plays_are_counted() {
    let (dir, lib) = setup();
    std::fs::write(dir.path().join("a.wav"), wav(1)).unwrap();
    let ids = lib.add_paths(&[dir.path().join("a.wav").to_string_lossy().into_owned()], None);
    let play = lib.play_start(ids[0], 1.0).unwrap();
    lib.play_end(play, 0.9, true);
    let t = lib.track(ids[0]).unwrap();
    assert_eq!(t.play_count, 1);
    assert!(t.last_played.is_some());
}

#[test]
fn rename_keeps_identity() {
    let (dir, lib) = setup();
    let a = dir.path().join("a.wav");
    std::fs::write(&a, wav(1)).unwrap();
    let ids = lib.add_paths(&[a.to_string_lossy().into_owned()], None);
    let b = dir.path().join("b.wav");
    std::fs::rename(&a, &b).unwrap();
    let t = lib.refresh_track(ids[0], Some(&b.to_string_lossy())).unwrap();
    assert_eq!(t.id, ids[0]);
    assert_eq!(t.title, "b");
}

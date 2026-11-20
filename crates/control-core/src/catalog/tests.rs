use super::*;
use crate::model::{Kind, PlaylistKind, SmartRule, SmartSort};

fn tr(id: i64, title: &str, artist: &str, album: &str, album_artist: &str, path: &str) -> Track {
    Track {
        id,
        title: title.into(),
        artist: artist.into(),
        album: album.into(),
        album_artist: album_artist.into(),
        path: path.into(),
        duration: 100.0,
        ..Track::default()
    }
}

fn sample() -> Catalog {
    let mut v = vec![
        tr(1, "Two", "Alpha", "First", "", "/m/a/2.mp3"),
        tr(2, "One", "Alpha feat. Beta", "First", "", "/m/a/1.mp3"),
        tr(3, "Mix", "Gamma", "Hits", "", "/m/c/1.mp3"),
        tr(4, "Mix 2", "Delta", "Hits", "", "/m/c/2.mp3"),
        tr(5, "Clip", "Alpha", "", "", "/m/v.mp4"),
        tr(6, "Same", "ALPHA", "First", "", "/m/other/x.mp3"),
    ];
    v[0].track_no = Some(2);
    v[1].track_no = Some(1);
    v[2].year = Some(2001);
    v[3].year = Some(1999);
    v[4].kind = Kind::Video;
    v[0].play_count = 5;
    Catalog::build(v, Vec::new(), Vec::new())
}

#[test]
fn albums_group_by_title_and_folder() {
    let c = sample();
    // « First » existe dans deux dossiers sans artiste d'album : deux albums.
    assert_eq!(c.albums.len(), 3);
    let first = &c.albums[c.album_by_key[&album_key_of(c.track(1).unwrap()).unwrap()]];
    assert_eq!(first.tracks.iter().map(|&i| c.entries[i].track.id).collect::<Vec<_>>(), vec![2, 1]);
    assert_eq!(first.artist, "Alpha");
    assert!(!first.various);
    let hits = c.albums.iter().find(|a| a.title == "Hits").unwrap();
    assert!(hits.various);
    assert_eq!(hits.artist_key, None);
    assert_eq!(hits.year, Some(1999));
}

#[test]
fn artists_split_and_merge_spellings() {
    let c = sample();
    let alpha = &c.artists[c.artist_by_key["alpha"]];
    assert_eq!(alpha.name, "Alpha");
    assert_eq!(alpha.tracks.len(), 3, "vidéos exclues");
    assert_eq!(c.entries[alpha.tracks[0]].track.id, 1, "le plus écouté d'abord");
    assert!(c.artist_by_key.contains_key("beta"));
    assert_eq!(c.entry(2).unwrap().artist_label, "Alpha, Beta");
}

#[test]
fn search_tokens_and_accents() {
    let mut c = sample();
    c.entries[0].search = crate::util::fold("Café Alpha");
    let r = c.search("cafe ALP").unwrap();
    assert_eq!(r.tracks, vec![0]);
    assert!(c.search("   ").is_none());
    assert_eq!(c.search("clip").unwrap().videos.len(), 1);
}

#[test]
fn smart_playlist_rows() {
    let mut c = sample();
    let spec = SmartSpec {
        match_: "all".into(),
        rules: vec![SmartRule { field: "artist".into(), op: "contains".into(), value: serde_json::json!("alpha") }],
        sort: SmartSort { key: "title".into(), dir: "desc".into() },
        limit: 2,
    };
    c.playlists.push(Playlist {
        id: 9,
        name: "S".into(),
        kind: PlaylistKind::Smart,
        rules: Some(spec),
        ..Playlist::default()
    });
    let p = c.playlist(9).unwrap().clone();
    let rows = c.playlist_rows(&p, 0);
    let titles: Vec<&str> = rows.iter().map(|r| c.entries[r.entry].track.title.as_str()).collect();
    assert_eq!(titles, vec!["Two", "Same"]);
}

#[test]
fn track_sorting() {
    let c = sample();
    let mut list = c.audio.clone();
    sort_tracks(&c, &mut list, "title", false);
    let titles: Vec<&str> = list.iter().map(|&i| c.entries[i].track.title.as_str()).collect();
    assert_eq!(titles, vec!["Mix", "Mix 2", "One", "Same", "Two"]);
    sort_tracks(&c, &mut list, "playCount", true);
    assert_eq!(c.entries[list[0]].track.id, 1);
}

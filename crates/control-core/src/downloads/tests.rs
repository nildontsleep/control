use std::ffi::OsString;

use serde_json::json;

use super::info::{VideoInfo, parse_info, parse_search, title_from_url};
use super::job::{INTERRUPTED, load_jobs, save_jobs};
use super::ytdlp::*;
use super::*;
use crate::model::Kind;
use crate::settings::Dl;

fn strings(a: &[OsString]) -> Vec<String> {
    a.iter().map(|s| s.to_string_lossy().into_owned()).collect()
}

/// Valeur qui suit une option dans une liste d'arguments.
fn after(a: &[String], opt: &str) -> Option<String> {
    a.iter().position(|x| x == opt).and_then(|i| a.get(i + 1).cloned())
}

fn spec<'a>(kind: Kind, format: &'a str, quality: &'a str) -> DownloadSpec<'a> {
    DownloadSpec {
        url: "https://www.youtube.com/watch?v=x",
        is_playlist: false,
        kind,
        format,
        quality,
        parse_title: true,
    }
}

// ─── Arguments ───────────────────────────────────────────────────────────────

#[test]
fn audio_formats_and_quality() {
    assert_eq!(audio_format("mp3"), "mp3");
    assert_eq!(audio_format("FLAC"), "flac");
    assert_eq!(audio_format("original"), "best");
    assert_eq!(audio_format("best"), "best");
    assert_eq!(audio_format("--exec rm"), "mp3");

    let out = Path::new("/musique");
    let a = strings(&download_args(&spec(Kind::Audio, "mp3", "256"), out, Extras::default()));
    assert_eq!(after(&a, "-f").as_deref(), Some("bestaudio/best"));
    assert!(a.contains(&"-x".to_string()));
    assert_eq!(after(&a, "--audio-format").as_deref(), Some("mp3"));
    assert_eq!(after(&a, "--audio-quality").as_deref(), Some("256K"));
    assert_eq!(after(&a, "--parse-metadata").as_deref(), Some("title:%(artist)s - %(title)s"));
    assert_eq!(after(&a, "--replace-in-metadata").as_deref(), Some("title"));
    assert_eq!(after(&a, "title").as_deref(), Some(NOISE));
    assert!(a.contains(&"--no-playlist".to_string()));
    assert!(a.contains(&"--embed-metadata".to_string()) && a.contains(&"--embed-thumbnail".to_string()));
    assert_eq!(after(&a, "--convert-thumbnails").as_deref(), Some("jpg"));
    assert_eq!(after(&a, "-o").unwrap(), Path::new("/musique").join("%(title).150B.%(ext)s").to_string_lossy());
    assert_eq!(&a[a.len() - 2..], ["--", "https://www.youtube.com/watch?v=x"]);
    assert!(a.iter().any(|x| x.starts_with("download:CTRLPROG\t%(progress.status)s\t")));
    assert!(a.iter().any(|x| x == "after_move:CTRLFILE\t%(filepath)s\t%(webpage_url)s"));

    // Débit seulement pour le mp3 ; sans « Artiste - Titre » si demandé.
    let mut s = spec(Kind::Audio, "flac", "320");
    s.parse_title = false;
    let a = strings(&download_args(&s, out, Extras::default()));
    assert_eq!(after(&a, "--audio-format").as_deref(), Some("flac"));
    assert!(after(&a, "--audio-quality").is_none());
    assert!(after(&a, "--parse-metadata").is_none());
    let a = strings(&download_args(&spec(Kind::Audio, "mp3", "best"), out, Extras::default()));
    assert!(after(&a, "--audio-quality").is_none());
}

#[test]
fn video_and_playlist_arguments() {
    let out = Path::new("/videos");
    let a = strings(&download_args(&spec(Kind::Video, "mp3", "1080"), out, Extras::default()));
    assert_eq!(after(&a, "-S").as_deref(), Some("res:1080,vcodec:h264,acodec:aac"));
    assert_eq!(after(&a, "--merge-output-format").as_deref(), Some("mp4"));
    assert!(a.contains(&"--embed-chapters".to_string()));
    assert!(after(&a, "-x").is_none() && after(&a, "--audio-format").is_none());
    for q in ["480", "720", "1440", "2160"] {
        let a = strings(&download_args(&spec(Kind::Video, "", q), out, Extras::default()));
        assert_eq!(after(&a, "-S").unwrap(), format!("res:{q},vcodec:h264,acodec:aac"));
    }
    let a = strings(&download_args(&spec(Kind::Video, "", "best"), out, Extras::default()));
    assert_eq!(after(&a, "-S").as_deref(), Some("vcodec:h264,acodec:aac"));

    let mut s = spec(Kind::Audio, "opus", "");
    s.is_playlist = true;
    let ffmpeg = Path::new("/opt/bin/ffmpeg");
    let deno = Path::new("/opt/deno/deno");
    let extras = Extras { cookies: vec!["--cookies".into(), "/c.txt".into()], ffmpeg: Some(ffmpeg), deno: Some(deno) };
    let a = strings(&download_args(&s, out, extras));
    assert!(a.contains(&"--yes-playlist".to_string()));
    assert!(after(&a, "-o").unwrap().ends_with("%(playlist_title&{}/|)s%(title).150B.%(ext)s"));
    assert_eq!(after(&a, "--cookies").as_deref(), Some("/c.txt"));
    assert_eq!(after(&a, "--ffmpeg-location").as_deref(), Some("/opt/bin"));
    assert_eq!(after(&a, "--js-runtimes").as_deref(), Some("deno:/opt/deno/deno"));
    assert_eq!(a.last().unwrap(), "https://www.youtube.com/watch?v=x");
}

#[test]
fn cookies_order() {
    let mut dl = Dl::default();
    let all = |_: &Path| true;
    let none = |_: &Path| false;
    assert!(cookie_args(None, &dl, all).is_empty());
    dl.cookies = "firefox".into();
    assert_eq!(strings(&cookie_args(None, &dl, all)), ["--cookies-from-browser", "firefox"]);
    dl.cookies_file = "/home/moi/cookies.txt".into();
    assert_eq!(strings(&cookie_args(None, &dl, all)), ["--cookies", "/home/moi/cookies.txt"]);
    assert_eq!(strings(&cookie_args(None, &dl, none)), ["--cookies-from-browser", "firefox"], "fichier absent");
    let forced = Path::new("/srv/u/cookies.txt");
    assert_eq!(strings(&cookie_args(Some(forced), &dl, all)), ["--cookies", "/srv/u/cookies.txt"]);
    let only_forced_missing = |p: &Path| p != forced;
    assert_eq!(strings(&cookie_args(Some(forced), &dl, only_forced_missing)), ["--cookies", "/home/moi/cookies.txt"]);
}

#[test]
fn info_and_search_arguments() {
    let a = strings(&info_args("https://www.youtube.com/watch?v=x&list=RDx", vec![]));
    assert_eq!(&a[..2], ["-J", "--flat-playlist"]);
    assert!(a.contains(&"--no-playlist".to_string()));
    assert!(
        !strings(&info_args("https://www.youtube.com/playlist?list=PLx", vec![])).contains(&"--no-playlist".into())
    );
    assert!(is_mix("https://youtube.com/watch?list=RDabc"));
    assert!(!is_mix("https://youtube.com/watch?v=1&xlist=RD"));

    assert_eq!(search_target("soundcloud", "  daft   punk ", 12).as_deref(), Some("scsearch12:daft punk"));
    assert_eq!(search_target("youtube", "x", 99).as_deref(), Some("ytsearch25:x"));
    assert_eq!(search_target("youtube", "x", 0).as_deref(), Some("ytsearch1:x"));
    assert_eq!(search_target("bandcamp", "x", 5), None);
    assert_eq!(search_target("youtube", "   ", 5), None);
    assert_eq!(search_target("youtube", &"a".repeat(500), 5).unwrap().len(), "ytsearch5:".len() + 200);
    let a = strings(&search_args("ytsearch5:x", vec!["--cookies".into(), "/c".into()]));
    assert_eq!(&a[a.len() - 4..], ["--cookies", "/c", "--", "ytsearch5:x"]);
}

// ─── Lecture de la sortie ────────────────────────────────────────────────────

#[test]
fn progress_lines() {
    let line = "CTRLPROG\tdownloading\t512\t1024\tNA\t2048.5\t7\t2\t4\tUn titre\tavec tabulation";
    let Line::Progress(p) = parse_line(line) else { panic!("progression attendue") };
    assert_eq!(p.status, "downloading");
    assert_eq!((p.downloaded, p.size, p.speed, p.eta), (Some(512.0), Some(1024.0), Some(2048.5), Some(7.0)));
    assert_eq!((p.index, p.count), (Some(2), Some(4)));
    assert_eq!(p.title.as_deref(), Some("Un titre\tavec tabulation"));
    // Élément 2 sur 4 à moitié : 1,5 / 4.
    assert!((p.overall(None) - 0.375).abs() < 1e-9);

    // Taille inconnue : l'estimation sert ; rien du tout : 0.
    let Line::Progress(p) = parse_line("CTRLPROG\tdownloading\t50\tNA\t200\tNA\tNA\tNA\tNA\tNA") else { panic!() };
    assert_eq!((p.size, p.speed, p.eta, p.index, p.title.as_deref()), (Some(200.0), None, None, None, None));
    assert!((p.overall(None) - 0.25).abs() < 1e-9);
    assert!((p.overall(Some(2)) - 0.25).abs() < 1e-9, "sans rang, le nombre ne sert pas");
    let Line::Progress(p) = parse_line("CTRLPROG\tdownloading\t50\tNA\tNA\tNA\tNA\t3\tNA\tx") else { panic!() };
    assert_eq!(p.overall(None), 0.0);
    assert!((p.overall(Some(4)) - 0.5).abs() < 1e-9, "rang 3 sur 4 (nombre connu par l'analyse)");
    let Line::Progress(p) = parse_line("CTRLPROG\tfinished\t2000\t1000\tNA\tNA\tNA\tNA\tNA\tx") else { panic!() };
    assert_eq!(p.overall(None), 1.0, "jamais plus de 100 %");

    assert_eq!(parse_line("CTRLPOST\tstarted\tMon titre"), Line::Post(Some("Mon titre".into())));
    assert_eq!(parse_line("CTRLPOST\tstarted\tNA"), Line::Post(None));
    assert_eq!(
        parse_line("CTRLFILE\t/m/a b.mp3\thttps://x.org/a"),
        Line::File { path: "/m/a b.mp3".into(), url: Some("https://x.org/a".into()) }
    );
    assert_eq!(parse_line("CTRLFILE\t/m/a.mp3\tNA"), Line::File { path: "/m/a.mp3".into(), url: None });
    assert_eq!(parse_line("CTRLFILE\t\tNA"), Line::Other);
    assert_eq!(parse_line("ERROR: [youtube] abc: Video unavailable"), Line::Error("abc: Video unavailable".into()));
    assert_eq!(
        parse_line("ERROR: Requested format is not available"),
        Line::Error("Requested format is not available".into())
    );
    assert_eq!(parse_line("[download] Destination: x"), Line::Other);
    assert_eq!(num("1e3"), Some(1000.0));
    assert_eq!(num("None"), None);
    assert_eq!(num("inf"), None);
}

#[test]
fn title_cleanup_and_split() {
    assert_eq!(clean_title("Song (Official Video)"), "Song");
    assert_eq!(clean_title("Song [Official Music Video]"), "Song");
    assert_eq!(clean_title("Song (Official Lyric Video)"), "Song");
    assert_eq!(clean_title("Song [Lyrics]"), "Song");
    assert_eq!(clean_title("Song (lyric video)"), "Song");
    assert_eq!(clean_title("Song (4K Remaster)"), "Song");
    assert_eq!(clean_title("Song (Remastered 2009) [HD]"), "Song");
    assert_eq!(clean_title("Song (Audio)"), "Song");
    assert_eq!(clean_title("Song (feat. Moi)"), "Song (feat. Moi)");
    assert_eq!(clean_title("Video Killed the Radio Star"), "Video Killed the Radio Star");

    assert_eq!(
        split_artist_title("Daft Punk - One More Time (Official Video)"),
        (Some("Daft Punk".into()), "One More Time".into())
    );
    assert_eq!(split_artist_title("A - B - C"), (Some("A - B".into()), "C".into()));
    assert_eq!(split_artist_title("Pas d'artiste [Lyrics]"), (None, "Pas d'artiste".into()));
    assert_eq!(split_artist_title("Tiret-collé"), (None, "Tiret-collé".into()));
}

// ─── JSON de yt-dlp ──────────────────────────────────────────────────────────

#[test]
fn video_info() {
    let data = json!({
        "title": "Daft Punk - One More Time", "track": "One More Time", "artist": "Daft Punk",
        "uploader": "Daft Punk - Topic", "duration": 320.5,
        "thumbnails": [{"url": "https://i/1.jpg"}, {"id": "x"}, {"url": "https://i/2.jpg"}, {"id": "y"}]
    });
    let v = serde_json::to_value(parse_info(&data, "https://youtu.be/x")).unwrap();
    assert_eq!(
        v,
        json!({
            "type": "video", "title": "One More Time", "uploader": "Daft Punk", "duration": 320.5,
            "thumbnail": "https://i/2.jpg", "music": true
        })
    );
    let bare = parse_info(
        &json!({"title": "", "duration": 0, "uploader": "Chaîne"}),
        "https://soundcloud.com/moi/mon-super_titre",
    );
    let v = serde_json::to_value(bare).unwrap();
    assert_eq!(v["title"], "Mon super titre");
    assert_eq!(v["duration"], Value::Null);
    assert_eq!(v["music"], false);
    let topic = parse_info(&json!({"title": "x", "channel": "Artiste - Topic"}), "u");
    assert!(matches!(topic, Info::Video(VideoInfo { music: true, .. })));
}

#[test]
fn playlist_info() {
    let entries: Vec<Value> =
        (0..70).map(|i| json!({"title": format!("T{i}"), "duration": i, "channel": "C"})).collect();
    let mut data = json!({"_type": "playlist", "title": "Ma liste", "uploader": "Moi", "entries": entries});
    data["entries"].as_array_mut().unwrap().push(Value::Null);
    let Info::Playlist(p) = parse_info(&data, "u") else { panic!("playlist attendue") };
    assert_eq!((p.title.as_str(), p.count, p.entries.len()), ("Ma liste", 70, 60));
    assert_eq!(p.entries[0].duration, None, "durée nulle : absente");
    assert_eq!(p.entries[1].uploader.as_deref(), Some("C"));
    assert!(!p.music);
    assert_eq!(p.thumbnail, None);

    let data = json!({
        "_type": "playlist", "playlist_count": 12,
        "entries": [{"url": "https://soundcloud.com/a/city-ports", "thumbnail": "https://t/1.jpg", "artist": "A"}]
    });
    let v = serde_json::to_value(parse_info(&data, "u")).unwrap();
    assert_eq!(v["type"], "playlist");
    assert_eq!(v["title"], "Playlist");
    assert_eq!(v["count"], 12);
    assert_eq!(v["thumbnail"], "https://t/1.jpg");
    assert_eq!(v["entries"][0]["title"], "City ports");
    assert_eq!(v["music"], true);
}

#[test]
fn search_results() {
    let data = json!({"entries": [
        {"title": "Un", "url": "https://www.youtube.com/watch?v=1", "channel": "C", "duration": 61},
        {"webpage_url": "https://soundcloud.com/a/deux-titres", "url": "ignored", "artist": "A"},
        {"title": "Sans lien"},
        {"title": "Lien bizarre", "url": "javascript:alert(1)"},
        null
    ]});
    let r = parse_search(&data, "youtube");
    assert_eq!(r.len(), 2);
    assert_eq!((r[0].title.as_deref(), r[0].uploader.as_deref(), r[0].duration), (Some("Un"), Some("C"), Some(61.0)));
    assert_eq!(r[1].url, "https://soundcloud.com/a/deux-titres");
    assert_eq!(r[1].title.as_deref(), Some("Deux titres"));
    assert_eq!(r[1].source, "youtube");
    assert!(parse_search(&json!({}), "youtube").is_empty());
}

#[test]
fn titles_from_urls() {
    assert_eq!(title_from_url("https://soundcloud.com/x/sets/city-ports").as_deref(), Some("City ports"));
    assert_eq!(title_from_url("https://x.org/a/%C3%A9t%C3%A9_chaud/").as_deref(), Some("Été chaud"));
    assert_eq!(title_from_url("https://x.org/"), None);
    assert_eq!(title_from_url("https://x.org/%E0%A4%A"), None);
    assert_eq!(title_from_url("pas une adresse"), None);
}

// ─── Téléchargements de la file ──────────────────────────────────────────────

#[test]
fn add_options_and_new_jobs() {
    let o = AddOptions::from_value(&json!({
        "url": "https://soundcloud.com/a/sets/b", "title": "B", "isPlaylist": true, "kind": "audio",
        "format": "flac", "quality": "320", "createPlaylist": true, "playlistName": "", "count": 9
    }));
    let j = o.into_job(5, 1000, false);
    assert_eq!((j.id, j.created_at, j.status, j.kind), (5, 1000, JobStatus::Queued, Kind::Audio));
    assert_eq!((j.format.as_str(), j.quality.as_str(), j.count), ("flac", "320", Some(9)));
    assert_eq!(j.playlist_name.as_deref(), Some("B"));
    assert!(j.parse_title && j.is_playlist);

    let j = AddOptions::from_value(&json!({"url": "https://x.org/mon-titre", "kind": "video", "parseTitle": false}))
        .into_job(1, 0, true);
    assert_eq!(
        (j.title.as_str(), j.kind, j.format.as_str(), j.quality.as_str()),
        ("Mon titre", Kind::Video, "mp3", "320")
    );
    assert!(!j.parse_title);
    assert_eq!(j.playlist_name, None, "pas une playlist : le réglage ne s'applique pas");

    // createPlaylist absent : réglage dl.createPlaylist pour une playlist.
    let j = AddOptions::from_value(&json!({"url": "https://x.org/", "isPlaylist": true})).into_job(1, 0, true);
    assert_eq!(j.playlist_name.as_deref(), Some("Téléchargements"));
    assert_eq!(j.title, "https://x.org/");
    let j =
        AddOptions::from_value(&json!({"url": "u", "isPlaylist": true, "createPlaylist": false})).into_job(1, 0, true);
    assert_eq!(j.playlist_name, None);

    let v = serde_json::to_value(&j).unwrap();
    for key in [
        "id",
        "url",
        "title",
        "isPlaylist",
        "kind",
        "format",
        "quality",
        "parseTitle",
        "playlistName",
        "status",
        "progress",
        "speed",
        "eta",
        "current",
        "index",
        "count",
        "files",
        "trackIds",
        "error",
        "createdAt",
    ] {
        assert!(v.get(key).is_some(), "{key}");
    }
    assert!(v.get("playlistId").is_none() && v.get("skipped").is_none());
}

#[test]
fn job_progress_and_end() {
    let mut j = AddOptions::from_value(&json!({"url": "u", "isPlaylist": true, "count": 3})).into_job(1, 0, false);
    let p = |line: &str| match parse_line(line) {
        Line::Progress(p) => p,
        other => panic!("{other:?}"),
    };
    j.apply_progress(&p("CTRLPROG\tdownloading\t50\t100\tNA\t10\t5\t2\tNA\tDeux"));
    assert!((j.progress - 0.5).abs() < 1e-9, "{}", j.progress);
    assert_eq!((j.index, j.count, j.current.as_deref(), j.speed), (Some(2), Some(3), Some("Deux"), Some(10.0)));
    j.apply_progress(&p("CTRLPROG\tdownloading\t0\t100\tNA\tNA\tNA\t1\t3\tNA"));
    assert!((j.progress - 0.5).abs() < 1e-9, "ne recule pas");
    assert_eq!(j.current.as_deref(), Some("Deux"));
    j.apply_post(Some("Deux".into()));
    assert_eq!((j.status, j.speed), (JobStatus::Processing, None));

    // Fin avec des éléments manqués.
    j.files = vec!["/a".into(), "/b".into()];
    j.finish(1, Some("Video unavailable".into()), Some(7));
    assert_eq!((j.status, j.progress, j.skipped, j.playlist_id), (JobStatus::Done, 1.0, Some(1), Some(7)));
    assert_eq!(j.error.as_deref(), Some("Video unavailable"));
    j.finish(0, Some("x".into()), None);
    assert_eq!((j.skipped, j.error.as_deref()), (Some(0), None));

    j.files.clear();
    j.finish(0, None, None);
    assert_eq!((j.status, j.error.as_deref()), (JobStatus::Error, Some("yt-dlp:NOFILES")));
    j.finish(2, None, None);
    assert_eq!(j.error.as_deref(), Some("yt-dlp:EXIT:2"));
    j.finish(2, Some("Unsupported URL".into()), None);
    assert_eq!(j.error.as_deref(), Some("Unsupported URL"));
    j.canceled = true;
    j.finish(-1, Some("x".into()), None);
    assert_eq!(j.status, JobStatus::Canceled);
}

#[test]
fn jobs_file_roundtrip() {
    let text = json!({"jobs": [
        {"id": 1, "url": "u1", "title": "Un", "kind": "audio", "status": "done", "progress": 1, "files": ["/a"],
         "trackIds": [3], "createdAt": 5, "playlistId": 2, "skipped": 0, "futur": {"x": 1}},
        {"id": 2, "url": "u2", "title": "Deux", "kind": "video", "status": "downloading", "speed": 100, "eta": 3},
        {"id": 3, "url": "u3", "kind": "audio", "status": "queued", "count": 2.0},
        {"id": 4, "url": "u4", "kind": "audio", "status": "étrange"},
        {"id": "pas un nombre"},
        "pas un objet"
    ]})
    .to_string();
    let jobs = load_jobs(&text);
    assert_eq!(jobs.iter().map(|j| j.id).collect::<Vec<_>>(), [1, 2, 3, 4]);
    assert_eq!(
        (jobs[0].status, jobs[0].playlist_id, jobs[0].track_ids.as_slice()),
        (JobStatus::Done, Some(2), &[3][..])
    );
    assert_eq!(jobs[0].extra.get("futur"), Some(&json!({"x": 1})), "champ inconnu gardé");
    assert_eq!((jobs[1].status, jobs[1].error.as_deref(), jobs[1].speed), (JobStatus::Error, Some(INTERRUPTED), None));
    assert_eq!((jobs[2].status, jobs[2].count), (JobStatus::Error, Some(2)));
    assert_eq!(jobs[3].status, JobStatus::Error);

    let saved: Value = serde_json::from_str(&save_jobs(&jobs)).unwrap();
    assert_eq!(saved["jobs"][0]["futur"], json!({"x": 1}));
    assert_eq!(saved["jobs"][0]["trackIds"], json!([3]));
    assert_eq!(load_jobs(&save_jobs(&jobs)), jobs);

    let many: Vec<Job> = (0..100).map(|id| Job { id, ..Job::default() }).collect();
    let saved: Value = serde_json::from_str(&save_jobs(&many)).unwrap();
    assert_eq!(saved["jobs"].as_array().unwrap().len(), 80);
    assert_eq!(saved["jobs"][0]["id"], 20);
    assert!(load_jobs("pas du json").is_empty());
}

#[test]
fn concurrency_setting() {
    assert_eq!(concurrency_limit(2.0), 2);
    assert_eq!(concurrency_limit(0.0), 2);
    assert_eq!(concurrency_limit(f64::NAN), 2);
    assert_eq!(concurrency_limit(-3.0), 1);
    assert_eq!(concurrency_limit(9.0), 4);
    assert_eq!(concurrency_limit(2.5), 3);
}

// ─── File complète, avec un faux yt-dlp ──────────────────────────────────────

#[cfg(unix)]
mod queue {
    use super::*;
    use crate::covers::Covers;
    use crate::model::Event;

    /// Un WAV muet valide d'une seconde.
    fn wav() -> Vec<u8> {
        let (rate, data_len) = (8000u32, 16000u32);
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

    /// Faux yt-dlp : selon l'adresse (dernier argument), échoue, attend, ou « télécharge » deux fichiers.
    const SCRIPT: &str = r#"#!/bin/sh
for a in "$@"; do url="$a"; done
printf '%s\n' "$@" > "$ROOT/args.txt"
case "$url" in
  *fail*) echo "ERROR: [generic] Unsupported URL: $url" >&2; exit 1 ;;
  *slow*) sleep 30; exit 0 ;;
esac
printf 'CTRLPROG\tdownloading\t50\t100\tNA\t1000\t3\t1\t3\tUn\n'
cp "$ROOT/src.wav" "$ROOT/out/un.wav"
printf 'CTRLFILE\t%s\thttps://exemple.org/un\n' "$ROOT/out/un.wav"
printf 'CTRLPOST\tstarted\tDeux\n'
cp "$ROOT/src.wav" "$ROOT/out/deux.wav"
printf 'CTRLFILE\t%s\tNA\n' "$ROOT/out/deux.wav"
echo "ERROR: [youtube] trois: Video unavailable" >&2
exit 1
"#;

    struct Rig {
        _dir: tempfile::TempDir,
        root: PathBuf,
        dl: Downloads,
        library: Arc<Library>,
        events: Arc<Mutex<Vec<(String, Value)>>>,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::write(root.join("src.wav"), wav()).unwrap();
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let script = SCRIPT.replace("$ROOT", &root.to_string_lossy());
        std::fs::write(bin.join("yt-dlp"), script).unwrap();
        crate::tools::archive::make_executable(&bin.join("yt-dlp")).unwrap();

        let events: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let sink = events.clone();
        let emit: Emit = Arc::new(move |e| {
            if let Event::Json(ch, v) = e {
                sink.lock().push((ch.to_string(), v));
            }
        });
        let db = crate::db::open(&root.join("library.db")).unwrap();
        let library = Arc::new(Library::new(
            Arc::new(Mutex::new(db)),
            Covers::new(root.join("covers")),
            root.join("thumbs"),
            crate::model::emit_nothing(),
        ));
        let store = Store::open(root.join("settings.json"));
        store.patch(&json!({ "downloadDir": root.join("out").to_string_lossy() }));
        let tools = Arc::new(Tools::new(&bin, root.join("tmp"), crate::model::emit_nothing()));
        let dl = Downloads::new(DownloadsConfig {
            tools,
            library: library.clone(),
            store,
            file: root.join("downloads.json"),
            emit,
            cookies_file: Some(root.join("cookies.txt")),
            default_dir: root.join("default"),
        });
        Rig { _dir: dir, root, dl, library, events }
    }

    fn wait_done(dl: &Downloads, id: JobId) -> Job {
        let start = Instant::now();
        loop {
            let job = dl.job(id).unwrap();
            if !job.status.is_active() {
                return job;
            }
            assert!(start.elapsed() < Duration::from_secs(20), "toujours en cours : {job:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn playlist_download_adds_tracks_and_playlist() {
        let r = rig();
        let job = r.dl.add(AddOptions::from_value(&json!({
            "url": "https://exemple.org/sets/ma-liste", "title": "Ma liste", "isPlaylist": true,
            "createPlaylist": true, "count": 3
        })));
        let job = wait_done(&r.dl, job.id);
        assert_eq!(job.status, JobStatus::Done, "{job:?}");
        assert_eq!(job.files.len(), 2);
        assert_eq!(job.track_ids.len(), 2);
        assert_eq!((job.skipped, job.error.as_deref()), (Some(1), Some("trois: Video unavailable")));
        let tracks = r.library.tracks();
        assert_eq!(tracks.len(), 2);
        let un = tracks.iter().find(|t| t.path.ends_with("un.wav")).unwrap();
        assert_eq!(un.source_url.as_deref(), Some("https://exemple.org/un"));
        let deux = tracks.iter().find(|t| t.path.ends_with("deux.wav")).unwrap();
        assert_eq!(deux.source_url.as_deref(), Some("https://exemple.org/sets/ma-liste"));
        let (playlists, items) = r.library.playlists();
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].name, "Ma liste");
        assert_eq!(Some(playlists[0].id), job.playlist_id);
        assert_eq!(items.len(), 2);

        let args = std::fs::read_to_string(r.root.join("args.txt")).unwrap();
        assert!(
            args.contains("--yes-playlist") && !args.contains("--cookies"),
            "pas de cookies.txt : pas de --cookies"
        );

        let events = r.events.lock();
        let updates: Vec<&Value> = events.iter().filter(|(c, _)| c == "dl:update").map(|(_, v)| v).collect();
        assert!(updates.iter().any(|v| v["status"] == "processing" && v["current"] == "Deux"));
        assert!(updates.iter().any(|v| v["status"] == "downloading" && v["index"] == 1 && v["count"] == 3));
        assert_eq!(updates.last().unwrap()["status"], "done");
        drop(events);

        // L'historique survit à un redémarrage.
        r.dl.flush();
        let saved = load_jobs(&std::fs::read_to_string(r.root.join("downloads.json")).unwrap());
        assert_eq!(saved.last().map(|j| (j.id, j.status)), Some((job.id, JobStatus::Done)));
    }

    #[test]
    fn errors_cancel_retry_and_removal() {
        let r = rig();
        crate::downloads::cookies::save(
            &r.root.join("cookies.txt"),
            ".youtube.com\tTRUE\t/\tTRUE\t1893456000\tSID\tabc",
        )
        .unwrap();
        let failed = r.dl.add(AddOptions::from_value(&json!({"url": "https://exemple.org/fail"})));
        let failed = wait_done(&r.dl, failed.id);
        assert_eq!(failed.status, JobStatus::Error);
        assert_eq!(failed.error.as_deref(), Some("Unsupported URL: https://exemple.org/fail"));
        let args = std::fs::read_to_string(r.root.join("args.txt")).unwrap();
        assert!(args.contains(&format!("--cookies\n{}", r.root.join("cookies.txt").display())));

        let slow = r.dl.add(AddOptions::from_value(&json!({"url": "https://exemple.org/slow"})));
        let start = Instant::now();
        while !r.dl.inner.state.lock().procs.contains_key(&slow.id) {
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(10));
        }
        r.dl.remove(slow.id);
        assert!(r.dl.job(slow.id).is_some(), "un téléchargement en cours ne se retire pas");
        r.dl.cancel(slow.id);
        let slow = wait_done(&r.dl, slow.id);
        assert_eq!(slow.status, JobStatus::Canceled);
        assert!(start.elapsed() < Duration::from_secs(10), "arrêté sans attendre la fin du sleep");

        r.dl.retry(failed.id);
        let again = wait_done(&r.dl, failed.id);
        assert_eq!(again.status, JobStatus::Error);

        r.dl.remove(failed.id);
        assert!(r.dl.job(failed.id).is_none());
        r.dl.clear_finished();
        assert!(r.dl.list().is_empty());
        let events = r.events.lock();
        assert_eq!(events.last().map(|(c, v)| (c.as_str(), v.clone())), Some(("dl:reset", json!([]))));
    }

    #[test]
    fn queue_respects_concurrency() {
        let r = rig();
        r.dl.inner.cfg.store.patch(&json!({"dl": {"concurrency": 1}}));
        let a = r.dl.add(AddOptions::from_value(&json!({"url": "https://exemple.org/slow-a"})));
        let b = r.dl.add(AddOptions::from_value(&json!({"url": "https://exemple.org/slow-b"})));
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(r.dl.job(b.id).unwrap().status, JobStatus::Queued);
        r.dl.cancel(b.id);
        assert_eq!(r.dl.job(b.id).unwrap().status, JobStatus::Canceled, "en file : annulé tout de suite");
        r.dl.cancel(a.id);
        assert_eq!(wait_done(&r.dl, a.id).status, JobStatus::Canceled);

        // Sans yt-dlp : erreur claire.
        std::fs::remove_file(r.root.join("bin/yt-dlp")).unwrap();
        if crate::process::which("yt-dlp").is_none() {
            let c = r.dl.add(AddOptions::from_value(&json!({"url": "https://exemple.org/x"})));
            assert_eq!(wait_done(&r.dl, c.id).error.as_deref(), Some(NOT_INSTALLED_LONG));
            assert_eq!(r.dl.info("https://exemple.org/x").unwrap_err().to_string(), NOT_INSTALLED);
        }
    }
}

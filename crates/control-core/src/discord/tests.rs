use super::*;
use crate::model::{Kind, Track};
use crate::settings::Discord as Opts;
use std::path::Path;

fn track(kind: Kind) -> Track {
    Track {
        id: 7,
        kind,
        title: "Le titre".into(),
        artist: "L’artiste".into(),
        album: "L’album".into(),
        cover: Some("0123456789abcdef0123".into()),
        ..Track::default()
    }
}

fn np(t: &Track, playing: bool) -> NowPlaying<'_> {
    NowPlaying { track: Some(t), playing, position: 30.0, duration: 200.0, streaming: None, paused_since: None }
}

#[test]
fn frames_roundtrip_in_pieces() {
    let a = encode(OP_FRAME, &json!({"cmd": "SET_ACTIVITY", "nonce": "1"}));
    assert_eq!(&a[..4], &1i32.to_le_bytes());
    assert_eq!(i32::from_le_bytes([a[4], a[5], a[6], a[7]]) as usize, a.len() - 8);
    let b = encode(OP_PING, &json!({"x": 1}));
    let mut stream = a.clone();
    stream.extend_from_slice(&b);
    let mut d = Decoder::default();
    // Un octet à la fois : rien tant qu'une trame n'est pas complète.
    let mut got = Vec::new();
    for byte in &stream[..a.len() - 1] {
        got.extend(d.push(std::slice::from_ref(byte)));
    }
    assert!(got.is_empty());
    got.extend(d.push(&stream[a.len() - 1..]));
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], (OP_FRAME, Some(json!({"cmd": "SET_ACTIVITY", "nonce": "1"}))));
    assert_eq!(got[1].0, OP_PING);
    // Trame illisible : signalée sans contenu ; longueur absurde : le tampon est vidé.
    let mut bad = 1i32.to_le_bytes().to_vec();
    bad.extend_from_slice(&3i32.to_le_bytes());
    bad.extend_from_slice(b"{{{");
    assert_eq!(d.push(&bad), vec![(1, None)]);
    let mut huge = 1i32.to_le_bytes().to_vec();
    huge.extend_from_slice(&(-5i32).to_le_bytes());
    assert!(d.push(&huge).is_empty());
    assert!(d.push(&encode(OP_PONG, &json!({}))).len() == 1);
}

#[test]
fn pipe_locations() {
    let env = |k: &str| match k {
        "XDG_RUNTIME_DIR" => Some("/run/user/1000".to_string()),
        _ => None,
    };
    let p = pipe_paths(false, &env, None);
    assert_eq!(p[0], PathBuf::from("/run/user/1000/discord-ipc-0"));
    assert_eq!(p[9], PathBuf::from("/run/user/1000/discord-ipc-9"));
    assert!(p.contains(&PathBuf::from("/run/user/1000/snap.discord/discord-ipc-0")));
    assert!(p.contains(&PathBuf::from("/run/user/1000/app/com.discordapp.Discord/discord-ipc-3")));
    assert!(p.contains(&PathBuf::from("/run/user/1000/app/dev.vencord.Vesktop/discord-ipc-0")));
    assert_eq!(p.len(), 50);
    let none = |_: &str| None;
    assert_eq!(pipe_paths(false, &none, None)[0], PathBuf::from("/tmp/discord-ipc-0"));
    let tmp = |k: &str| (k == "TMPDIR").then(|| "/var/tmp".to_string());
    assert_eq!(pipe_paths(false, &tmp, None)[0], PathBuf::from("/var/tmp/discord-ipc-0"));
    let forced = pipe_paths(false, &env, Some(Path::new("/x")));
    assert_eq!(forced.len(), 10);
    let win = pipe_paths(true, &none, None);
    assert_eq!(win[0], PathBuf::from(r"\\.\pipe\discord-ipc-0"));
    assert_eq!(win.len(), 10);
}

#[test]
fn listening_activity() {
    let t = track(Kind::Audio);
    let a = build_activity(&np(&t, true), &Opts::default(), &Texts::fr(), 1_000_000).unwrap();
    assert_eq!(a.kind, ActivityKind::Listening);
    assert_eq!(a.details, "Le titre");
    assert_eq!(a.state.as_deref(), Some("L’artiste"));
    assert_eq!(a.image_text.as_deref(), Some("L’album"));
    assert_eq!(a.icon, Some(Icon::Play));
    assert_eq!(a.icon_text.as_deref(), Some("Écoute"));
    assert_eq!(a.cover, Some(CoverRef::Cover("0123456789abcdef0123".into())));
    let j = activity_json(&a, Some("https://files.catbox.moe/x.jpg"), 1_000_000);
    assert_eq!(j["type"], 2);
    assert_eq!(j["status_display_type"], 1);
    assert_eq!(j["assets"]["large_image"], "https://files.catbox.moe/x.jpg");
    assert_eq!(j["assets"]["small_image"], "play");
    assert_eq!(j["timestamps"]["start"], 1_000_000 - 30_000);
    assert_eq!(j["timestamps"]["end"], 1_000_000 - 30_000 + 200_000);
    assert_eq!(j["instance"], false);
    // Sans pochette publique, le logo de control.
    assert_eq!(activity_json(&a, None, 0)["assets"]["large_image"], "control");
}

#[test]
fn paused_and_idle() {
    let t = track(Kind::Audio);
    let mut n = np(&t, false);
    n.paused_since = Some(0);
    let a = build_activity(&n, &Opts::default(), &Texts::en(), IDLE_CLEAR_MS).unwrap();
    assert_eq!(a.icon, Some(Icon::Pause));
    assert_eq!(a.icon_text.as_deref(), Some("Paused"));
    assert!(activity_json(&a, None, 0).get("timestamps").is_none(), "pas de barre en pause");
    assert!(build_activity(&n, &Opts::default(), &Texts::en(), IDLE_CLEAR_MS + 1).is_none());
}

#[test]
fn video_and_options() {
    let mut t = track(Kind::Video);
    t.cover = None;
    t.artist.clear();
    let a = build_activity(&np(&t, true), &Opts::default(), &Texts::fr(), 0).unwrap();
    assert_eq!(a.kind, ActivityKind::Watching);
    assert_eq!(a.icon, Some(Icon::Video));
    assert_eq!(a.state.as_deref(), Some("Vidéo locale"));
    assert_eq!(a.cover, Some(CoverRef::VideoThumb(7)));
    assert_eq!(activity_json(&a, None, 0)["type"], 3);
    assert_eq!(activity_json(&a, None, 0)["status_display_type"], 2);
    let no_video = Opts { video: false, ..Opts::default() };
    assert!(build_activity(&np(&t, true), &no_video, &Texts::fr(), 0).is_none());
    let no_covers = Opts { covers: false, ..Opts::default() };
    assert_eq!(build_activity(&np(&t, true), &no_covers, &Texts::fr(), 0).unwrap().cover, None);
    let off = Opts { enabled: false, ..Opts::default() };
    assert!(build_activity(&np(&t, true), &off, &Texts::fr(), 0).is_none());
    let mut unknown = track(Kind::Audio);
    unknown.artist = "Unknown".into();
    let a = build_activity(&np(&unknown, true), &Opts::default(), &Texts::fr(), 0).unwrap();
    assert_eq!(a.state.as_deref(), Some("Artiste inconnu"));
}

#[test]
fn hidden_titles() {
    let t = track(Kind::Audio);
    let hide = Opts { hide_titles: true, ..Opts::default() };
    let a = build_activity(&np(&t, true), &hide, &Texts::fr(), 0).unwrap();
    assert_eq!(a.details, "Écoute de la musique");
    assert_eq!((a.state, a.image_text, a.cover), (None, None, None));
    let s = Streaming {
        kind: "movie".into(),
        title: "Film".into(),
        subtitle: Some("2024".into()),
        poster: Some("https://image.tmdb.org/p.jpg".into()),
        position: 10.0,
        duration: 100.0,
        playing: true,
        at: 0,
    };
    let n = NowPlaying { streaming: Some(&s), ..np(&t, true) };
    let a = build_activity(&n, &hide, &Texts::fr(), 5000).unwrap();
    assert_eq!(a.details, "Regarde un film");
    assert_eq!(a.image, None);
    let a = build_activity(&n, &Opts::default(), &Texts::fr(), 5000).unwrap();
    assert_eq!(a.details, "Film");
    assert_eq!(a.icon, Some(Icon::Watching));
    assert_eq!(a.image.as_deref(), Some("https://image.tmdb.org/p.jpg"));
    assert!((a.position - 15.0).abs() < 1e-9, "position avancée depuis la mesure");
    // Streaming désactivé : retour à la bibliothèque.
    let no_stream = Opts { streaming: false, ..Opts::default() };
    assert_eq!(build_activity(&n, &no_stream, &Texts::fr(), 0).unwrap().details, "Le titre");
}

#[test]
fn texts_are_clipped() {
    assert_eq!(clip("  a  ", 128).as_deref(), Some("a "));
    assert_eq!(clip(" \n ", 128), None);
    assert_eq!(clip("un   deux\ttrois", 128).as_deref(), Some("un deux trois"));
    let long = "é".repeat(200);
    let c = clip(&long, 128).unwrap();
    assert_eq!(c.chars().count(), 128);
    assert!(c.ends_with('…'));
    let a = Activity {
        kind: ActivityKind::Listening,
        details: "x".into(),
        state: Some("  ".into()),
        image: Some("http://insecure/p.jpg".into()),
        image_text: None,
        icon: None,
        icon_text: None,
        playing: true,
        position: f64::NAN,
        duration: -3.0,
        cover: Some(CoverRef::Cover("pas-une-empreinte".into())),
    }
    .sanitized()
    .unwrap();
    assert_eq!((a.details.as_str(), a.state, a.image, a.cover), ("x ", None, None, None));
    assert_eq!((a.position, a.duration), (0.0, 0.0));
}

#[test]
fn feed_sends_only_real_changes() {
    let t = track(Kind::Audio);
    let opts = Opts::default();
    let texts = Texts::fr();
    let mut feed = PresenceFeed::default();
    let mut n = np(&t, true);
    let a = build_activity(&n, &opts, &texts, 0);
    assert!(feed.should_send(a.as_ref(), 0));
    // 1,5 s plus tard, la position a avancé comme prévu : rien à envoyer.
    n.position = 31.5;
    assert!(!feed.should_send(build_activity(&n, &opts, &texts, 1500).as_ref(), 1500));
    // Un saut dans le morceau : envoyé.
    n.position = 120.0;
    assert!(feed.should_send(build_activity(&n, &opts, &texts, 3000).as_ref(), 3000));
    // Pause : envoyée, puis effacée au bout de 10 minutes.
    n.playing = false;
    feed.playback_changed(false, 4000);
    n.paused_since = feed.paused_since();
    assert!(feed.should_send(build_activity(&n, &opts, &texts, 4000).as_ref(), 4000));
    assert!(!feed.should_send(build_activity(&n, &opts, &texts, 60_000).as_ref(), 60_000));
    let later = 4000 + IDLE_CLEAR_MS + 1;
    let cleared = build_activity(&n, &opts, &texts, later);
    assert!(cleared.is_none());
    assert!(feed.should_send(cleared.as_ref(), later));
    feed.invalidate();
    assert!(feed.should_send(None, later));
    feed.track_changed();
    assert_eq!(feed.paused_since(), None);
}

#[test]
fn image_types_and_multipart() {
    assert_eq!(sniff_image(&[0xff, 0xd8, 0xff, 0xe0]), Some(("jpg", "image/jpeg")));
    assert_eq!(sniff_image(b"\x89PNG\r\n"), Some(("png", "image/png")));
    assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some(("webp", "image/webp")));
    assert_eq!(sniff_image(b"GIF89a"), Some(("gif", "image/gif")));
    assert_eq!(sniff_image(b"hello"), None);
    let body = multipart("B", &[("reqtype", "fileupload")], "fileToUpload", "png", "image/png", b"DATA");
    let text = String::from_utf8(body).unwrap();
    assert!(text.starts_with("--B\r\nContent-Disposition: form-data; name=\"reqtype\"\r\n\r\nfileupload\r\n"));
    assert!(text.contains("name=\"fileToUpload\"; filename=\"cover.png\"\r\nContent-Type: image/png\r\n\r\nDATA\r\n--B--\r\n"));
}

#[test]
fn cover_cache_keeps_valid_entries() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("discord-covers.json");
    let now = now_ms();
    std::fs::write(
        &f,
        json!({
            "cover:a": {"url": "https://files.catbox.moe/abc.jpg", "at": now, "exp": now + 60_000, "host": "litterbox"},
            "cover:old": {"url": "https://files.catbox.moe/old.jpg", "at": now - covers::COVER_TTL_MS - 1},
            "cover:evil": {"url": "https://evil.example/x.jpg", "at": now},
        })
        .to_string(),
    )
    .unwrap();
    let c = CoverCache::load(Some(f));
    assert_eq!(c.hit("cover:a", now).unwrap().host.as_deref(), Some("litterbox"));
    assert!(c.hit("cover:old", now).is_none());
    assert!(c.hit("cover:evil", now).is_none());
    assert_eq!(CoverRef::VideoThumb(3).key(), "vthumb:3");
    assert_eq!(serde_json::to_value(CoverRef::Cover("h".into())).unwrap(), json!({"kind": "cover", "id": "h"}));
}

#[test]
fn close_frames_are_classified() {
    assert!(matches!(
        classify(OP_CLOSE, Some(&json!({"code": 4000, "message": "Invalid Client ID"}))),
        Incoming::Closed { bad_id: true, .. }
    ));
    assert!(matches!(
        classify(OP_CLOSE, Some(&json!({"code": 1000, "message": "bye"}))),
        Incoming::Closed { bad_id: false, .. }
    ));
    assert!(matches!(
        classify(OP_FRAME, Some(&json!({"cmd": "DISPATCH", "evt": "READY", "data": {"user": {"username": "u"}}}))),
        Incoming::Ready(u) if u == "u"
    ));
    assert!(matches!(classify(OP_FRAME, Some(&json!({"evt": "ERROR", "data": {"code": 4002}}))), Incoming::Error(m) if m == "4002"));
    assert!(valid_client_id(APP_ID));
    assert!(!valid_client_id("123"));
}

/// Un faux Discord sur un socket Unix : poignée de main, READY, puis réponse à chaque commande.
#[cfg(unix)]
#[test]
fn talks_to_a_local_discord() {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;

    let dir = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(dir.path().join("discord-ipc-0")).unwrap();
    let (seen_tx, seen_rx) = crossbeam_channel::unbounded::<(i32, Value)>();
    std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        let mut dec = Decoder::default();
        let mut buf = [0u8; 4096];
        loop {
            let n = match sock.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            for (op, data) in dec.push(&buf[..n]) {
                let data = data.unwrap_or(Value::Null);
                let _ = seen_tx.send((op, data.clone()));
                if op == OP_HANDSHAKE {
                    let ready = json!({"cmd": "DISPATCH", "evt": "READY", "data": {"user": {"username": "tester"}}});
                    sock.write_all(&encode(OP_FRAME, &ready)).unwrap();
                } else if op == OP_FRAME {
                    let reply = json!({"cmd": data["cmd"], "data": {}, "evt": null, "nonce": data["nonce"]});
                    sock.write_all(&encode(OP_FRAME, &reply)).unwrap();
                }
            }
        }
    });

    let statuses = Arc::new(Mutex::new(Vec::new()));
    let seen_status = statuses.clone();
    let emit: Emit = Arc::new(move |e| {
        if let Event::Json(STATUS_CHANNEL, v) = e {
            seen_status.lock().push(v["state"].as_str().unwrap_or_default().to_string());
        }
    });
    let client = DiscordClient::new(DiscordOptions {
        emit,
        cover_file: None,
        cache_file: None,
        ipc_dir: Some(dir.path().to_path_buf()),
    });
    client.configure(true, "");
    let st = client.test();
    assert_eq!(st.state, "connected");
    assert_eq!(st.detail, "tester");
    assert_eq!(st.client_id, std::env::var("CONTROL_DISCORD_ID").unwrap_or_else(|_| APP_ID.into()));

    let recv = || seen_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (op, hs) = recv();
    assert_eq!(op, OP_HANDSHAKE);
    assert_eq!(hs["v"], 1);
    // À la connexion, la présence est d'abord effacée (rien ne joue encore).
    let (op, first) = recv();
    assert_eq!(op, OP_FRAME);
    assert_eq!(first["cmd"], "SET_ACTIVITY");
    assert!(first["args"].get("activity").is_none());
    assert_eq!(first["args"]["pid"], std::process::id());

    let mut t = track(Kind::Audio);
    t.cover = None;
    client.set_activity(build_activity(&np(&t, true), &Opts::default(), &Texts::fr(), now_ms()));
    let (_, frame) = recv();
    assert_eq!(frame["args"]["activity"]["details"], "Le titre");
    assert_eq!(frame["args"]["activity"]["assets"]["small_image"], "play");
    assert_eq!(frame["nonce"], "2");

    client.shutdown();
    let (_, bye) = recv();
    assert_eq!(bye["nonce"], "bye");
    assert!(bye["args"].get("activity").is_none());
    let (op, _) = recv();
    assert_eq!(op, OP_CLOSE);
    assert!(statuses.lock().iter().any(|s| s == "connecting"));
    assert!(statuses.lock().iter().any(|s| s == "connected"));
}

#[test]
fn missing_discord_and_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let client = DiscordClient::new(DiscordOptions {
        emit: crate::model::emit_nothing(),
        cover_file: None,
        cache_file: None,
        ipc_dir: Some(dir.path().to_path_buf()),
    });
    assert_eq!(client.test().state, "off");
    client.configure(true, "");
    assert_eq!(client.test().state, "no-discord");
    client.configure(false, "");
    assert_eq!(client.test().state, "off");
    client.configure(true, "123456789012345678");
    assert_eq!(client.test().client_id, "123456789012345678");
}

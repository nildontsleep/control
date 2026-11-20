use super::*;
use crate::model::emit_nothing;
use md5::{Digest, Md5};
use serde_json::json;

fn setup() -> (tempfile::TempDir, Scrobbler, SharedDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Mutex::new(crate::db::open_in_memory().unwrap()));
    {
        let d = db.lock();
        d.execute(
            "INSERT INTO tracks (id, path, kind, title, artist, album, track_no, duration, added_at) VALUES \
             (1, '/a.mp3', 'audio', 'Titre', 'Artiste', 'Album', 3, 200.4, 0), \
             (2, '/b.mp4', 'video', 'Clip', 'Artiste', '', NULL, 200, 0), \
             (3, '/c.mp3', 'audio', 'Sans artiste', '', '', NULL, 200, 0), \
             (4, '/d.mp3', 'audio', 'Court', 'Artiste', '', NULL, 20, 0)",
            [],
        )
        .unwrap();
    }
    let s = Scrobbler::new(ScrobblerOptions {
        dir: dir.path().to_path_buf(),
        db: db.clone(),
        emit: emit_nothing(),
        keys: None,
        guard: None,
    });
    (dir, s, db)
}

fn connect_both(s: &Scrobbler) {
    let mut st = s.inner.state.lock();
    st.data.listenbrainz =
        Some(ListenBrainzAccount { token: "abcdefgh12".into(), user: Some("lb".into()), url: None, error: None });
    st.data.lastfm = Some(LastfmAccount { session: "sess".into(), user: Some("fm".into()), error: None });
    st.data.lastfm_app = Some(LastfmApp { key: "k".into(), secret: "s".into() });
}

#[test]
fn eligibility_rules() {
    assert!(!counts(29.0, 29.0), "morceau trop court");
    assert!(counts(30.0, 15.0));
    assert!(!counts(30.0, 14.9));
    assert!(counts(200.0, 100.0));
    assert!(!counts(200.0, 99.0));
    // Au-delà de 8 minutes, 4 minutes suffisent.
    assert!(counts(3600.0, 240.0));
    assert!(!counts(3600.0, 239.0));
}

#[test]
fn signature_matches_lastfm_rules() {
    let p = params(&[("method", "auth.getSession".into()), ("token", "tok".into())]);
    let app = LastfmApp { key: "key".into(), secret: "sec".into() };
    let form = lastfm_form(&p, &app, true);
    // md5("api_keykeymethodauth.getSessiontokentok" + "sec")
    let expected = hex::encode(Md5::digest(b"api_keykeymethodauth.getSessiontokentoksec"));
    let sig = form.iter().find(|(k, _)| k == "api_sig").map(|(_, v)| v.clone()).unwrap();
    assert_eq!(sig, expected);
    assert_eq!(form.last().unwrap(), &("format".to_string(), "json".to_string()));
    assert!(form.iter().any(|(k, v)| k == "api_key" && v == "key"));
    let unsigned = lastfm_form(&p, &app, false);
    assert!(!unsigned.iter().any(|(k, _)| k == "api_sig"));
}

#[test]
fn signature_sorts_indexed_params() {
    let q = |at, title: &str, album: &str| Queued {
        at,
        play: Play { title: title.into(), artist: "A".into(), album: album.into(), duration: 180.0, track_no: None },
        todo: vec!["lastfm".into()],
        seq: 0,
    };
    let p = lastfm_scrobble_params(&[q(10, "x", ""), q(20, "y", "Alb")], "sk");
    assert_eq!(p.get("method").unwrap(), "track.scrobble");
    assert_eq!(p.get("timestamp[1]").unwrap(), "20");
    assert_eq!(p.get("album[1]").unwrap(), "Alb");
    assert!(!p.contains_key("album[0]"));
    assert_eq!(p.get("duration[0]").unwrap(), "180");
    let joined: String = p.iter().map(|(k, v)| format!("{k}{v}")).collect();
    assert!(joined.starts_with("album[1]Albartist[0]Aartist[1]A"));
}

#[test]
fn listenbrainz_bodies() {
    let play = Play { title: "T".into(), artist: "A".into(), album: "".into(), duration: 200.4, track_no: Some(2) };
    let now = lb_body(&[(None, &play)]);
    assert_eq!(now["listen_type"], "playing_now");
    assert!(now["payload"][0].get("listened_at").is_none());
    let meta = &now["payload"][0]["track_metadata"];
    assert_eq!(meta["artist_name"], "A");
    assert!(meta.get("release_name").is_none());
    assert_eq!(meta["additional_info"]["duration_ms"], 200400);
    assert_eq!(meta["additional_info"]["tracknumber"], 2);
    assert_eq!(meta["additional_info"]["media_player"], "control");
    assert_eq!(lb_body(&[(Some(5), &play)])["listen_type"], "single");
    let many = lb_body(&[(Some(5), &play), (Some(6), &play)]);
    assert_eq!(many["listen_type"], "import");
    assert_eq!(many["payload"][1]["listened_at"], 6);
}

#[test]
fn listenbrainz_responses() {
    assert!(lb_result(200, None).is_ok());
    assert!(lb_result(401, None).unwrap_err().permanent);
    let limited = lb_result(429, Some("12")).unwrap_err();
    assert!(!limited.permanent);
    assert_eq!(limited.retry_after, Some(12));
    assert_eq!(lb_result(429, None).unwrap_err().retry_after, Some(30));
    assert!(lb_result(400, None).unwrap_err().permanent);
    assert!(!lb_result(503, None).unwrap_err().permanent);
    assert_eq!(lb_validate(200, r#"{"valid":true,"user_name":"bob"}"#).unwrap(), Some("bob".into()));
    assert!(lb_validate(200, r#"{"valid":false}"#).is_err());
    assert!(lb_validate(500, "oops").is_err());
    assert!(valid_lb_token("abcd-EF_12"));
    assert!(!valid_lb_token("short"));
    assert!(!valid_lb_token("avec espace"));
}

#[test]
fn lastfm_responses() {
    assert_eq!(lastfm_result(200, r#"{"token":"t"}"#).unwrap()["token"], "t");
    let bad_session = lastfm_result(403, r#"{"error":9,"message":"Invalid session key"}"#).unwrap_err();
    assert!(bad_session.permanent);
    assert_eq!(bad_session.message, "Invalid session key");
    let busy = lastfm_result(200, r#"{"error":16}"#).unwrap_err();
    assert!(!busy.permanent);
    assert_eq!(busy.message, "Last.fm : erreur 16");
    assert!(!lastfm_result(502, "<html>").unwrap_err().permanent);
    assert_eq!(lastfm_auth_url("a b", "t&"), "https://www.last.fm/api/auth/?api_key=a%20b&token=t%26");
}

#[test]
fn data_file_is_compatible() {
    let text = r#"{"listenbrainz":{"token":"abcdefgh","user":"u","url":null},"lastfm":null,"lastfmApp":{"key":"k","secret":"s"},
        "queue":[{"at":100,"play":{"title":"T","artist":"A","album":"","duration":215,"trackNo":null},"todo":["listenbrainz"]},
                 {"broken":true}]}"#;
    let d = ScrobbleData::parse(text);
    assert_eq!(d.queue.len(), 1);
    assert_eq!(d.listenbrainz.as_ref().unwrap().user.as_deref(), Some("u"));
    assert_eq!(d.services(), vec![Service::ListenBrainz]);
    let out: Value = serde_json::to_value(&d).unwrap();
    assert_eq!(out["queue"][0]["play"]["duration"], json!(215));
    assert!(out["queue"][0]["play"]["duration"].is_i64());
    assert_eq!(out["lastfm"], Value::Null);
    assert_eq!(out["lastfmApp"]["key"], "k");
    assert!(out["listenbrainz"].get("error").is_none());
    assert_eq!(ScrobbleData::parse("pas du json"), ScrobbleData::default());
}

#[test]
fn describe_skips_videos_and_missing_artists() {
    let (_dir, s, _db) = setup();
    let p = s.describe(1).unwrap();
    assert_eq!((p.title.as_str(), p.track_no), ("Titre", Some(3)));
    assert!(s.describe(2).is_none());
    assert!(s.describe(3).is_none());
    assert!(s.describe(99).is_none());
}

#[test]
fn ended_enqueues_only_counted_plays() {
    let (dir, s, _db) = setup();
    // Sans service connecté, rien n'est retenu.
    s.started(1, 1);
    s.ended(1, 150.0);
    assert_eq!(s.status().pending, 0);

    connect_both(&s);
    s.close(); // pas d'envoi réseau pendant le test
    s.started(2, 1);
    s.ended(2, 50.0);
    assert_eq!(s.status().pending, 0, "écoute trop courte");
    s.started(3, 1);
    s.ended(3, 101.0);
    s.started(4, 4);
    s.ended(4, 20.0);
    let d = s.data();
    assert_eq!(d.queue.len(), 1);
    assert_eq!(d.queue[0].todo, vec!["listenbrainz".to_string(), "lastfm".to_string()]);
    assert!((d.queue[0].play.duration - 200.0).abs() < 1e-9);
    // Une fin inconnue ou répétée n'ajoute rien.
    s.ended(3, 200.0);
    assert_eq!(s.status().pending, 1);
    // La file est sur le disque.
    let saved = ScrobbleData::parse(&std::fs::read_to_string(dir.path().join("scrobble.json")).unwrap());
    assert_eq!(saved.queue.len(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("scrobble.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn flush_keeps_temporary_failures_and_drops_permanent_ones() {
    let (_dir, s, _db) = setup();
    connect_both(&s);
    s.close();
    let now = now_ms() / 1000;
    let play = s.describe(1).unwrap();
    for i in 0..3 {
        s.enqueue(&play, now - i);
    }
    // Une écoute trop vieille pour Last.fm disparaît.
    s.enqueue(&play, now - MAX_AGE_S - 10);
    assert_eq!(s.status().pending, 4);

    // ListenBrainz répond, Last.fm est indisponible : les écoutes restent pour Last.fm seulement.
    s.flush_with(&|items, creds| match creds {
        Creds::ListenBrainz { token, .. } => {
            assert_eq!(token, "abcdefgh12");
            assert_eq!(items.len(), 3);
            Ok(())
        }
        Creds::Lastfm { app, .. } => {
            assert_eq!(app.as_ref().unwrap().key, "k");
            Err(SendError::temporary("Last.fm : 503"))
        }
    });
    let d = s.data();
    assert_eq!(d.queue.len(), 3);
    assert!(d.queue.iter().all(|q| q.todo == vec!["lastfm".to_string()]));

    // Session refusée : les écoutes sont abandonnées et l'erreur est affichée.
    s.flush_with(&|_, _| Err(SendError::permanent("Invalid session key")));
    let st = s.status();
    assert_eq!(st.pending, 0);
    assert_eq!(st.lastfm.error.as_deref(), Some("Invalid session key"));
    assert!(st.lastfm.connected);

    // Un envoi réussi efface l'erreur.
    s.enqueue(&play, now);
    s.flush_with(&|_, _| Ok(()));
    assert_eq!(s.status().lastfm.error, None);
    assert_eq!(s.status().pending, 0);
}

#[test]
fn flush_sends_batches_of_fifty() {
    let (_dir, s, _db) = setup();
    {
        let mut st = s.inner.state.lock();
        st.data.listenbrainz =
            Some(ListenBrainzAccount { token: "abcdefgh12".into(), user: None, url: None, error: None });
    }
    s.close();
    let now = now_ms() / 1000;
    let play = s.describe(1).unwrap();
    for i in 0..120 {
        s.enqueue(&play, now - i);
    }
    let sizes = Mutex::new(Vec::new());
    s.flush_with(&|items, _| {
        sizes.lock().push(items.len());
        if sizes.lock().len() == 2 { Err(SendError::temporary("coupure")) } else { Ok(()) }
    });
    assert_eq!(*sizes.lock(), vec![50, 50]);
    assert_eq!(s.status().pending, 70);
}

#[test]
fn disconnect_drops_pending_for_that_service() {
    let (_dir, s, _db) = setup();
    connect_both(&s);
    s.close();
    let play = s.describe(1).unwrap();
    s.enqueue(&play, now_ms() / 1000);
    let st = s.disconnect("lastfm");
    assert!(!st.lastfm.connected);
    assert_eq!(st.pending, 1);
    assert_eq!(s.data().queue[0].todo, vec!["listenbrainz".to_string()]);
    let st = s.disconnect("listenbrainz");
    assert_eq!(st.pending, 0);
}

#[test]
fn keys_from_server_take_precedence() {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Mutex::new(crate::db::open_in_memory().unwrap()));
    let s = Scrobbler::new(ScrobblerOptions {
        dir: dir.path().to_path_buf(),
        db,
        emit: emit_nothing(),
        keys: Some(Arc::new(|| Some(LastfmApp { key: "srv".into(), secret: "x".into() }))),
        guard: None,
    });
    s.close();
    let st = s.set_lastfm_keys(" mine ", " sec ");
    assert!(st.lastfm.configured);
    assert!(st.lastfm.keys_from_server);
    assert_eq!(s.keys().unwrap().key, "srv");
    assert_eq!(s.data().lastfm_app.unwrap().key, "mine");
    assert!(s.set_lastfm_keys("", "x").lastfm.configured, "les clés du serveur restent");
    assert_eq!(s.data().lastfm_app, None);
}

#[test]
fn status_json_shape() {
    let (_dir, s, _db) = setup();
    s.close();
    let v = serde_json::to_value(s.status()).unwrap();
    assert_eq!(
        v,
        json!({
            "listenbrainz": {"connected": false, "user": null, "url": null, "error": null},
            "lastfm": {"connected": false, "user": null, "error": null, "configured": false, "keysFromServer": false},
            "pending": 0
        })
    );
}

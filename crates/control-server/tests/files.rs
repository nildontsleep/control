//! Envois de musique, cookies.txt du compte, liens de téléchargement, scrobbling côté serveur
//! (port de `upload.test.js`, `cookies.test.js`, `urlguard.test.js`, et de la partie serveur de `scrobble.test.js`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Boot, Client, PASSWORD, boot, wav};
use serde_json::json;

fn wav_bytes(seconds: u32) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.wav");
    wav(&f, seconds);
    std::fs::read(f).unwrap()
}

fn upload(
    c: &Client,
    name: &str,
    dir: Option<&str>,
    body: Vec<u8>,
    extra: &[(&str, &str)],
    csrf: bool,
) -> common::Resp {
    let mut url = format!("/api/upload?name={}", common::urlencode(name));
    if let Some(d) = dir {
        url.push_str(&format!("&dir={}", common::urlencode(d)));
    }
    c.raw("POST", &url, Some((body, "application/octet-stream")), extra, csrf)
}

#[test]
fn upload_lands_in_the_account_folder_and_library() {
    let s = boot(Boot { no_common: true, ..Boot::default() });
    let admin = s.client();
    admin.setup(json!({ "username": "nil", "password": PASSWORD }));
    let bytes = wav_bytes(2);

    assert_eq!(upload(&s.client(), "x.wav", None, bytes.clone(), &[], true).status, 401, "connexion requise");
    assert_eq!(upload(&admin, "x.wav", None, bytes.clone(), &[], false).status, 403, "en-tête anti-CSRF requis");

    let res = upload(&admin, "Mon titre: essai.wav", Some("Album/../Disque 1"), bytes.clone(), &[], true);
    assert_eq!(res.status, 200, "{}", res.text());
    let body = res.json();
    assert_eq!(body["result"]["name"], "Mon titre_ essai.wav");
    assert_eq!(body["result"]["trackIds"].as_array().unwrap().len(), 1);
    let user = s.hub().accounts.by_name("nil").unwrap();
    let envois = s.hub().download_dir_for(&user).join("Envois");
    let expected = envois.join("Album").join("Disque 1").join("Mon titre_ essai.wav");
    assert!(expected.exists(), "« .. » est écarté, pas suivi : {}", expected.display());
    let partial = std::fs::read_dir(expected.parent().unwrap())
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().ends_with(".part"));
    assert!(!partial, "pas de fichier partiel");
    let tracks = admin.wait_tracks(1);
    assert_eq!(tracks[0]["path"], json!(expected));

    // Le même nom ne remplace rien.
    let again = upload(&admin, "Mon titre: essai.wav", Some("Album/../Disque 1"), bytes.clone(), &[], true).json();
    assert_eq!(again["result"]["name"], "Mon titre_ essai (2).wav");
    assert_eq!(admin.tracks().len(), 2);

    // Refus : autre type de fichier, contenu qui n'est pas un média, fichier vide, trop gros.
    assert_eq!(upload(&admin, "script.sh", None, b"echo hi".to_vec(), &[], true).status, 415);
    assert_eq!(
        upload(&admin, "faux.mp3", None, "ceci n’est pas de la musique".as_bytes().to_vec(), &[], true).status,
        415
    );
    assert!(!envois.join("faux.mp3").exists(), "le faux fichier est supprimé");
    assert_eq!(upload(&admin, "vide.wav", None, Vec::new(), &[], true).status, 400);
    admin.rpc("srv:settings", vec![json!({ "uploadMaxMb": 1 })]).unwrap();
    assert_eq!(upload(&admin, "gros.wav", None, vec![0u8; 2 * 1048576], &[], true).status, 413);
    assert!(!std::fs::read_dir(&envois).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("gros")));
}

#[test]
fn upload_is_admin_only_until_opened_to_everyone() {
    let s = boot(Boot { no_common: true, ..Boot::default() });
    let admin = s.client();
    admin.setup(json!({ "username": "nil", "password": PASSWORD }));
    admin.rpc("srv:users:create", vec![json!({ "username": "ami", "password": PASSWORD })]).unwrap();
    let ami = s.client();
    ami.login("ami", PASSWORD);
    let bytes = wav_bytes(1);
    assert_eq!(ami.boot_state("/")["caps"]["upload"], false);
    assert_eq!(upload(&ami, "a.wav", None, bytes.clone(), &[], true).status, 403);
    admin.rpc("srv:settings", vec![json!({ "allowUserUploads": true })]).unwrap();
    let caps = ami.boot_state("/")["caps"].clone();
    assert_eq!((&caps["upload"], &caps["uploadMaxMb"]), (&json!(true), &json!(500)));
    assert_eq!(upload(&ami, "a.wav", None, bytes, &[], true).status, 200);
    // Chacun dans son propre dossier.
    ami.wait_tracks(1);
    assert_eq!(admin.tracks().len(), 0);
}

fn row(domain: &str, name: &str, value: &str) -> String {
    [domain, "TRUE", "/", "TRUE", "1893456000", name, value].join("\t")
}

#[test]
fn cookies_are_per_account_kept_on_the_server_and_never_returned() {
    let s = boot(Boot { no_common: true, ..Boot::default() });
    let admin = s.client();
    admin.setup(json!({ "username": "nil", "password": PASSWORD, "libraryFolders": [s.music] }));
    admin.rpc("srv:users:create", vec![json!({ "username": "ami", "password": PASSWORD })]).unwrap();
    let ami = s.client();
    ami.login("ami", PASSWORD);
    let sample = [
        "# Netscape HTTP Cookie File".to_string(),
        "# https://curl.haxx.se/rfc/cookie_spec.html".into(),
        String::new(),
        row(".youtube.com", "SID", "abc"),
        format!("#HttpOnly_{}", row(".google.com", "__Secure-1PSID", "def")),
        row("www.youtube.com", "PREF", "f6=40000000"),
    ]
    .join("\n");

    assert_eq!(admin.rpc("dl:cookies-status", vec![]).unwrap(), json!({ "present": false }));
    assert_eq!(admin.rpc("dl:set-cookies", vec![json!("pas des cookies")]).unwrap_err().status, 400);
    let status = admin.rpc("dl:set-cookies", vec![json!(sample)]).unwrap();
    assert_eq!((&status["present"], &status["count"]), (&json!(true), &json!(3)));
    assert!(status["domains"].as_array().unwrap().contains(&json!("youtube.com")));
    assert!(!status.to_string().contains("abc"), "le contenu ne revient jamais au navigateur");

    let id = admin.me()["id"].as_str().unwrap().to_string();
    let file = s.root.join("data").join("users").join(&id).join("cookies.txt");
    assert!(file.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600,
            "lisible par le serveur seul"
        );
    }
    assert!(!std::fs::read_to_string(&file).unwrap().contains('\r'));

    // Les téléchargements sont réservés à l'administrateur tant qu'il ne les ouvre pas : les cookies suivent.
    assert_eq!(ami.rpc("dl:set-cookies", vec![json!(sample)]).unwrap_err().status, 403);
    assert_eq!(ami.rpc("dl:cookies-status", vec![]).unwrap_err().status, 403);
    admin.rpc("srv:settings", vec![json!({ "allowUserDownloads": true })]).unwrap();
    assert_eq!(
        ami.rpc("dl:cookies-status", vec![]).unwrap(),
        json!({ "present": false }),
        "le fichier de l'administrateur n'est pas le sien"
    );
    ami.rpc("dl:set-cookies", vec![json!(row(".youtube.com", "SID", "autre"))]).unwrap();
    assert_eq!(ami.rpc("dl:cookies-status", vec![]).unwrap()["count"], 1);
    assert_eq!(admin.rpc("dl:cookies-status", vec![]).unwrap()["count"], 3);

    admin.rpc("dl:clear-cookies", vec![]).unwrap();
    assert!(!file.exists());
    assert_eq!(admin.rpc("dl:cookies-status", vec![]).unwrap(), json!({ "present": false }));
    assert_eq!(ami.rpc("dl:cookies-status", vec![]).unwrap()["count"], 1, "celui de l'autre compte reste");

    // Ces fichiers ne passent pas par les réglages : un chemin envoyé par le navigateur est ignoré.
    admin.send("settings:set", vec![json!({ "dl": { "cookiesFile": "/etc/passwd" } })]);
    assert!(admin.boot_state("/")["settings"]["dl"]["cookiesFile"].as_str().is_none_or(str::is_empty));
}

#[test]
fn download_links_are_checked_before_yt_dlp() {
    let s = boot(Boot::default());
    let admin = s.client();
    admin.setup(json!({ "username": "nil", "password": PASSWORD, "libraryFolders": [s.music] }));
    let info = admin.rpc("dl:info", vec![json!("http://127.0.0.1:5180/api/health")]).unwrap();
    assert_eq!(info["ok"], false);
    assert!(info["error"].as_str().unwrap().contains("réseau du serveur"));
    let err = admin.rpc("dl:add", vec![json!({ "url": "file:///etc/passwd", "title": "x" })]).unwrap_err();
    assert!(err.message.contains("http et https"), "{}", err.message);
    let err = admin.rpc("dl:add", vec![json!("pas un objet")]).unwrap_err();
    assert!(err.status == 400 && err.message.contains("invalide"));
    assert_eq!(admin.rpc("dl:list", vec![]).unwrap(), json!([]), "rien n'est mis en file");
}

#[test]
fn scrobbling_status_custom_address_and_server_keys() {
    let s = boot(Boot { no_common: true, ..Boot::default() });
    let admin = s.client();
    admin.setup(json!({ "username": "nil", "password": PASSWORD, "libraryFolders": [s.music] }));
    admin.wait_tracks(3);
    let status = admin.rpc("scrobble:status", vec![]).unwrap();
    assert_eq!(status["listenbrainz"]["connected"], false);
    assert_eq!((&status["lastfm"]["configured"], &status["pending"]), (&json!(false), &json!(0)));
    // Adresse personnalisée : publique seulement (rien ne part sur le réseau).
    let bad = admin
        .rpc("scrobble:connect-listenbrainz", vec![json!("abcdef12345-token"), json!("http://127.0.0.1:9/")])
        .unwrap();
    assert_eq!(bad["ok"], false);
    assert!(bad["message"].as_str().unwrap().contains("réseau du serveur"), "{bad}");
    assert_eq!(admin.rpc("scrobble:lastfm-start", vec![]).unwrap()["ok"], false, "sans clés d'API : refusé");

    // L'administrateur règle l'application Last.fm : disponible pour les comptes, sans que le secret revienne.
    admin.rpc("srv:settings", vec![json!({ "lastfmKey": "APIKEY", "lastfmSecret": "SECRET" })]).unwrap();
    let shown = admin.rpc("srv:settings:get", vec![]).unwrap();
    assert_eq!(
        (&shown["lastfmKey"], shown.get("lastfmSecret"), &shown["hasLastfmSecret"]),
        (&json!("APIKEY"), None, &json!(true))
    );
    assert_eq!(admin.rpc("scrobble:status", vec![]).unwrap()["lastfm"]["configured"], true);
    assert!(!admin.json("GET", "/", None).text().contains("SECRET"));
}

#[test]
fn media_and_watch_channels_answer_without_network() {
    let s = boot(Boot::default());
    let admin = s.client();
    admin.setup(json!({ "username": "nil", "password": PASSWORD }));
    assert_eq!(admin.rpc("watchlist:list", vec![]).unwrap()["ok"], true);
    assert_eq!(admin.rpc("watch:history", vec![]).unwrap()["data"], json!([]));
    let progress = admin.rpc("loudness:progress", vec![]).unwrap();
    assert_eq!(progress["enabled"], false);
    let levels = admin.rpc("loudness:levels", vec![json!([99999])]).unwrap();
    assert_eq!(levels["99999"], serde_json::Value::Null);
    assert!(admin.rpc("tools:status", vec![]).unwrap()["ytdlp"].is_object());
    assert_eq!(admin.rpc("tags:read", vec![json!([99999])]).unwrap()["data"][0]["error"]["code"], "missing");
    let refused = admin.rpc("tags:stage-cover", vec![json!({ "url": "https://evil.example/x.jpg" })]).unwrap();
    assert_eq!(refused["message"], "Source de pochette refusée.");
}

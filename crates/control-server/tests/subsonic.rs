//! API compatible Subsonic / OpenSubsonic : authentification, parcours, recherche, flux, pochettes, playlists,
//! favoris, écoutes (port de `server/test/subsonic.test.js`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Boot, Client, PASSWORD, Server, boot, urlencode};
use md5::{Digest as _, Md5};
use serde_json::{Value, json};

fn md5(s: &str) -> String {
    hex::encode(Md5::digest(s.as_bytes()))
}

/// Un client Subsonic : signe ses requêtes avec le jeton md5(secret + sel), comme les applications du commerce.
struct Sub {
    client: Client,
    user: String,
    secret: String,
}

impl Sub {
    fn url(&self, method: &str, params: &[(&str, String)]) -> String {
        let salt = hex::encode(rand::random::<[u8; 5]>());
        let mut q = vec![
            ("u".to_string(), self.user.clone()),
            ("t".to_string(), md5(&format!("{}{salt}", self.secret))),
            ("s".to_string(), salt),
            ("v".to_string(), "1.16.1".to_string()),
            ("c".to_string(), "essai".to_string()),
        ];
        q.extend(params.iter().map(|(k, v)| (k.to_string(), v.clone())));
        let query: Vec<String> = q.iter().map(|(k, v)| format!("{k}={}", urlencode(v))).collect();
        format!("/rest/{method}.view?{}", query.join("&"))
    }

    fn raw(&self, method: &str, params: &[(&str, String)], extra: &[(&str, &str)]) -> common::Resp {
        self.client.raw("GET", &self.url(method, params), None, extra, false)
    }

    fn call(&self, method: &str, params: &[(&str, String)]) -> Value {
        let mut p = vec![("f", "json".to_string())];
        p.extend(params.iter().cloned());
        self.raw(method, &p, &[]).json()["subsonic-response"].clone()
    }
}

fn s(v: impl ToString) -> String {
    v.to_string()
}

fn get(c: &Client, query: &str) -> Value {
    c.raw("GET", &format!("/rest/{query}"), None, &[], false).json()["subsonic-response"].clone()
}

struct House {
    s: Server,
    nil: Client,
    sorted: Vec<Value>,
    secret: String,
    sub: Sub,
    cover: String,
}

fn house() -> House {
    let s = boot(Boot { no_common: true, ..Boot::default() });
    let nil = s.client();
    nil.setup(json!({ "username": "nil", "password": PASSWORD, "libraryFolders": [s.music] }));
    let mut sorted = nil.wait_tracks(3);
    sorted.sort_by_key(|t| t["id"].as_i64());
    let hub = s.hub();
    let entry = hub.entry(&hub.accounts.by_name("nil").unwrap()).unwrap();
    // Les fichiers d'essai n'ont pas de tags : on leur en donne.
    let cover = "a".repeat(20);
    std::fs::create_dir_all(&hub.covers_dir).unwrap();
    std::fs::write(hub.covers_dir.join(format!("{cover}.jpg")), [0xFFu8, 0xD8, 0xFF, 0xE0, 0, 0x10]).unwrap();
    let meta = [
        ("Lune", "Aya", "Nuits", 2023, 1, "Pop"),
        ("Soleil", "Aya", "Nuits", 2023, 2, "Pop"),
        ("Perihelie", "Kiro", "Orbit", 2022, 1, "Electro"),
    ];
    {
        let db = entry.core.db.lock();
        for (i, (t, (title, artist, album, year, no, genre))) in sorted.iter().zip(meta).enumerate() {
            let c = if i < 2 { Some(cover.clone()) } else { None };
            db.execute(
                "UPDATE tracks SET title = ?, artist = ?, album = ?, year = ?, track_no = ?, genre = ?, cover = ? WHERE id = ?",
                rusqlite::params![title, artist, album, year, no, genre, c, t["id"].as_i64()],
            )
            .unwrap();
        }
    }
    entry.sub_model.lock().take();
    let secret =
        nil.rpc("srv:me:subsonic-reveal", vec![json!(PASSWORD)]).unwrap()["secret"].as_str().unwrap().to_string();
    let sub = Sub { client: s.client(), user: "nil".into(), secret: secret.clone() };
    House { s, nil, sorted, secret, sub, cover }
}

#[test]
fn authentication_app_password_token_api_key_refusals_and_throttle() {
    let h = house();
    let c = h.s.client();
    let wrong = Sub { client: h.s.client(), user: "nil".into(), secret: "pas-le-bon".into() };
    assert_eq!(wrong.call("ping", &[])["error"]["code"], 40);

    // Le mot de passe d'application ne se révèle qu'avec le mot de passe du compte.
    assert_eq!(h.nil.rpc("srv:me:subsonic-reveal", vec![json!("faux")]).unwrap_err().status, 403);
    assert_eq!(h.nil.rpc("srv:me:subsonic", vec![]).unwrap()["hasSecret"], true);

    let ok = h.sub.call("ping", &[]);
    assert_eq!(
        (&ok["status"], &ok["version"], &ok["type"], &ok["openSubsonic"]),
        (&json!("ok"), &json!("1.16.1"), &json!("control"), &json!(true))
    );
    let secret = &h.secret;
    assert_eq!(get(&c, &format!("ping?u=nil&p={secret}&v=1.16.1&c=x&f=json"))["status"], "ok", "mot de passe en clair");
    let hex_secret = hex::encode(secret);
    assert_eq!(
        get(&c, &format!("ping?u=nil&p=enc:{hex_secret}&v=1.16.1&c=x&f=json"))["status"],
        "ok",
        "en hexadécimal"
    );
    let account = urlencode(PASSWORD);
    assert_eq!(
        get(&c, &format!("ping?u=nil&p={account}&v=1.16.1&c=x&f=json"))["error"]["code"],
        40,
        "le mot de passe du compte n'ouvre pas l'API"
    );
    let fake = md5("fauxsel");
    assert_eq!(get(&c, &format!("ping?u=nil&t={fake}&s=sel&v=1.16.1&c=x&f=json"))["error"]["code"], 40);

    // Clé d'API (OpenSubsonic) : même secret, sans nom d'utilisateur ; les deux ensemble sont refusés.
    assert_eq!(get(&c, &format!("ping?apiKey={secret}&c=x&f=json"))["status"], "ok");
    assert_eq!(get(&c, &format!("tokenInfo?apiKey={secret}&c=x&f=json"))["tokenInfo"]["username"], "nil");
    assert_eq!(get(&c, &format!("ping?apiKey={secret}&u=nil&c=x&f=json"))["error"]["code"], 43);
    assert_eq!(get(&c, "ping?apiKey=mauvaise&c=x&f=json")["error"]["code"], 44);
    // Pas de cookie de session : l'API ne s'ouvre pas avec la connexion du navigateur.
    assert_eq!(
        h.nil.raw("GET", "/rest/ping?c=x&f=json", None, &[], false).json()["subsonic-response"]["error"]["code"],
        10
    );

    // XML par défaut, paramètre c obligatoire hors ping, méthode inconnue.
    let xml = h.sub.raw("getLicense", &[], &[]).text();
    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<subsonic-response xmlns=\"http://subsonic.org/restapi\" status=\"ok\" version=\"1.16.1\" type=\"control\""), "{xml}");
    assert!(xml.contains("<license valid=\"true\""));
    assert_eq!(get(&c, &format!("getArtists?u=nil&p={secret}&f=json"))["error"]["code"], 10);
    assert_eq!(h.sub.call("nimporte", &[])["error"]["code"], 0);
    let ext: Vec<Value> = h.sub.call("getOpenSubsonicExtensions", &[])["openSubsonicExtensions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].clone())
        .collect();
    assert_eq!(ext, [json!("apiKeyAuthentication"), json!("formPost"), json!("songLyrics")]);

    // Révocation puis renouvellement : l'ancien secret ne marche plus.
    let renewed =
        h.nil.rpc("srv:me:subsonic-renew", vec![json!(PASSWORD)]).unwrap()["secret"].as_str().unwrap().to_string();
    assert_ne!(&renewed, secret);
    assert_eq!(h.sub.call("ping", &[])["error"]["code"], 40);
    let fresh = Sub { client: h.s.client(), user: "nil".into(), secret: renewed };
    assert_eq!(fresh.call("ping", &[])["status"], "ok");
    h.nil.rpc("srv:me:subsonic-revoke", vec![]).unwrap();
    assert_eq!(fresh.call("ping", &[])["error"]["code"], 40);

    // L'administrateur peut couper l'API ; trop d'essais ratés sont limités.
    let again =
        h.nil.rpc("srv:me:subsonic-reveal", vec![json!(PASSWORD)]).unwrap()["secret"].as_str().unwrap().to_string();
    let sub = Sub { client: h.s.client(), user: "nil".into(), secret: again };
    h.nil.rpc("srv:settings", vec![json!({ "subsonicEnabled": false })]).unwrap();
    assert_eq!(sub.call("ping", &[])["error"]["code"], 50);
    h.nil.rpc("srv:settings", vec![json!({ "subsonicEnabled": true })]).unwrap();
    for _ in 0..30 {
        get(&c, "ping?u=nil&p=non&c=x&f=json");
    }
    assert!(sub.call("ping", &[])["error"]["message"].as_str().unwrap().contains("Trop d’essais"));
}

#[test]
fn browsing_artists_albums_songs_genres_lists_search() {
    let h = house();
    let sub = &h.sub;
    assert_eq!(
        sub.call("getMusicFolders", &[])["musicFolders"]["musicFolder"],
        json!([{ "id": 1, "name": "Bibliothèque" }])
    );
    let artists = sub.call("getArtists", &[])["artists"].clone();
    let letters: Vec<&str> = artists["index"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert_eq!(letters, ["A", "K"]);
    let aya = artists["index"][0]["artist"][0].clone();
    assert_eq!((&aya["name"], &aya["albumCount"]), (&json!("Aya"), &json!(1)));
    assert!(aya["id"].as_str().unwrap().starts_with("ar-"));
    assert_eq!(sub.call("getIndexes", &[])["indexes"]["index"][0]["artist"][0]["name"], "Aya");

    let artist = sub.call("getArtist", &[("id", s(aya["id"].as_str().unwrap()))])["artist"].clone();
    assert_eq!((&artist["name"], artist["album"].as_array().unwrap().len()), (&json!("Aya"), 1));
    assert_eq!(
        (&artist["album"][0]["name"], &artist["album"][0]["songCount"], &artist["album"][0]["year"]),
        (&json!("Nuits"), &json!(2), &json!(2023))
    );
    let album = sub.call("getAlbum", &[("id", s(artist["album"][0]["id"].as_str().unwrap()))])["album"].clone();
    let titles: Vec<&str> = album["song"].as_array().unwrap().iter().map(|x| x["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["Lune", "Soleil"], "dans l'ordre des pistes");
    let song = album["song"][0].clone();
    assert_eq!(song["id"], json!(h.sorted[0]["id"].to_string()));
    assert_eq!(
        (&song["album"], &song["artist"], &song["track"], &song["genre"]),
        (&json!("Nuits"), &json!("Aya"), &json!(1), &json!("Pop"))
    );
    assert_eq!(
        (&song["suffix"], &song["contentType"], &song["type"], &song["isDir"]),
        (&json!("wav"), &json!("audio/wav"), &json!("music"), &json!(false))
    );
    assert!(song["coverArt"].as_str().unwrap().starts_with("cv-") && song["duration"].as_i64().unwrap() >= 1);
    assert!(song["path"].as_str().unwrap().starts_with("Aya/Nuits/"));
    assert_eq!(sub.call("getSong", &[("id", s(song["id"].as_str().unwrap()))])["song"]["title"], "Lune");
    assert_eq!(sub.call("getSong", &[("id", s(99999))])["error"]["code"], 70);
    assert_eq!(sub.call("getAlbum", &[("id", s("al-inconnu"))])["error"]["code"], 70);

    // Navigation par dossiers : artiste → albums → morceaux.
    let dir = sub.call("getMusicDirectory", &[("id", s(aya["id"].as_str().unwrap()))])["directory"].clone();
    assert_eq!(
        (&dir["name"], &dir["child"][0]["isDir"], &dir["child"][0]["title"]),
        (&json!("Aya"), &json!(true), &json!("Nuits"))
    );
    let child = sub.call("getMusicDirectory", &[("id", s(dir["child"][0]["id"].as_str().unwrap()))]);
    assert_eq!(child["directory"]["child"].as_array().unwrap().len(), 2);

    let genres: Vec<Value> = sub.call("getGenres", &[])["genres"]["genre"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| json!([g["value"], g["songCount"], g["albumCount"]]))
        .collect();
    assert_eq!(genres, [json!(["Electro", 1, 1]), json!(["Pop", 2, 1])]);
    let names = |v: &Value, key: &str| -> Vec<String> {
        v[key]["album"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["name"].as_str().unwrap_or(a["title"].as_str().unwrap_or("")).to_string())
            .collect()
    };
    assert_eq!(
        names(&sub.call("getAlbumList2", &[("type", s("alphabeticalByName"))]), "albumList2"),
        ["Nuits", "Orbit"]
    );
    assert_eq!(
        names(
            &sub.call("getAlbumList2", &[("type", s("byYear")), ("fromYear", s(2022)), ("toYear", s(2022))]),
            "albumList2"
        ),
        ["Orbit"]
    );
    assert_eq!(
        names(&sub.call("getAlbumList2", &[("type", s("byGenre")), ("genre", s("pop"))]), "albumList2"),
        ["Nuits"]
    );
    assert_eq!(
        sub.call("getAlbumList2", &[("type", s("random")), ("size", s(1))])["albumList2"]["album"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(sub.call("getAlbumList", &[("type", s("newest"))])["albumList"]["album"][0]["isDir"], true);
    assert_eq!(sub.call("getRandomSongs", &[("size", s(5))])["randomSongs"]["song"].as_array().unwrap().len(), 3);
    let by_genre = sub.call("getSongsByGenre", &[("genre", s("Electro"))]);
    assert_eq!(by_genre["songsByGenre"]["song"][0]["title"], "Perihelie");

    let found = sub.call("search3", &[("query", s("lun")), ("songCount", s(5))])["searchResult3"].clone();
    assert_eq!((found["song"].as_array().unwrap().len(), &found["song"][0]["title"]), (1, &json!("Lune")));
    assert_eq!((found["artist"].as_array().unwrap().len(), found["album"].as_array().unwrap().len()), (0, 0));
    assert_eq!(sub.call("search3", &[("query", s("aya"))])["searchResult3"]["artist"][0]["name"], "Aya");
    assert_eq!(sub.call("search2", &[("query", s("orbit"))])["searchResult2"]["album"][0]["title"], "Orbit");
    assert_eq!(
        sub.call("search3", &[("query", s(""))])["searchResult3"]["song"].as_array().unwrap().len(),
        3,
        "requête vide : tout"
    );
    assert!(sub.call("getArtistInfo2", &[("id", s(aya["id"].as_str().unwrap()))]).get("artistInfo2").is_some());
    assert_eq!(sub.call("getScanStatus", &[])["scanStatus"]["count"], 3);
    assert_eq!(sub.call("getUser", &[("username", s("nil"))])["user"]["streamRole"], true);
    assert_eq!(sub.call("jukeboxControl", &[])["error"]["code"], 50);
}

#[test]
fn media_stream_with_range_covers_and_library_only_files() {
    let h = house();
    let id = s(h.sorted[0]["id"].as_i64().unwrap());
    let full = h.sub.raw("stream", &[("id", id.clone())], &[]);
    assert_eq!((full.status, full.header("content-type")), (200, Some("audio/wav")));
    let part = h.sub.raw("stream", &[("id", id.clone())], &[("range", "bytes=0-99")]);
    assert_eq!((part.status, part.body.len()), (206, 100));
    assert_eq!(part.header("content-range"), Some(format!("bytes 0-99/{}", full.body.len()).as_str()));
    let dl = h.sub.raw("download", &[("id", id.clone())], &[]);
    let disposition = dl.header("content-disposition").unwrap();
    assert!(disposition.starts_with("attachment; filename=\"") && disposition.ends_with(".wav\""));
    // Un identifiant qui n'est pas de cette bibliothèque : erreur Subsonic (HTTP 200, code 70), jamais un fichier.
    let missing = h.sub.raw("stream", &[("id", s(424242)), ("f", s("json"))], &[]);
    assert_eq!((missing.status, &missing.json()["subsonic-response"]["error"]["code"]), (200, &json!(70)));

    let art = h.sub.raw("getCoverArt", &[("id", format!("cv-{}", h.cover)), ("size", s(100))], &[]);
    assert_eq!((art.status, art.header("content-type")), (200, Some("image/jpeg")));
    assert_eq!(h.sub.raw("getCoverArt", &[("id", id)], &[]).status, 200, "par morceau");
    let escape = h.sub.raw("getCoverArt", &[("id", s("cv-../../etc/passwd")), ("f", s("json"))], &[]);
    assert_eq!(escape.json()["subsonic-response"]["error"]["code"], 70);
    let none = h.sub.raw("getCoverArt", &[("id", s(h.sorted[2]["id"].as_i64().unwrap())), ("f", s("json"))], &[]);
    assert_eq!(none.json()["subsonic-response"]["error"]["code"], 70, "sans pochette");

    // En POST (formPost) aussi.
    let url = h.sub.url("ping", &[("f", s("json"))]);
    let query = url.split_once('?').unwrap().1.to_string();
    let posted = h.s.client().raw(
        "POST",
        "/rest/ping",
        Some((query.into_bytes(), "application/x-www-form-urlencoded")),
        &[],
        false,
    );
    assert_eq!(posted.json()["subsonic-response"]["status"], "ok");
}

#[test]
fn playlists_favorites_plays_and_lyrics() {
    let h = house();
    let (sub, ids) = (&h.sub, &h.sorted);
    let id = |i: usize| s(ids[i]["id"].as_i64().unwrap());
    let created =
        sub.call("createPlaylist", &[("name", s("Route")), ("songId", id(0)), ("songId", id(2))])["playlist"].clone();
    let entries: Vec<&str> =
        created["entry"].as_array().unwrap().iter().map(|x| x["title"].as_str().unwrap()).collect();
    assert_eq!(
        (&created["name"], &created["songCount"], entries),
        (&json!("Route"), &json!(2), vec!["Lune", "Perihelie"])
    );
    let pid = created["id"].as_str().unwrap().to_string();
    assert!(pid.starts_with("pl-"));
    let listed = sub.call("getPlaylists", &[])["playlists"]["playlist"].clone();
    assert_eq!(
        (&listed[0]["name"], &listed[0]["owner"], &listed[0]["songCount"]),
        (&json!("Route"), &json!("nil"), &json!(2))
    );
    sub.call(
        "updatePlaylist",
        &[
            ("playlistId", pid.clone()),
            ("name", s("Route 66")),
            ("comment", s("Pour rouler")),
            ("songIdToAdd", id(1)),
            ("songIndexToRemove", s(0)),
        ],
    );
    let updated = sub.call("getPlaylist", &[("id", pid.clone())])["playlist"].clone();
    let titles: Vec<&str> = updated["entry"].as_array().unwrap().iter().map(|x| x["title"].as_str().unwrap()).collect();
    assert_eq!(
        (&updated["name"], &updated["comment"], titles),
        (&json!("Route 66"), &json!("Pour rouler"), vec!["Perihelie", "Soleil"])
    );
    assert_eq!(
        h.nil.rpc("lib:playlists", vec![]).unwrap()["playlists"][0]["name"],
        "Route 66",
        "c'est aussi celle de control"
    );
    assert_eq!(sub.call("deletePlaylist", &[("id", pid.clone())])["status"], "ok");
    assert_eq!(sub.call("getPlaylist", &[("id", pid)])["error"]["code"], 70);

    // Favoris : star / unstar passent par la playlist « Favoris ».
    assert_eq!(sub.call("getStarred2", &[])["starred2"]["song"], json!([]));
    sub.call("star", &[("id", id(0)), ("id", id(1))]);
    sub.call("star", &[("id", id(0))]); // déjà étoilé : pas de doublon
    let starred: Vec<Value> = sub.call("getStarred2", &[])["starred2"]["song"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["title"].clone())
        .collect();
    assert_eq!(starred, [json!("Lune"), json!("Soleil")]);
    assert!(
        h.nil.rpc("lib:playlists", vec![]).unwrap()["playlists"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "Favoris")
    );
    sub.call("unstar", &[("id", id(0))]);
    assert_eq!(sub.call("getStarred", &[])["starred"]["song"][0]["title"], "Soleil");

    // Écoutes : « en cours » ne compte pas, « terminée » compte comme une écoute entière.
    let count = || h.nil.tracks().iter().find(|t| t["id"] == ids[0]["id"]).unwrap()["playCount"].clone();
    sub.call("scrobble", &[("id", id(0)), ("submission", s("false"))]);
    assert_eq!(count(), 0);
    let when = control_core::util::now_ms() - 60000;
    sub.call("scrobble", &[("id", id(0)), ("time", s(when))]);
    assert_eq!(count(), 1);
    let today = control_core::util::local_day(control_core::util::now_ms());
    assert_eq!(
        h.nil.rpc("stats:get", vec![json!({ "from": today, "to": today })]).unwrap()["data"]["music"]["plays"],
        1,
        "compte dans les statistiques"
    );
    assert_eq!(sub.call("scrobble", &[])["error"]["code"], 10);

    // Paroles : texte synchronisé converti en lignes avec leur temps (millisecondes).
    let path = std::path::PathBuf::from(ids[0]["path"].as_str().unwrap());
    std::fs::write(path.with_extension("lrc"), "[00:01.50] Bonjour\n[01:03.00] Monde\n[01:10.00] Fin\n").unwrap();
    let lyrics = sub.call("getLyricsBySongId", &[("id", id(0))])["lyricsList"]["structuredLyrics"][0].clone();
    assert_eq!(lyrics["synced"], true);
    assert_eq!(
        lyrics["line"],
        json!([{ "start": 1500, "value": "Bonjour" }, { "start": 63000, "value": "Monde" }, { "start": 70000, "value": "Fin" }])
    );
    let legacy = sub.call("getLyrics", &[("artist", s("aya")), ("title", s("LUNE"))])["lyrics"].clone();
    assert_eq!(legacy["value"], "Bonjour\nMonde\nFin");
    assert_eq!(sub.call("getNowPlaying", &[])["nowPlaying"], json!({}));
}

#[test]
fn connection_journal_gives_the_exact_reason_to_the_admin_only() {
    let s = boot(Boot { no_common: true, ..Boot::default() });
    let nil = s.client();
    nil.setup(json!({ "username": "nil", "password": PASSWORD, "libraryFolders": [s.music] }));
    let c = s.client();
    let ping = |q: &str, extra: &[(&str, &str)]| {
        c.raw("GET", &format!("/rest/ping.view?{q}&v=1.13.0&c=Amperfy&f=json"), None, extra, false).json()["subsonic-response"].clone()
    };

    ping("u=nil&p=n-importe-quoi", &[]);
    let secret =
        nil.rpc("srv:me:subsonic-reveal", vec![json!(PASSWORD)]).unwrap()["secret"].as_str().unwrap().to_string();
    ping(&format!("u=nil&p={}", urlencode(PASSWORD)), &[]);
    ping("u=nil&t=00000000000000000000000000000000&s=abc", &[]);
    ping("u=inconnu&p=x", &[]);
    ping(&format!("u=NIL&p={secret}"), &[("user-agent", "Amperfy/1.0")]);

    let log = nil.rpc("srv:subsonic:log", vec![]).unwrap();
    let log = log.as_array().unwrap();
    let reasons: Vec<&str> = log.iter().map(|e| e["reason"].as_str().unwrap()).collect();
    assert!(reasons[0].contains("connexion acceptée"), "la plus récente d'abord");
    assert!(log[0]["ok"] == true && log[0]["client"] == "Amperfy" && log[0]["user"] == "nil");
    assert!(log[0]["agent"].as_str().unwrap().starts_with("Amperfy"));
    assert!(reasons.iter().any(|r| r.contains("pas encore de mot de passe d’application")));
    assert!(reasons.iter().any(|r| r.contains("mot de passe du COMPTE")));
    assert!(reasons.iter().any(|r| r.contains("jeton incorrect")));
    assert!(reasons.iter().any(|r| r.contains("introuvable")));
    let text = serde_json::to_string(log).unwrap();
    assert!(!text.contains(&secret) && !text.contains(PASSWORD), "rien de secret dans le journal");
    // Le client, lui, ne reçoit que le message générique.
    let generic = ping(&format!("u=nil&p={}", urlencode(PASSWORD)), &[]);
    assert_eq!(generic["error"]["message"], "Nom d’utilisateur ou mot de passe d’application incorrect.");

    nil.rpc("srv:users:create", vec![json!({ "username": "ami", "password": PASSWORD })]).unwrap();
    let ami = s.client();
    ami.login("ami", PASSWORD);
    assert_eq!(ami.rpc("srv:subsonic:log", vec![]).unwrap_err().status, 403);
}

#[test]
fn throttle_uses_the_real_address_behind_a_proxy() {
    let s = boot(Boot { no_common: true, trust_proxy: true, ..Boot::default() });
    let nil = s.client();
    nil.setup(json!({ "username": "nil", "password": PASSWORD, "libraryFolders": [s.music] }));
    let secret =
        nil.rpc("srv:me:subsonic-reveal", vec![json!(PASSWORD)]).unwrap()["secret"].as_str().unwrap().to_string();
    let c = s.client();
    let call = |ip: &str, pass: &str| {
        c.raw("GET", &format!("/rest/ping?u=nil&p={pass}&v=1.16.1&c=x&f=json"), None, &[("x-forwarded-for", ip)], false)
            .json()["subsonic-response"]
            .clone()
    };
    for _ in 0..30 {
        call("203.0.113.7", "non");
    }
    assert!(
        call("203.0.113.7", &secret)["error"]["message"].as_str().unwrap().contains("Trop d’essais"),
        "l'attaquant est bloqué"
    );
    assert_eq!(call("198.51.100.20", &secret)["status"], "ok", "un autre client derrière le même proxy ne l'est pas");
}

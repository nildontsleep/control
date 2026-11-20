//! Essais du serveur web : démarrage, comptes, droits, médias, événements (port de `server/test/server.test.js`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::io::{BufRead as _, BufReader, Write as _};
use std::time::Duration;

use common::{Boot, PASSWORD, boot, raw_status, until, wav};
use serde_json::{Value, json};

#[test]
fn first_launch_leads_to_setup_and_creates_the_admin_once() {
    let s = boot(Boot::default());
    let c = s.client();
    assert_eq!(c.json("GET", "/api/setup/status", None).json()["initialized"], false);
    assert_eq!(c.json("GET", "/", None).header("location"), Some("/setup"));
    assert_eq!(c.json("GET", "/setup", None).status, 200);

    assert_eq!(c.setup(json!({ "username": "nil", "password": "court" })).status, 400);
    assert_eq!(c.setup(json!({ "username": "x", "password": PASSWORD })).status, 400);
    let ok = c.setup(json!({ "serverName": "Mon serveur", "username": "nil", "password": PASSWORD }));
    assert_eq!(ok.status, 200);
    let cookie = ok.header("set-cookie").unwrap();
    assert!(cookie.starts_with("control_session=") && cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"));
    assert!(!cookie.contains("Secure"), "pas de Secure en HTTP");
    assert_eq!(c.setup(json!({ "username": "other", "password": PASSWORD })).status, 403);
    assert_eq!(c.json("GET", "/", None).status, 200);
    assert_eq!(c.json("GET", "/setup", None).header("location"), Some("/"));
    assert_eq!(c.json("GET", "/api/setup/status", None).json()["serverName"], "Mon serveur");
}

#[test]
fn setup_token_is_required_when_defined() {
    let s = boot(Boot { setup_token: Some("sesame".into()), ..Boot::default() });
    let c = s.client();
    assert_eq!(c.json("GET", "/api/setup/status", None).json()["tokenRequired"], true);
    assert_eq!(c.setup(json!({ "username": "nil", "password": PASSWORD })).status, 403);
    assert_eq!(c.setup(json!({ "username": "nil", "password": PASSWORD, "token": "sesame" })).status, 200);
}

#[test]
fn login_wrong_password_throttling_and_logout() {
    let s = boot(Boot::default());
    s.client().setup(json!({ "username": "nil", "password": PASSWORD }));

    let c = s.client();
    assert_eq!(c.post("/api/rpc/lib:tracks", json!({ "args": [] })).status, 401);
    assert_eq!(c.json("GET", "/", None).header("location"), Some("/login"));
    assert_eq!(c.json("GET", "/admin", None).header("location"), Some("/login"));
    assert_eq!(c.json("GET", "/login", None).status, 200);
    assert_eq!(c.login("nil", "wrong password").status, 401);
    let good = c.login("NIL", PASSWORD); // le nom ne dépend pas de la casse
    assert_eq!(good.status, 200);
    assert_eq!(good.json()["user"]["role"], "admin");
    assert_eq!(c.me()["username"], "nil");
    assert_eq!(c.json("GET", "/login", None).header("location"), Some("/"));
    let out = c.post("/api/auth/logout", json!({}));
    assert!(out.header("set-cookie").unwrap().contains("Max-Age=0"));
    assert_eq!(c.json("GET", "/api/auth/me", None).status, 401);

    let attacker = s.client();
    for i in 0..8 {
        attacker.login("nil", &format!("guess {i}"));
    }
    let blocked = attacker.login("nil", PASSWORD);
    assert_eq!(blocked.status, 429);
    assert!(blocked.header("retry-after").unwrap().parse::<i64>().unwrap() > 0);
    assert_eq!(blocked.json()["error"]["code"], "too_many");
}

#[test]
fn changes_require_the_anti_csrf_header() {
    let s = boot(Boot::default());
    let c = s.client();
    c.setup(json!({ "username": "nil", "password": PASSWORD }));
    let res = c.raw("POST", "/api/rpc/lib:scan", Some((b"{\"args\":[]}".to_vec(), "application/json")), &[], false);
    assert_eq!(res.status, 403);
    assert_eq!(res.json()["error"]["code"], "csrf");
}

#[test]
fn library_media_with_range_covers_and_home_page() {
    let s = boot(Boot::default());
    let c = s.client();
    c.setup(json!({ "username": "nil", "password": PASSWORD }));
    let tracks = c.wait_tracks(3);
    let mut titles: Vec<&str> = tracks.iter().map(|t| t["title"].as_str().unwrap()).collect();
    titles.sort();
    assert_eq!(titles, ["01 - One", "02 - Two", "03 - Three"]);

    let id = tracks[0]["id"].as_i64().unwrap();
    let full = c.json("GET", &format!("/media/{id}"), None);
    assert_eq!(full.status, 200);
    assert_eq!(full.header("content-type"), Some("audio/wav"));
    assert_eq!(full.header("accept-ranges"), Some("bytes"));
    assert_eq!(full.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(full.header("x-frame-options"), Some("DENY"));
    let size = full.body.len();
    assert!(size > 1000);
    let part = c.request("GET", &format!("/media/{id}"), None, &[("range", "bytes=10-109")]);
    assert_eq!(part.status, 206);
    assert_eq!(part.header("content-range"), Some(format!("bytes 10-109/{size}").as_str()));
    assert_eq!(part.body.len(), 100);
    assert_eq!(part.body, full.body[10..110]);
    let tail = c.request("GET", &format!("/media/{id}"), None, &[("range", "bytes=-50")]);
    assert_eq!(tail.body.len(), 50);
    let range = format!("bytes={}-", size + 5);
    assert_eq!(c.request("GET", &format!("/media/{id}"), None, &[("range", &range)]).status, 416);
    let etag = full.header("etag").unwrap().to_string();
    assert_eq!(c.request("GET", &format!("/media/{id}"), None, &[("if-none-match", &etag)]).status, 304);
    let head = c.raw("HEAD", &format!("/media/{id}"), None, &[], true);
    assert_eq!((head.status, head.body.len()), (200, 0));
    assert_eq!(c.json("GET", "/media/99999", None).status, 404);
    assert_eq!(c.json("GET", "/media/abc", None).status, 404);
    assert_eq!(raw_status(&s.base(), "/cover/../../etc/passwd", &c.cookie()), 404);

    let page = c.json("GET", "/", None);
    let csp = page.header("content-security-policy").unwrap();
    assert!(csp.contains("frame-src") && csp.contains("media-src 'self' blob:"));
    let text = page.text();
    assert!(text.contains("/web/shim.js") && text.contains("/web/app-web.css"));
    let state = c.boot_state("/");
    assert_eq!(state["user"]["username"], "nil");
    assert_eq!(state["caps"]["manageServer"], true);
    assert!(!state["settings"]["commonFolders"].as_array().unwrap().is_empty());
    assert_eq!(state["info"]["platform"], "web");

    // Jamais de fichier hors des dossiers de l'application ou de la bibliothèque.
    let cookie = c.cookie();
    assert_eq!(raw_status(&s.base(), "/js/../../package.json", &cookie), 404);
    assert_eq!(raw_status(&s.base(), "/js/%2e%2e/%2e%2e/package.json", &cookie), 404);
    assert_eq!(raw_status(&s.base(), "/web/../../server/config.js", &cookie), 404);
    assert_eq!(c.json("GET", "/js/app.js", None).status, 200);
    assert_eq!(c.json("GET", "/web/login.js", None).status, 200);
    let manifest = c.json("GET", "/manifest.webmanifest", None);
    assert_eq!(manifest.header("content-type"), Some("application/manifest+json"));
    assert_eq!(manifest.json()["name"], "control");
    assert_eq!(c.json("GET", "/sw.js", None).header("service-worker-allowed"), Some("/"));
    assert_eq!(c.json("GET", "/api/health", None).json()["ok"], true);
    assert_eq!(c.json("GET", "/api/nimporte", None).json()["error"]["code"], "not_found");
}

#[test]
fn each_account_has_its_own_plays_and_rights() {
    let s = boot(Boot::default());
    let admin = s.client();
    admin.setup(json!({ "username": "nil", "password": PASSWORD }));
    let tracks = admin.wait_tracks(3);
    let id = tracks[0]["id"].clone();
    let play = admin.rpc("lib:play-start", vec![id.clone(), json!(1)]).unwrap();
    assert_eq!(admin.send("lib:play-end", vec![play, json!(40), json!(true)]).status, 204);
    let created =
        admin.rpc("srv:users:create", vec![json!({ "username": "ami", "password": "another password" })]).unwrap();
    assert_eq!(created["role"], "user");
    let again = admin.rpc("srv:users:create", vec![json!({ "username": "ami", "password": "another password" })]);
    assert_eq!(again.unwrap_err().status, 409);

    let ami = s.client();
    assert_eq!(ami.login("ami", "another password").status, 200);
    let mine = ami.wait_tracks(3);
    assert!(mine.iter().all(|t| t["playCount"] == 0));
    assert_eq!(admin.tracks().iter().find(|t| t["id"] == id).unwrap()["playCount"], 1);
    let today = control_core::util::local_day(control_core::util::now_ms());
    let range = json!({ "from": today, "to": today });
    assert_eq!(ami.rpc("stats:get", vec![range.clone()]).unwrap()["data"]["music"]["plays"], 0);
    assert_eq!(admin.rpc("stats:get", vec![range]).unwrap()["data"]["music"]["plays"], 1);

    let first = mine[0]["id"].clone();
    for (channel, args) in [
        ("tags:write", vec![json!([])]),
        ("tools:install", vec![]),
        ("srv:users:list", vec![]),
        ("srv:add-folder", vec![json!("/tmp")]),
        ("lib:remove-folder", vec![json!("/x")]),
        ("lyrics:save-file", vec![first.clone()]),
        ("lib:remove-tracks", vec![json!([first.clone()]), json!(true)]),
        ("dl:add", vec![json!({})]),
        ("lib:add-paths", vec![json!(["/etc/passwd"])]),
    ] {
        assert_eq!(ami.rpc(channel, args).unwrap_err().status, 403, "{channel}");
    }
    assert_eq!(ami.rpc("nimporte:quoi", vec![]).unwrap_err().status, 404);
    // Ce qui reste permis : ses propres playlists et réglages.
    let pl = ami.rpc("pl:create", vec![json!("Mes favoris"), json!("manual"), Value::Null, json!([first])]).unwrap();
    assert!(pl.as_i64().unwrap() > 0);
    assert_eq!(admin.rpc("lib:playlists", vec![]).unwrap()["playlists"], json!([]));

    // Les réglages du serveur ne passent pas par les réglages d'un compte ordinaire.
    ami.send("settings:set", vec![json!({ "libraryFolders": ["/etc"], "theme": "light" })]);
    let state = ami.boot_state("/");
    assert_eq!(state["settings"]["theme"], "light");
    assert_eq!(state["settings"]["commonFolders"], json!([s.music.to_string_lossy()]));
    assert_eq!(state["settings"]["libraryFolders"], json!([]));
    assert_eq!(state["caps"]["editFiles"], false);
    assert_eq!(ami.json("GET", "/admin", None).header("location"), Some("/"));

    // Compte désactivé : sa session est coupée.
    admin.rpc("srv:users:update", vec![created["id"].clone(), json!({ "disabled": true })]).unwrap();
    assert_eq!(ami.json("GET", "/api/auth/me", None).status, 401);
    assert_eq!(ami.login("ami", "another password").status, 401);
    // Le dernier administrateur ne peut ni se rétrograder ni se supprimer.
    let me = admin.me();
    assert_eq!(
        admin.rpc("srv:users:update", vec![me["id"].clone(), json!({ "role": "user" })]).unwrap_err().status,
        409
    );
    assert_eq!(admin.rpc("srv:users:delete", vec![me["id"].clone()]).unwrap_err().status, 409);
    // Supprimer un compte efface son dossier.
    admin.rpc("srv:users:delete", vec![created["id"].clone()]).unwrap();
    assert!(!s.root.join("data").join("users").join(created["id"].as_str().unwrap()).exists());
    assert_eq!(admin.rpc("srv:users:list", vec![]).unwrap().as_array().unwrap().len(), 1);
}

#[test]
fn registration_closed_by_default_opened_by_admin() {
    let s = boot(Boot::default());
    let admin = s.client();
    admin.setup(json!({ "username": "nil", "password": PASSWORD }));
    let anon = s.client();
    let body = json!({ "username": "newbie", "password": "some password 1" });
    assert_eq!(anon.post("/api/auth/register", body.clone()).status, 403);
    admin.rpc("srv:settings", vec![json!({ "registrationEnabled": true })]).unwrap();
    let ok = anon.post("/api/auth/register", body.clone());
    assert_eq!(ok.status, 200);
    assert_eq!(ok.json()["user"]["role"], "user");
    assert_eq!(anon.post("/api/auth/register", body).status, 409);
}

/// Un flux d'événements lu en arrière-plan : (nom, données).
fn listen(base: &str, cookie: &str, client: &str) -> std::sync::Arc<parking_lot::Mutex<Vec<(String, Value)>>> {
    let events = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let addr = base.trim_start_matches("http://").to_string();
    let (cookie, client, out) = (cookie.to_string(), client.to_string(), events.clone());
    std::thread::spawn(move || {
        let mut s = std::net::TcpStream::connect(&addr).unwrap();
        write!(s, "GET /api/events?c={client} HTTP/1.1\r\nHost: {addr}\r\nCookie: {cookie}\r\n\r\n").unwrap();
        let mut reader = BufReader::new(s);
        let (mut name, mut headers_done, mut is_sse) = (String::new(), false, false);
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            let l = line.trim_end().to_string();
            line.clear();
            if !headers_done {
                if l.to_lowercase() == "content-type: text/event-stream" {
                    is_sse = true;
                }
                headers_done = l.is_empty();
                continue;
            }
            if let Some(n) = l.strip_prefix("event: ") {
                name = n.to_string();
            } else if let Some(d) = l.strip_prefix("data: ")
                && is_sse
            {
                out.lock().push((std::mem::take(&mut name), serde_json::from_str(d).unwrap_or(Value::Null)));
            }
        }
    });
    events
}

#[test]
fn live_events_scan_and_settings() {
    let s = boot(Boot::default());
    let c = s.client();
    c.setup(json!({ "username": "nil", "password": PASSWORD }));
    c.wait_tracks(3);
    let events = listen(&s.base(), &c.cookie(), "abc123");
    until(|| events.lock().iter().any(|(n, _)| n == "ready").then_some(()));

    // L'auteur d'un réglage ne reçoit pas son propre écho ; un autre onglet, si.
    c.request(
        "POST",
        "/api/send/settings%3Aset",
        Some(&json!({ "args": [{ "volume": 0.3 }] })),
        &[("x-client", "abc123")],
    );
    c.request(
        "POST",
        "/api/send/settings%3Aset",
        Some(&json!({ "args": [{ "volume": 0.6 }] })),
        &[("x-client", "autre")],
    );
    until(|| events.lock().iter().any(|(n, d)| n == "settings:changed" && d["volume"] == 0.6).then_some(()));
    assert!(!events.lock().iter().any(|(n, d)| n == "settings:changed" && d["volume"] == 0.3));

    // Une nouvelle musique apparaît dans les dossiers : la bibliothèque le signale.
    wav(&s.music.join("Artist C").join("04 - Four.wav"), 1);
    c.rpc("lib:scan", vec![]).unwrap();
    until(|| events.lock().iter().any(|(n, _)| n == "lib:changed").then_some(()));
    assert_eq!(c.tracks().len(), 4);
    assert!(s.hub().connected_clients() >= 1);
}

#[test]
fn server_folders_add_remove_and_browse() {
    let s = boot(Boot::default());
    let c = s.client();
    c.setup(json!({ "username": "nil", "password": PASSWORD }));
    let extra = s.root.join("more");
    wav(&extra.join("Artist Z").join("09 - Nine.wav"), 1);
    assert_eq!(c.rpc("srv:add-folder", vec![json!(s.root.join("absent"))]).unwrap_err().status, 400);
    let file = extra.join("Artist Z").join("09 - Nine.wav");
    assert_eq!(c.rpc("srv:add-folder", vec![json!(file)]).unwrap_err().status, 400);
    assert_eq!(c.rpc("srv:add-folder", vec![json!("/")]).unwrap_err().code, "too_broad");
    let folders = c.rpc("srv:add-folder", vec![json!(extra)]).unwrap();
    assert_eq!(folders.as_array().unwrap().len(), 2);
    c.wait_tracks(4);
    let listing = c.rpc("srv:browse", vec![json!(s.root)]).unwrap();
    let dirs: Vec<&str> = listing["dirs"].as_array().unwrap().iter().map(|d| d.as_str().unwrap()).collect();
    assert!(dirs.contains(&"music") && dirs.contains(&"more"));
    assert_eq!(listing["parent"], json!(s.root.parent().unwrap()));
    c.rpc("srv:remove-folder", vec![json!(extra)]).unwrap();
    c.wait_tracks(3);
    let info = c.rpc("srv:info", vec![]).unwrap();
    assert_eq!((info["role"].as_str(), info["users"].as_i64()), (Some("admin"), Some(1)));
}

#[test]
fn two_simultaneous_setups_only_one_succeeds() {
    let s = boot(Boot::default());
    let base = s.base();
    let handles: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|name| {
            let base = base.clone();
            std::thread::spawn(move || {
                common::Client::new(&base).setup(json!({ "username": name, "password": PASSWORD })).status
            })
        })
        .collect();
    let mut statuses: Vec<u16> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    statuses.sort();
    assert_eq!(statuses, [200, 403]);
    assert_eq!(s.hub().accounts.count(), 1);
}

#[test]
fn sessions_list_revoke_and_isolation() {
    let s = boot(Boot::default());
    let phone = s.client();
    phone.setup(json!({ "username": "nil", "password": PASSWORD, "libraryFolders": [s.music] }));
    let laptop = s.client();
    let login = json!({ "username": "nil", "password": PASSWORD });
    laptop.request(
        "POST",
        "/api/auth/login",
        Some(&login),
        &[("user-agent", "Mozilla/5.0 (X11; Linux x86_64) Firefox/130.0")],
    );
    let tablet = s.client();
    tablet.request(
        "POST",
        "/api/auth/login",
        Some(&login),
        &[("user-agent", "Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) Safari/605.1.15")],
    );

    let list = laptop.rpc("srv:me:sessions", vec![]).unwrap();
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list.iter().filter(|x| x["current"] == true).count(), 1);
    assert!(list.iter().find(|x| x["current"] == true).unwrap()["userAgent"].as_str().unwrap().contains("Firefox"));
    assert!(list.iter().all(|x| x["id"].as_str().unwrap().len() == 16 && x["lastSeen"].as_i64().unwrap() > 0));

    // Un autre compte ne voit ni ne ferme les sessions du premier.
    phone.rpc("srv:users:create", vec![json!({ "username": "ami", "password": PASSWORD })]).unwrap();
    let ami = s.client();
    ami.login("ami", PASSWORD);
    assert_eq!(ami.rpc("srv:me:sessions", vec![]).unwrap().as_array().unwrap().len(), 1);
    assert_eq!(ami.rpc("srv:me:revoke-session", vec![list[0]["id"].clone()]).unwrap(), false);
    assert_eq!(laptop.rpc("srv:me:sessions", vec![]).unwrap().as_array().unwrap().len(), 3);

    // Fermer la session de la tablette : elle perd l'accès, les autres restent.
    let tablet_id = list.iter().find(|x| x["userAgent"].as_str().unwrap().contains("iPad")).unwrap()["id"].clone();
    assert_eq!(laptop.rpc("srv:me:revoke-session", vec![tablet_id]).unwrap(), true);
    assert_eq!(tablet.rpc("lib:tracks", vec![]).unwrap_err().status, 401);
    assert_eq!(laptop.rpc("srv:me:sessions", vec![]).unwrap().as_array().unwrap().len(), 2);

    // Fermer toutes les autres : seule celle du portable reste.
    assert_eq!(laptop.rpc("srv:me:revoke-others", vec![]).unwrap(), 1);
    assert_eq!(phone.rpc("lib:tracks", vec![]).unwrap_err().status, 401);
    let left = laptop.rpc("srv:me:sessions", vec![]).unwrap();
    assert_eq!(left.as_array().unwrap().iter().map(|x| x["current"].clone()).collect::<Vec<_>>(), [json!(true)]);

    // Changer son mot de passe demande l'actuel.
    assert_eq!(
        laptop.rpc("srv:me:password", vec![json!("faux"), json!("nouveau mot de passe")]).unwrap_err().status,
        403
    );
    assert_eq!(laptop.rpc("srv:me:password", vec![json!(PASSWORD), json!("nouveau mot de passe")]).unwrap(), true);
    assert_eq!(s.client().login("nil", "nouveau mot de passe").status, 200);
}

#[test]
fn tracks_payload_is_cached_until_the_library_changes() {
    let s = boot(Boot::default());
    let admin = s.client();
    admin.setup(json!({ "username": "nil", "password": PASSWORD, "libraryFolders": [s.music] }));
    let tracks = admin.wait_tracks(3);
    let mut keys: Vec<&str> = tracks[0].as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    let mut expected = [
        "addedAt",
        "album",
        "albumArtist",
        "artist",
        "bitrate",
        "codec",
        "cover",
        "discNo",
        "duration",
        "genre",
        "height",
        "id",
        "kind",
        "lastPlayed",
        "loudness",
        "path",
        "playCount",
        "sourceUrl",
        "size",
        "title",
        "trackNo",
        "truePeak",
        "width",
        "year",
    ];
    expected.sort();
    assert_eq!(keys, expected);

    let hub = s.hub();
    let hits = || hub.cache_hits.load(std::sync::atomic::Ordering::SeqCst);
    let misses = || hub.cache_misses.load(std::sync::atomic::Ordering::SeqCst);
    until(|| {
        let before = hits();
        admin.tracks();
        (hits() > before).then_some(())
    });
    let m = misses();
    let again = admin.tracks();
    assert_eq!(misses(), m);
    assert_eq!(again, admin.tracks());

    // Une écoute change le compteur : la liste est refaite.
    let play = admin.rpc("lib:play-start", vec![tracks[0]["id"].clone(), json!(1)]).unwrap();
    assert!(play.as_i64().unwrap() > 0);
    let after = admin.tracks();
    assert!(misses() > m);
    assert_eq!(after.iter().find(|t| t["id"] == tracks[0]["id"]).unwrap()["playCount"], 1);

    // Un client qui ne sait pas lire le gzip reçoit le même contenu décompressé.
    let raw = admin.raw(
        "POST",
        "/api/rpc/lib%3Atracks",
        Some((b"{\"args\":[]}".to_vec(), "application/json")),
        &[("accept-encoding", "identity")],
        true,
    );
    assert_eq!(raw.header("content-encoding"), None);
    assert_eq!(raw.json()["result"].as_array().unwrap().len(), 3);

    // Une playlist aussi modifie la base : elle ne sert jamais une liste périmée.
    let h = hits();
    admin.rpc("pl:create", vec![json!("Essai"), json!("manual"), Value::Null, json!([tracks[1]["id"]])]).unwrap();
    admin.tracks();
    assert_eq!(hits(), h, "après un changement : un nouveau calcul");
    std::thread::sleep(Duration::from_millis(10));
}

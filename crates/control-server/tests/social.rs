//! Bibliothèques par compte, partages, lecture seule, profils, playlists, comparaison, « en écoute »
//! (port de `server/test/social.test.js` et `presence.test.js`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;
use std::path::PathBuf;

use base64::Engine as _;
use common::{Boot, Client, PASSWORD, Server, boot, wav};
use serde_json::{Value, json};

/// Un PNG de couleur unie, en adresse data: (de quoi tester les envois d'images).
fn png_data_url(w: u32, h: u32) -> String {
    let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 60, 90]));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(out))
}

struct House {
    s: Server,
    nil: Client,
    ami: Client,
    zoe: Client,
    ids: HashMap<&'static str, String>,
    ami_music: PathBuf,
}

/// Un serveur sans dossier commun : nil (administrateur, 3 morceaux à lui), ami (1 morceau), zoe (rien).
fn house() -> House {
    let s = boot(Boot { no_common: true, ..Boot::default() });
    let nil = s.client();
    nil.setup(json!({ "username": "nil", "password": PASSWORD, "libraryFolders": [s.music] }));
    let mut ids = HashMap::new();
    for name in ["ami", "zoe"] {
        let u = nil.rpc("srv:users:create", vec![json!({ "username": name, "password": PASSWORD })]).unwrap();
        ids.insert(name, u["id"].as_str().unwrap().to_string());
    }
    let ami_music = s.root.join("ami-music");
    wav(&ami_music.join("Band").join("Song.wav"), 1);
    nil.rpc("srv:users:add-folder", vec![json!(ids["ami"]), json!(ami_music)]).unwrap();
    let ami = s.client();
    ami.login("ami", PASSWORD);
    let zoe = s.client();
    zoe.login("zoe", PASSWORD);
    ids.insert("nil", nil.me()["id"].as_str().unwrap().to_string());
    House { s, nil, ami, zoe, ids, ami_music }
}

#[test]
fn each_account_has_its_own_library_and_admin_assigns_folders() {
    let h = house();
    h.nil.wait_tracks(3);
    h.ami.wait_tracks(1);
    assert_eq!(h.zoe.tracks().len(), 0);

    // Un compte ordinaire ne choisit pas ses dossiers.
    assert_eq!(h.ami.rpc("srv:me:add-folder", vec![json!(h.s.music)]).unwrap_err().status, 403);
    assert_eq!(h.ami.rpc("srv:users:add-folder", vec![json!(h.ids["ami"]), json!(h.s.music)]).unwrap_err().status, 403);
    h.ami.send("settings:set", vec![json!({ "libraryFolders": [h.s.music], "downloadDir": "/tmp" })]);
    let detail = h.nil.rpc("srv:users:detail", vec![json!(h.ids["ami"])]).unwrap();
    assert_eq!(detail["folders"].as_array().unwrap().len(), 1);

    // Les fichiers servis sont ceux de son propre compte.
    let mine = h.ami.tracks()[0].clone();
    assert_eq!(h.ami.json("GET", &format!("/media/{}", mine["id"]), None).status, 200);
    let high = h.nil.tracks().iter().map(|t| t["id"].as_i64().unwrap()).max().unwrap();
    assert_eq!(h.ami.json("GET", &format!("/media/{high}"), None).status, 404);

    // Retirer un dossier de la liste d'un compte le vide de ses morceaux (les fichiers restent).
    let folder = detail["folders"][0].clone();
    h.nil.rpc("srv:users:remove-folder", vec![json!(h.ids["ami"]), folder]).unwrap();
    h.ami.wait_tracks(0);
    assert!(h.ami_music.join("Band").join("Song.wav").exists());
}

#[test]
fn personal_folder_and_common_folders() {
    let h = house();
    let users_root = h.s.root.join("users-root");
    std::fs::create_dir_all(&users_root).unwrap();
    h.nil.rpc("srv:settings", vec![json!({ "usersRoot": users_root })]).unwrap();
    let created = h.nil.rpc("srv:users:create", vec![json!({ "username": "lea", "password": PASSWORD })]).unwrap();
    assert!(users_root.join("lea").is_dir());
    let detail = h.nil.rpc("srv:users:detail", vec![created["id"].clone()]).unwrap();
    assert_eq!(detail["folders"], json!([users_root.join("lea")]));

    // Pour un compte existant, à la demande.
    h.nil.rpc("srv:users:personal-folder", vec![json!(h.ids["zoe"])]).unwrap();
    assert!(users_root.join("zoe").exists());

    // Un dossier commun apparaît chez tout le monde.
    let shared = h.s.root.join("shared");
    wav(&shared.join("Commun").join("Tune.wav"), 1);
    h.nil.rpc("srv:add-folder", vec![json!(shared)]).unwrap();
    h.zoe.wait_tracks(1);
}

#[test]
fn sharing_is_read_only_and_only_what_is_shared() {
    let h = house();
    h.nil.wait_tracks(3);
    let nil_id = h.ids["nil"].clone();

    // Rien n'est partagé : refus, et aucune fuite.
    assert_eq!(h.ami.rpc_in(&nil_id, "lib:tracks", vec![]).unwrap_err().status, 403);
    let none = h.ami.rpc("social:others", vec![]).unwrap();
    assert_eq!(none.as_array().unwrap().iter().find(|p| p["username"] == "nil").unwrap()["access"]["browse"], false);

    // nil partage avec ami seulement.
    h.nil
        .rpc("social:set-grants", vec![json!([{ "viewerId": h.ids["ami"], "browse": true, "compare": false }])])
        .unwrap();
    let view = h.ami.rpc_in(&nil_id, "lib:tracks", vec![]).unwrap();
    let view = view.as_array().unwrap();
    assert_eq!(view.len(), 3);
    assert!(view.iter().all(|t| !PathBuf::from(t["path"].as_str().unwrap()).is_absolute() && t["playCount"] == 0));
    assert_eq!(h.zoe.rpc_in(&nil_id, "lib:tracks", vec![]).unwrap_err().status, 403);

    // Les fichiers : seulement avec ?lib=, et jamais hors des dossiers propres du propriétaire.
    let id = view[0]["id"].as_i64().unwrap();
    let shared = h.ami.request("GET", &format!("/media/{id}?lib={nil_id}"), None, &[("range", "bytes=0-9")]);
    assert_eq!(shared.status, 206);
    assert_eq!(h.zoe.json("GET", &format!("/media/{id}?lib={nil_id}"), None).status, 403);

    // Lecture seule : ni playlists, ni tags, ni suppression ; les écoutes ne sont pas comptées.
    let err = h.ami.rpc_in(&nil_id, "pl:create", vec![json!("Intruse")]).unwrap_err();
    assert_eq!((err.status, err.code.as_str()), (403, "read_only"));
    assert_eq!(h.ami.rpc_in(&nil_id, "lib:remove-tracks", vec![json!([id]), json!(true)]).unwrap_err().status, 403);
    assert_eq!(h.ami.rpc_in(&nil_id, "lib:play-start", vec![json!(id), json!(10)]).unwrap(), 0);
    let r = h.ami.request(
        "POST",
        "/api/send/lib%3Aplay-end",
        Some(&json!({ "args": [1, 10, true] })),
        &[("x-library", &nil_id)],
    );
    assert_eq!(r.status, 204);
    assert_eq!(h.nil.tracks().iter().find(|t| t["id"] == id).unwrap()["playCount"], 0);
    assert!(h.ami.rpc_in(&nil_id, "lyrics:get", vec![json!(id)]).unwrap()["ok"].is_boolean());

    // La page d'accueil annonce la vue ; une bibliothèque non partagée ramène à la sienne.
    let state = h.ami.boot_state(&format!("/?lib={nil_id}"));
    assert_eq!(state["viewing"]["username"], "nil");
    assert_eq!(state["caps"]["readOnly"], true);
    assert!(state["libraries"].as_array().unwrap().iter().any(|p| p["username"] == "nil"));
    assert_eq!(h.zoe.json("GET", &format!("/?lib={nil_id}"), None).status, 302);

    // Tout le monde ('*') ; l'administrateur parcourt tout, mais pas les statistiques.
    h.ami.rpc("social:set-grants", vec![json!([{ "viewerId": "*", "browse": true }])]).unwrap();
    h.ami.wait_tracks(1);
    assert_eq!(h.zoe.rpc_in(&h.ids["ami"], "lib:tracks", vec![]).unwrap().as_array().unwrap().len(), 1);
    let others = h.nil.rpc("social:others", vec![]).unwrap();
    let admin_view = others.as_array().unwrap().iter().find(|p| p["username"] == "zoe").unwrap().clone();
    assert_eq!(admin_view["access"], json!({ "browse": true, "compare": false }));

    // Retirer le partage coupe l'accès aussitôt.
    h.nil.rpc("social:set-grants", vec![json!([])]).unwrap();
    assert_eq!(h.ami.rpc_in(&nil_id, "lib:tracks", vec![]).unwrap_err().status, 403);
}

#[test]
fn custom_playlists_description_cover_visibility() {
    let h = house();
    let tracks = h.nil.wait_tracks(3);
    let pl = h
        .nil
        .rpc(
            "pl:create",
            vec![json!("Ma sélection"), json!("manual"), Value::Null, json!([tracks[0]["id"], tracks[1]["id"]])],
        )
        .unwrap();
    let patch = json!({ "name": "Été", "description": "Pour la route", "shared": true });
    assert_eq!(h.nil.rpc("pl:update", vec![pl.clone(), patch]).unwrap(), true);
    let cover = h.nil.rpc("pl:set-cover", vec![pl.clone(), json!({ "data": png_data_url(24, 16) })]).unwrap();
    assert_eq!(cover["ok"], true);
    let hash = cover["cover"].as_str().unwrap().to_string();
    assert!(hash.len() == 20 && hash.chars().all(|c| c.is_ascii_hexdigit()));
    let img = h.nil.json("GET", &format!("/cover/{hash}.jpg"), None);
    assert_eq!((img.status, img.header("content-type")), (200, Some("image/jpeg")));
    assert_eq!(h.nil.json("GET", &format!("/cover/{hash}-s.jpg"), None).status, 200);
    let listed = h.nil.rpc("lib:playlists", vec![]).unwrap()["playlists"][0].clone();
    assert_eq!(
        (&listed["name"], &listed["description"], &listed["shared"], &listed["cover"]),
        (&json!("Été"), &json!("Pour la route"), &json!(true), &json!(hash))
    );

    // Une image qui n'en est pas une est refusée ; null revient à la mosaïque.
    assert_eq!(
        h.nil.rpc("pl:set-cover", vec![pl.clone(), json!({ "data": "data:image/png;base64,AAAA" })]).unwrap()["ok"],
        false
    );
    assert_eq!(h.nil.rpc("pl:set-cover", vec![pl.clone(), json!({ "data": "pas une image" })]).unwrap()["ok"], false);
    assert_eq!(h.nil.rpc("pl:set-cover", vec![pl.clone(), json!({ "trackId": 999999 })]).unwrap()["ok"], false);
    assert_eq!(h.nil.rpc("pl:set-cover", vec![pl.clone(), Value::Null]).unwrap()["cover"], Value::Null);

    // Visible des autres seulement si partagée, et seulement avec le droit de parcourir.
    let secret = h.nil.rpc("pl:create", vec![json!("Perso")]).unwrap();
    h.nil.rpc("social:set-grants", vec![json!([{ "viewerId": h.ids["ami"], "browse": true }])]).unwrap();
    let seen = h.ami.rpc_in(&h.ids["nil"], "lib:playlists", vec![]).unwrap();
    let names: Vec<&str> = seen["playlists"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Été"]);
    assert_eq!(seen["items"].as_array().unwrap().len(), 2);
    assert!(!seen["playlists"].as_array().unwrap().iter().any(|p| p["id"] == secret));
    let profile = h.ami.rpc("social:profile", vec![json!(h.ids["nil"])]).unwrap();
    assert_eq!(
        profile["playlists"],
        json!([{ "id": pl, "name": "Été", "description": "Pour la route", "cover": null, "count": 2 }])
    );
    assert_eq!(profile["library"]["tracks"], 3);
}

#[test]
fn profile_name_bio_color_avatar() {
    let h = house();
    let saved = h
        .ami
        .rpc(
            "social:set-profile",
            vec![json!({ "displayName": "  Amélie ", "bio": "Du rock, du jazz.", "color": "#ff6a5f" })],
        )
        .unwrap();
    assert_eq!(
        (&saved["displayName"], &saved["name"], &saved["bio"], &saved["color"]),
        (&json!("Amélie"), &json!("Amélie"), &json!("Du rock, du jazz."), &json!("#ff6a5f"))
    );
    assert_eq!(h.ami.rpc("social:set-profile", vec![json!({ "color": "rouge" })]).unwrap_err().status, 400);

    let with_avatar =
        h.ami.rpc("social:set-profile", vec![json!({ "avatar": { "data": png_data_url(80, 40) } })]).unwrap();
    let avatar = with_avatar["avatar"].as_str().unwrap().to_string();
    assert_eq!(avatar.len(), 20);
    let res = h.nil.json("GET", &format!("/avatar/{avatar}.jpg"), None);
    assert_eq!((res.status, res.header("content-type")), (200, Some("image/jpeg")));
    let img = image::load_from_memory(&res.body).unwrap();
    assert_eq!((img.width(), img.height()), (40, 40), "recadrée au centre, en carré");
    let bad = h.ami.rpc("social:set-profile", vec![json!({ "avatar": { "data": "data:image/png;base64,AAAA" } })]);
    assert_eq!(bad.unwrap_err().status, 400);
    assert_eq!(h.ami.rpc("social:set-profile", vec![json!({ "avatar": null })]).unwrap()["avatar"], Value::Null);

    // Les autres comptes voient le nom affiché.
    let others = h.nil.rpc("social:others", vec![]).unwrap();
    assert_eq!(others.as_array().unwrap().iter().find(|p| p["id"] == h.ids["ami"].as_str()).unwrap()["name"], "Amélie");
    assert_eq!(h.ami.me()["name"], "Amélie");
    let me = h.ami.rpc("social:me", vec![]).unwrap();
    assert_eq!((&me["shareListening"], &me["profile"]["name"]), (&json!(false), &json!("Amélie")));
}

#[test]
fn compare_only_with_permission_and_without_history() {
    let h = house();
    let nil_id = json!(h.ids["nil"]);
    assert_eq!(h.ami.rpc("social:compare", vec![nil_id.clone(), json!({})]).unwrap_err().status, 403);
    assert_eq!(h.ami.rpc("social:compare", vec![json!(h.ids["ami"]), json!({})]).unwrap_err().status, 400);

    h.nil
        .rpc("social:set-grants", vec![json!([{ "viewerId": h.ids["ami"], "browse": false, "compare": true }])])
        .unwrap();
    let out =
        h.ami.rpc("social:compare", vec![nil_id.clone(), json!({ "from": "2000-01-01", "to": "2999-12-31" })]).unwrap();
    assert_eq!(out["with"]["username"], "nil");
    assert!(out["score"].is_number());
    assert!(out["me"]["topArtists"].is_array() && out["other"]["topArtists"].is_array());
    assert!(!out.to_string().contains("played_at"));
    // Comparer ne donne pas accès à la bibliothèque.
    assert_eq!(h.ami.rpc_in(&h.ids["nil"], "lib:tracks", vec![]).unwrap_err().status, 403);
    let profile = h.ami.rpc("social:profile", vec![nil_id]).unwrap();
    assert_eq!(profile["library"], Value::Null);
    assert!(profile["stats"]["year"].as_i64().unwrap() > 2000);
}

#[test]
fn listening_is_opt_in_visible_to_shares_live_and_expires() {
    let h = house();
    let tracks = h.nil.wait_tracks(3);
    let listeners = |who: &Client| -> HashMap<String, Value> {
        who.rpc("social:others", vec![])
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (p["username"].as_str().unwrap().to_string(), p["listening"].clone()))
            .collect()
    };
    h.nil
        .rpc("social:set-grants", vec![json!([{ "viewerId": h.ids["ami"], "browse": true, "compare": false }])])
        .unwrap();
    // Les événements d'ami : un flux SSE ouvert à côté.
    let entry = h.s.hub().entry(&h.s.hub().accounts.by_id(&h.ids["ami"]).unwrap()).unwrap();
    let (_conn, mut rx) = entry.clients.add("essai");
    let mut events = Vec::new();
    let mut drain = |events: &mut Vec<(String, Value)>| {
        while let Ok(b) = rx.try_recv() {
            let text = String::from_utf8_lossy(&b).into_owned();
            if let (Some(n), Some(d)) = (
                text.lines().find_map(|l| l.strip_prefix("event: ")),
                text.lines().find_map(|l| l.strip_prefix("data: ")),
            ) {
                events.push((n.to_string(), serde_json::from_str::<Value>(d).unwrap()));
            }
        }
    };

    // Par défaut rien n'est montré.
    assert_eq!(h.nil.rpc("social:me", vec![]).unwrap()["shareListening"], false);
    h.nil.rpc("lib:play-start", vec![tracks[0]["id"].clone(), json!(5)]).unwrap();
    assert_eq!(listeners(&h.ami)["nil"], Value::Null);

    // L'option activée : le morceau suivant apparaît pour ami, pas pour zoe.
    assert_eq!(h.nil.rpc("social:set-listening", vec![json!(true)]).unwrap(), true);
    let play = h.nil.rpc("lib:play-start", vec![tracks[1]["id"].clone(), json!(5)]).unwrap().as_i64().unwrap();
    let seen = listeners(&h.ami)["nil"].clone();
    assert_eq!(
        (&seen["trackId"], &seen["title"], &seen["duration"]),
        (&tracks[1]["id"], &tracks[1]["title"], &tracks[1]["duration"])
    );
    assert_eq!(listeners(&h.zoe)["nil"], Value::Null, "zoe n'a pas de partage");
    assert!(!listeners(&h.nil).contains_key("nil"), "on ne se voit pas soi-même");
    drain(&mut events);
    assert!(events.iter().any(|(c, p)| c == "social:listening"
        && p["id"] == h.ids["nil"].as_str()
        && p["now"]["title"] == tracks[1]["title"]));

    // « Tout le monde » : zoe voit aussi.
    h.nil.rpc("social:set-grants", vec![json!([{ "viewerId": "*", "browse": true, "compare": false }])]).unwrap();
    assert_eq!(listeners(&h.zoe)["nil"]["title"], tracks[1]["title"]);
    h.nil
        .rpc("social:set-grants", vec![json!([{ "viewerId": h.ids["ami"], "browse": true, "compare": false }])])
        .unwrap();
    assert_eq!(listeners(&h.zoe)["nil"], Value::Null);

    // La fin de l'écoute efface ; la fin d'une autre écoute n'efface pas celle-ci.
    h.nil.send("lib:play-end", vec![json!(play + 1000), json!(1), json!(true)]);
    assert!(listeners(&h.ami)["nil"].is_object());
    h.nil.send("lib:play-end", vec![json!(play), json!(5), json!(true)]);
    assert_eq!(listeners(&h.ami)["nil"], Value::Null);
    drain(&mut events);
    assert!(events.iter().any(|(c, p)| c == "social:listening" && p["now"].is_null()));

    // Expiration : un morceau dont la pause dépasse sa durée disparaît.
    h.nil.rpc("lib:play-start", vec![tracks[2]["id"].clone(), json!(5)]).unwrap();
    assert!(listeners(&h.ami)["nil"].is_object());
    h.s.hub().presence.expire(&h.ids["nil"]);
    h.s.hub().presence.sweep(h.s.hub());
    assert_eq!(listeners(&h.ami)["nil"], Value::Null);

    // Couper l'option efface aussitôt.
    h.nil.rpc("lib:play-start", vec![tracks[0]["id"].clone(), json!(5)]).unwrap();
    assert!(listeners(&h.ami)["nil"].is_object());
    h.nil.rpc("social:set-listening", vec![json!(false)]).unwrap();
    assert_eq!(listeners(&h.ami)["nil"], Value::Null);
    h.nil.rpc("lib:play-start", vec![tracks[0]["id"].clone(), json!(5)]).unwrap();
    assert_eq!(listeners(&h.ami)["nil"], Value::Null, "option coupée : plus rien ne remonte");

    // L'administrateur ne voit pas ce qu'on n'a pas partagé avec lui.
    h.ami.rpc("social:set-listening", vec![json!(true)]).unwrap();
    assert_eq!(listeners(&h.nil)["ami"], Value::Null);
}

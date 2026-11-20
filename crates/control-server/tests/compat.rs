//! Un dossier de données écrit par la version Node (`tests/data/node-data` : server.db avec un administrateur
//! « nil », des réglages, un mot de passe d'application Subsonic scellé par server/secrets.js) se rouvre tel quel.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use common::{Client, PASSWORD};
use control_server::Background;
use control_server::config::Config;
use control_server::hub::HubOptions;
use serde_json::json;

#[test]
fn node_data_directory_is_reused_as_is() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/node-data");
    for name in ["server.db", "secret.key"] {
        std::fs::copy(fixture.join(name), data.join(name)).unwrap();
    }
    let mut config = Config::from_map(&HashMap::new());
    config.port = 0;
    config.host = "127.0.0.1".into();
    config.data_dir = data;
    let options = HubOptions { update_url: Some("http://127.0.0.1:9/releases/latest".into()), ..HubOptions::default() };
    let bg = Background::start(config, options).unwrap();
    let c = Client::new(&bg.url());

    let status = c.json("GET", "/api/setup/status", None).json();
    assert_eq!(
        (&status["initialized"], &status["serverName"], &status["registrationEnabled"]),
        (&json!(true), &json!("Chez Nil"), &json!(true))
    );
    // Empreinte scrypt faite par Node : le mot de passe est reconnu.
    assert_eq!(c.login("nil", "mauvais mot de passe").status, 401);
    let ok = c.login("nil", PASSWORD);
    assert_eq!((ok.status, &ok.json()["user"]["role"]), (200, &json!("admin")));
    assert_eq!(c.rpc("srv:settings:get", vec![]).unwrap()["uploadMaxMb"], 42);
    // Secret scellé par Node : la clé d'API et le jeton md5 fonctionnent.
    let ping = c.raw("GET", "/rest/ping?apiKey=appsecret123&c=x&f=json", None, &[], false).json();
    assert_eq!(ping["subsonic-response"]["status"], "ok");
    assert_eq!(c.rpc("srv:me:subsonic", vec![]).unwrap()["hasSecret"], true);
}

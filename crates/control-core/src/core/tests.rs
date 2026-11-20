use super::*;
use crate::model::emit_nothing;

/// Hôte de test : un dossier à analyser, et l'écho des réglages noté.
struct TestHost {
    roots: Vec<String>,
    echoed: Mutex<Vec<(String, String, Value)>>,
}

impl Host for TestHost {
    fn scan_roots(&self) -> Vec<String> {
        self.roots.clone()
    }

    fn emit_others(&self, sender: &str, channel: &str, payload: &Value) {
        self.echoed.lock().push((sender.to_string(), channel.to_string(), payload.clone()));
    }

    fn guard_url(&self, url: &str) -> Result<String, RpcError> {
        if url.starts_with("https://") {
            Ok(url.to_string())
        } else {
            Err(RpcError::new(400, "invalid_url", "refusé"))
        }
    }
}

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

fn setup() -> (tempfile::TempDir, Core, Arc<TestHost>) {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(music.join("A")).unwrap();
    std::fs::write(music.join("A").join("01 - Un.wav"), wav(1)).unwrap();
    std::fs::write(music.join("A").join("02 - Deux.wav"), wav(2)).unwrap();
    let host = Arc::new(TestHost { roots: vec![music.to_string_lossy().into_owned()], echoed: Mutex::new(Vec::new()) });
    let core = Core::new(CoreOptions {
        dir: dir.path().join("user"),
        covers_dir: None,
        tools: None,
        emit: emit_nothing(),
        default_download_dir: dir.path().join("dl"),
        tmdb_fallback: None,
        cookies_file: None,
        lastfm_keys: None,
        guard_url: None,
        host: host.clone(),
    })
    .unwrap();
    (dir, core, host)
}

#[test]
fn library_playlists_and_plays() {
    let (_dir, core, _host) = setup();
    let scan = core.dispatch("lib:scan", &[]).unwrap();
    assert_eq!(scan["added"], 2);
    let tracks = core.dispatch("lib:tracks", &[]).unwrap();
    let list = tracks.as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert!(list[0].get("sourceUrl").is_some() && list[0].get("mtime").is_none());
    let id = list[0]["id"].as_i64().unwrap();

    let pl = core.dispatch("pl:create", &[json!("Ma liste"), json!("manual"), Value::Null, json!([id])]).unwrap();
    let pl_id = pl.as_i64().unwrap();
    let patch = json!({ "description": "Pour la route", "shared": true });
    assert_eq!(core.dispatch("pl:update", &[pl, patch]).unwrap(), true);
    let pls = core.dispatch("lib:playlists", &[]).unwrap();
    assert_eq!(pls["playlists"][0]["description"], "Pour la route");
    assert_eq!(pls["items"].as_array().unwrap().len(), 1);
    assert_eq!(core.dispatch("pl:update", &[json!("x"), json!({})]).unwrap(), false);
    let refused = core.dispatch("pl:set-cover", &[json!(pl_id), json!({ "data": "pas une image" })]).unwrap();
    assert_eq!(refused["ok"], false);
    assert_eq!(core.dispatch("pl:set-cover", &[json!(pl_id), Value::Null]).unwrap()["cover"], Value::Null);

    let play = core.dispatch("lib:play-start", &[json!(id), json!(1)]).unwrap();
    assert!(play.as_i64().unwrap() > 0);
    assert_eq!(core.dispatch("lib:play-end", &[play, json!(1), json!(true)]).unwrap(), Value::Null);
    let today = crate::util::local_day(now_ms());
    let stats = core.dispatch("stats:get", &[json!({ "from": today, "to": today })]).unwrap();
    assert_eq!(stats["ok"], true);
    assert_eq!(stats["data"]["music"]["plays"], 1);
}

#[test]
fn settings_echo_and_errors() {
    let (_dir, core, host) = setup();
    core.dispatch_from("onglet-1", "settings:set", &[json!({ "volume": 0.4 })]).unwrap();
    assert_eq!(core.store.get("volume"), json!(0.4));
    assert_eq!(host.echoed.lock()[0], ("onglet-1".into(), "settings:changed".into(), json!({ "volume": 0.4 })));
    core.dispatch("settings:set", &[json!("pas un objet")]).unwrap();
    assert_eq!(host.echoed.lock().len(), 1);

    assert_eq!(core.dispatch("nimporte", &[]).unwrap_err().status, 404);
    assert_eq!(core.dispatch("lyrics:get", &[json!("x")]).unwrap()["message"], "Morceau invalide.");
    let err = core.dispatch("dl:add", &[json!("pas un objet")]).unwrap_err();
    assert_eq!((err.status, err.message.as_str()), (400, "Demande invalide."));
    assert_eq!(core.dispatch("dl:add", &[json!({ "url": "file:///etc/passwd" })]).unwrap_err().code, "invalid_url");
    assert_eq!(core.dispatch("dl:info", &[json!("http://127.0.0.1/")]).unwrap()["ok"], false);
    assert_eq!(core.dispatch("dl:list", &[]).unwrap(), json!([]));
    assert!(Core::has_channel("lib:play-end", true) && !Core::has_channel("lib:play-end", false));
    assert!(core.dispatch("watchlist:list", &[]).unwrap()["ok"].as_bool().unwrap());
}

#[test]
fn json_helpers_follow_javascript() {
    assert_eq!(int(&json!(3.0)), Some(3));
    assert_eq!(int(&json!(3.5)), None);
    assert_eq!(int(&json!("3")), None);
    assert!(truthy(&json!(1)) && !truthy(&json!("")) && !truthy(&Value::Null) && truthy(&json!({})));
    assert_eq!(text(&Value::Null), "");
    let tiny = image_from_data_url(&json!("data:image/png;base64,AAAA")).unwrap();
    assert!(!decodable_image(&tiny));
    assert!(image_from_data_url(&json!("data:text/plain;base64,AAAA")).is_none());
}

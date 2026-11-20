//! Outils communs aux essais du serveur : serveur d'essai, client HTTP à cookie, fichiers audio minuscules.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use control_server::Background;
use control_server::config::Config;
use control_server::hub::HubOptions;
use serde_json::{Value, json};

pub const PASSWORD: &str = "long enough password";

/// Un petit fichier WAV (silence) : de quoi être analysé et servi.
pub fn wav(file: &Path, seconds: u32) {
    let rate = 8000u32;
    let len = rate * seconds * 2;
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&len.to_le_bytes());
    v.resize(v.len() + len as usize, 0);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, v).unwrap();
}

pub struct Server {
    pub bg: Background,
    pub root: PathBuf,
    pub music: PathBuf,
    _dir: tempfile::TempDir,
}

impl Server {
    pub fn base(&self) -> String {
        self.bg.url()
    }

    pub fn hub(&self) -> &std::sync::Arc<control_server::hub::Hub> {
        &self.bg.hub
    }

    pub fn client(&self) -> Client {
        Client::new(&self.base())
    }
}

#[derive(Default)]
pub struct Boot {
    /// Pas de dossier commun : chaque compte n'a que ses dossiers.
    pub no_common: bool,
    pub setup_token: Option<String>,
    pub trust_proxy: bool,
    pub backup_any_url: bool,
}

pub fn boot(opts: Boot) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let music = root.join("music");
    wav(&music.join("Artist A").join("Album 1").join("01 - One.wav"), 1);
    wav(&music.join("Artist A").join("Album 1").join("02 - Two.wav"), 2);
    wav(&music.join("Artist B").join("03 - Three.wav"), 1);
    let mut config = Config::from_map(&HashMap::new());
    config.port = 0;
    config.host = "127.0.0.1".into();
    config.data_dir = root.join("data");
    config.library_folders = vec![music.to_string_lossy().into_owned()];
    config.rescan_minutes = 1000.0;
    config.trust_proxy = opts.trust_proxy;
    config.backup_any_url = opts.backup_any_url;
    if let Some(t) = opts.setup_token {
        config.setup_token = t;
    }
    // Jamais GitHub pendant les essais : la vérification périodique vise une adresse locale fermée.
    let options = HubOptions { update_url: Some("http://127.0.0.1:9/releases/latest".into()), ..HubOptions::default() };
    let bg = Background::start(config, options).unwrap();
    if !opts.no_common {
        bg.hub.patch_settings(&json!({ "libraryFolders": [music.to_string_lossy()] })).unwrap();
    }
    Server { bg, root, music, _dir: dir }
}

/// Une réponse lue en entier.
pub struct Resp {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

/// Erreur d'un appel RPC : statut, code, message.
#[derive(Debug, Clone)]
pub struct RpcErr {
    pub status: u16,
    pub code: String,
    pub message: String,
}

/// Client minimal : garde le cookie de session et envoie l'en-tête anti-CSRF.
pub struct Client {
    pub base: String,
    cookie: RefCell<String>,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(base: &str) -> Client {
        let agent: ureq::Agent =
            ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).build().into();
        Client { base: base.to_string(), cookie: RefCell::new(String::new()), agent }
    }

    pub fn cookie(&self) -> String {
        self.cookie.borrow().clone()
    }

    pub fn request(&self, method: &str, url: &str, body: Option<&Value>, extra: &[(&str, &str)]) -> Resp {
        self.raw(method, url, body.map(|b| (b.to_string().into_bytes(), "application/json")), extra, true)
    }

    /// Requête brute : corps et type, en-tête anti-CSRF ou non.
    pub fn raw(
        &self,
        method: &str,
        url: &str,
        body: Option<(Vec<u8>, &str)>,
        extra: &[(&str, &str)],
        csrf: bool,
    ) -> Resp {
        let mut b = ureq::http::Request::builder().method(method).uri(format!("{}{url}", self.base));
        if csrf {
            b = b.header("x-control", "1");
        }
        let cookie = self.cookie();
        if !cookie.is_empty() {
            b = b.header("cookie", cookie);
        }
        if let Some((_, kind)) = &body {
            b = b.header("content-type", *kind);
        }
        for (k, v) in extra {
            b = b.header(*k, *v);
        }
        let data = body.map(|(d, _)| d).unwrap_or_default();
        let req = b.body(data).unwrap();
        let mut res = self.agent.run(req).expect("requête");
        let mut headers = HashMap::new();
        for (k, v) in res.headers() {
            let v = v.to_str().unwrap_or("").to_string();
            headers
                .entry(k.as_str().to_string())
                .and_modify(|e: &mut String| e.push_str(&format!(", {v}")))
                .or_insert(v);
        }
        if let Some(set) = headers.get("set-cookie") {
            let first = set.split(';').next().unwrap_or("");
            *self.cookie.borrow_mut() = if first.ends_with('=') { String::new() } else { first.to_string() };
        }
        let body = res.body_mut().with_config().limit(100 * 1024 * 1024).read_to_vec().unwrap_or_default();
        Resp { status: res.status().as_u16(), headers, body }
    }

    pub fn json(&self, method: &str, url: &str, body: Option<Value>) -> Resp {
        self.request(method, url, body.as_ref(), &[])
    }

    pub fn post(&self, url: &str, body: Value) -> Resp {
        self.request("POST", url, Some(&body), &[])
    }

    fn call(&self, channel: &str, args: Vec<Value>, extra: &[(&str, &str)]) -> Result<Value, RpcErr> {
        let url = format!("/api/rpc/{}", urlencode(channel));
        let r = self.request("POST", &url, Some(&json!({ "args": args })), extra);
        let body = r.json();
        if r.status != 200 {
            return Err(RpcErr {
                status: r.status,
                code: body["error"]["code"].as_str().unwrap_or("").into(),
                message: body["error"]["message"].as_str().unwrap_or("").into(),
            });
        }
        Ok(body["result"].clone())
    }

    pub fn rpc(&self, channel: &str, args: Vec<Value>) -> Result<Value, RpcErr> {
        self.call(channel, args, &[])
    }

    /// Appel dans la bibliothèque d'un autre compte (en-tête x-library).
    pub fn rpc_in(&self, library: &str, channel: &str, args: Vec<Value>) -> Result<Value, RpcErr> {
        self.call(channel, args, &[("x-library", library)])
    }

    pub fn send(&self, channel: &str, args: Vec<Value>) -> Resp {
        self.request("POST", &format!("/api/send/{}", urlencode(channel)), Some(&json!({ "args": args })), &[])
    }

    /// Crée le premier compte (administrateur).
    pub fn setup(&self, body: Value) -> Resp {
        self.post("/api/setup/init", body)
    }

    pub fn login(&self, username: &str, password: &str) -> Resp {
        self.post("/api/auth/login", json!({ "username": username, "password": password }))
    }

    pub fn me(&self) -> Value {
        self.json("GET", "/api/auth/me", None).json()["user"].clone()
    }

    /// L'état de démarrage de la page d'accueil.
    pub fn boot_state(&self, path: &str) -> Value {
        let page = self.request("GET", path, None, &[]).text();
        let start = page.find("<script id=\"boot\" type=\"application/json\">").expect("état de démarrage") + 42;
        let end = page[start..].find("</script>").unwrap() + start;
        serde_json::from_str(&page[start..end]).unwrap()
    }

    pub fn tracks(&self) -> Vec<Value> {
        self.rpc("lib:tracks", vec![]).unwrap().as_array().cloned().unwrap_or_default()
    }

    /// Attend que la bibliothèque compte `n` morceaux.
    pub fn wait_tracks(&self, n: usize) -> Vec<Value> {
        until(|| {
            let t = self.tracks();
            (t.len() == n).then_some(t)
        })
    }
}

pub fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>().replace('+', "%20")
}

/// Attend qu'une condition soit remplie (15 s au plus).
pub fn until<T>(mut f: impl FnMut() -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < end, "délai dépassé");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Requête HTTP écrite à la main (chemins que les clients normalisent, comme `/js/../x`) : renvoie le statut.
pub fn raw_status(base: &str, path: &str, cookie: &str) -> u16 {
    let addr = base.trim_start_matches("http://");
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: {addr}\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n").unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0)
}

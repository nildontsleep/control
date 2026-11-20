//! Mise à jour de l'application : cherche la dernière release GitHub, télécharge l'installeur (ou l'exécutable
//! portable, ou l'AppImage) en vérifiant son empreinte SHA-256, puis l'installe et relance control.
//!
//! États (`update:state`, même forme JSON que l'original ; `current` toujours présent) :
//! `idle` → (`checking`) → `none` | `available` → `downloading` → `ready` → `installing`, et `error` (`code`).
//! Le paquet .deb (et toute installation sans fichier de remplacement) est seulement prévenu : `available` avec
//! `manual: true` et la page de la release.
//!
//! Toutes les méthodes sont bloquantes ; [`Updater::spawn_checks`] cherche au lancement puis toutes les 6 heures.

mod install;

use std::cmp::Ordering;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

pub use install::{InstallPlan, appimage_plan, portable_plan, portable_script, setup_plan};

use crate::model::{Emit, Event};
use crate::util::now_ms;

pub const REPO: &str = "nildontsleep/control";
pub const STATE_CHANNEL: &str = "update:state";
const AGENT: &str = "control-updater (https://github.com/nildontsleep/control)";
const FIRST_CHECK: Duration = Duration::from_secs(12);
const EVERY: Duration = Duration::from_secs(6 * 3600);

static STALE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-(\d+\.\d+\.\d+)-(?:setup\.exe|portable\.exe|x86_64\.AppImage)(\.part)?$").expect("regex constante")
});
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/releases/tag/([^/?#]+)").expect("regex constante"));

/// Adresse de l'API (remplaçable pour les essais : `CONTROL_UPDATE_URL`).
fn api_url() -> String {
    std::env::var("CONTROL_UPDATE_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("https://api.github.com/repos/{REPO}/releases/latest"))
}

fn test_override() -> bool {
    std::env::var_os("CONTROL_UPDATE_URL").is_some_and(|v| !v.is_empty())
}

/// Compare deux versions « 1.2.3 » (le suffixe de préversion et un « v » initial sont ignorés).
pub fn compare(a: &str, b: &str) -> Ordering {
    fn parts(v: &str) -> Vec<i64> {
        let v = v.trim();
        let v = v.strip_prefix(['v', 'V']).unwrap_or(v);
        v.split('-').next().unwrap_or("").split('.').map(|n| n.trim().parse::<i64>().unwrap_or(0)).collect()
    }
    let (x, y) = (parts(a), parts(b));
    for i in 0..x.len().max(y.len()) {
        let d = x.get(i).copied().unwrap_or(0).cmp(&y.get(i).copied().unwrap_or(0));
        if d != Ordering::Equal {
            return d;
        }
    }
    Ordering::Equal
}

/// Façon dont control est installé, qui décide du fichier de la release qui le remplace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edition {
    /// Installeur Windows (`control-<v>-setup.exe`).
    Setup,
    /// Exécutable portable Windows (`control-<v>-portable.exe`).
    Portable,
    /// AppImage Linux (`control-<v>-x86_64.AppImage`).
    AppImage,
    /// Paquet Debian (`control-<v>-amd64.deb`) : installé par le gestionnaire de paquets, on prévient seulement.
    Deb,
    /// Autre (lancement depuis les sources, macOS…) : on prévient seulement.
    Manual,
}

impl Edition {
    pub fn as_str(self) -> &'static str {
        match self {
            Edition::Setup => "setup",
            Edition::Portable => "portable",
            Edition::AppImage => "appimage",
            Edition::Deb => "deb",
            Edition::Manual => "manual",
        }
    }

    pub fn parse(s: &str) -> Edition {
        match s {
            "setup" => Edition::Setup,
            "portable" => Edition::Portable,
            "appimage" => Edition::AppImage,
            "deb" => Edition::Deb,
            _ => Edition::Manual,
        }
    }

    /// Devine l'édition en cours d'après l'exécutable et l'environnement.
    pub fn detect(exe: &Path) -> Edition {
        let name = exe.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        if cfg!(windows) {
            if name.contains("portable") || std::env::var_os("CONTROL_PORTABLE").is_some() {
                Edition::Portable
            } else {
                Edition::Setup
            }
        } else if cfg!(target_os = "linux") {
            if std::env::var_os("APPIMAGE").is_some() {
                Edition::AppImage
            } else if exe.starts_with("/usr") || exe.starts_with("/opt") {
                Edition::Deb
            } else {
                Edition::Manual
            }
        } else {
            Edition::Manual
        }
    }

    /// Fin du nom du fichier de la release qui remplace cette édition (`None` : rien à installer tout seul).
    pub fn asset_suffix(self) -> Option<&'static str> {
        match self {
            Edition::Setup => Some("-setup.exe"),
            Edition::Portable => Some("-portable.exe"),
            Edition::AppImage => Some("-x86_64.AppImage"),
            Edition::Deb | Edition::Manual => None,
        }
    }
}

/// Fichiers d'une release, tels que les donne l'API GitHub.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: Option<String>,
    /// Adresse de l'API (dépôt privé, avec jeton).
    pub url: Option<String>,
    /// `sha256:<hex>`.
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct Release {
    pub tag_name: String,
    pub name: Option<String>,
    pub body: Option<String>,
    pub html_url: Option<String>,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<Asset>,
}

/// Choisit le bon fichier de la release, selon l'édition en cours (insensible à la casse, comme l'original).
pub fn pick_asset(assets: &[Asset], edition: Edition) -> Option<&Asset> {
    let suffix = edition.asset_suffix()?.to_lowercase();
    assets.iter().find(|a| a.name.to_lowercase().ends_with(&suffix) && a.browser_download_url.is_some())
}

/// Noms prévisibles des fichiers d'une version (pour la recherche sans l'API).
pub fn asset_names(version: &str) -> Vec<String> {
    ["setup.exe", "portable.exe", "x86_64.AppImage", "amd64.deb"].iter().map(|s| format!("control-{version}-{s}")).collect()
}

/// Code d'erreur d'une réponse de l'API. Sans jeton, GitHub répond 404 pour un dépôt privé ; avec un jeton,
/// 401/404 veulent dire qu'il est refusé. 403/429 sans jeton : limite des requêtes anonymes.
pub fn api_error_code(status: u16, authed: bool) -> &'static str {
    match status {
        404 if authed => "token",
        404 => "private",
        401 => "token",
        403 | 429 if authed => "token",
        403 | 429 => "busy",
        _ => "http",
    }
}

/// Empreinte SHA-256 annoncée par GitHub (`sha256:<64 hex>`), en minuscules.
pub fn asset_sha256(digest: Option<&str>) -> Option<String> {
    let d = digest?.trim().to_lowercase();
    let hex = d.strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hex.to_string())
}

/// Adresse de téléchargement digne de confiance (fichiers des releases du dépôt, ou API du dépôt).
pub fn trusted_url(url: &str) -> bool {
    let download = format!("https://github.com/{REPO}/releases/download/");
    let api = format!("https://api.github.com/repos/{REPO}/releases/assets/");
    url.starts_with(&download)
        || url.strip_prefix(&api).is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
}

/// Redirection de l'API acceptable : https, vers GitHub.
pub fn trusted_redirect(target: &str) -> bool {
    let Ok(u) = url::Url::parse(target) else { return false };
    let host = u.host_str().unwrap_or_default().to_lowercase();
    u.scheme() == "https"
        && ["github.com", "githubusercontent.com"].iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// Tag désigné par la redirection de `releases/latest`.
pub fn tag_from_location(location: &str) -> Option<String> {
    TAG.captures(location).map(|c| c[1].to_string())
}

/// Fichier d'un ancien téléchargement à supprimer : version ≤ version en cours, ou téléchargement interrompu.
pub fn stale_download(name: &str, current: &str) -> bool {
    STALE.captures(name).is_some_and(|c| c.get(2).is_some() || compare(&c[1], current) != Ordering::Greater)
}

// ─── Updater ─────────────────────────────────────────────────────────────────

/// Jeton GitHub en lecture (dépôt privé), lu à chaque requête.
pub type TokenFn = Arc<dyn Fn() -> String + Send + Sync>;

pub struct UpdaterOptions {
    /// Version en cours.
    pub version: String,
    pub edition: Edition,
    /// Dossier de travail pour les téléchargements.
    pub dir: PathBuf,
    pub emit: Emit,
    /// Exécutable à remplacer : l'AppImage (`$APPIMAGE`) ou l'exécutable portable.
    pub self_file: Option<PathBuf>,
    pub token: Option<TokenFn>,
}

/// La release à installer.
#[derive(Debug, Clone, PartialEq)]
struct Pending {
    version: String,
    url: String,
    size: u64,
    digest: Option<String>,
    file_name: String,
    /// Téléchargement par l'API (dépôt privé).
    api: bool,
}

/// Échec, avec son code pour l'interface.
#[derive(Debug)]
struct Fail(&'static str);

impl From<std::io::Error> for Fail {
    fn from(_: std::io::Error) -> Fail {
        Fail("network")
    }
}

impl From<ureq::Error> for Fail {
    fn from(_: ureq::Error) -> Fail {
        Fail("network")
    }
}

struct Inner {
    o: UpdaterOptions,
    agent: ureq::Agent,
    /// Pour lire une redirection sans la suivre.
    manual: ureq::Agent,
    state: Mutex<Value>,
    release: Mutex<Option<Pending>>,
    file: Mutex<Option<PathBuf>>,
    busy: AtomicBool,
}

/// Ce que l'application doit faire après [`Updater::install`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallOutcome {
    /// La nouvelle version est lancée et attend : l'application doit se fermer maintenant.
    Quit,
    /// Rien n'a été lancé (voir l'état : erreur, ou essai `CONTROL_UPDATE_DRYRUN`).
    Stay,
}

#[derive(Clone)]
pub struct Updater {
    inner: Arc<Inner>,
}

impl Updater {
    pub fn new(o: UpdaterOptions) -> Updater {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).user_agent(AGENT).build().into();
        let manual: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .user_agent(AGENT)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(15)))
            .build()
            .into();
        let state = json!({ "phase": "idle", "current": o.version });
        Updater {
            inner: Arc::new(Inner {
                o,
                agent,
                manual,
                state: Mutex::new(state),
                release: Mutex::new(None),
                file: Mutex::new(None),
                busy: AtomicBool::new(false),
            }),
        }
    }

    pub fn edition(&self) -> Edition {
        self.inner.o.edition
    }

    /// État actuel (`update:state`).
    pub fn status(&self) -> Value {
        self.inner.state.lock().clone()
    }

    fn phase(&self) -> String {
        self.inner.state.lock().get("phase").and_then(Value::as_str).unwrap_or("idle").to_string()
    }

    fn set(&self, patch: Value) {
        let mut m = Map::new();
        m.insert("current".into(), json!(self.inner.o.version));
        if let Value::Object(p) = patch {
            m.extend(p);
        }
        let v = Value::Object(m);
        *self.inner.state.lock() = v.clone();
        (self.inner.o.emit)(Event::Json(STATE_CHANNEL, v));
    }

    fn token(&self) -> String {
        self.inner.o.token.as_ref().map(|f| f().trim().to_string()).unwrap_or_default()
    }

    fn get(&self, agent: &ureq::Agent, url: &str, headers: &[(&str, String)], timeout: Option<Duration>) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
        let mut req = agent.get(url);
        for (k, v) in headers {
            req = req.header(*k, v.as_str());
        }
        req.config().timeout_global(timeout).build().call()
    }

    fn auth(&self) -> Vec<(&'static str, String)> {
        let token = self.token();
        if token.is_empty() { Vec::new() } else { vec![("authorization", format!("Bearer {token}"))] }
    }

    fn api(&self) -> Result<Release, Fail> {
        let mut headers = vec![("accept", "application/vnd.github+json".to_string())];
        headers.extend(self.auth());
        let authed = headers.len() > 1;
        let mut res = self.get(&self.inner.agent, &api_url(), &headers, Some(Duration::from_secs(12)))?;
        let status = res.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(Fail(api_error_code(status, authed)));
        }
        res.body_mut().read_json::<Release>().map_err(|_| Fail("http"))
    }

    /// Adresse de redirection (sans la suivre), ou `None` si la réponse n'en est pas une.
    fn redirect(&self, url: &str, headers: &[(&str, String)]) -> Result<Option<String>, Fail> {
        let res = self.get(&self.inner.manual, url, headers, Some(Duration::from_secs(15)))?;
        if !res.status().is_redirection() {
            return Ok(None);
        }
        Ok(res.headers().get("location").and_then(|h| h.to_str().ok()).map(str::to_string))
    }

    /// Sans l'API : `releases/latest` redirige vers la page du dernier tag, et les fichiers portent un nom prévisible.
    fn web(&self) -> Result<Release, Fail> {
        let location = self.redirect(&format!("https://github.com/{REPO}/releases/latest"), &[])?;
        let tag = location.as_deref().and_then(tag_from_location).ok_or(Fail("http"))?;
        let decoded = url::form_urlencoded::parse(format!("t={tag}").as_bytes())
            .next()
            .map(|(_, v)| v.into_owned())
            .unwrap_or_else(|| tag.clone());
        let version = decoded.trim_start_matches(['v', 'V']).to_string();
        let mut assets = Vec::new();
        for name in asset_names(&version) {
            let url = format!("https://github.com/{REPO}/releases/download/{tag}/{name}");
            let head = self.inner.agent.head(&url).config().timeout_global(Some(Duration::from_secs(12))).build().call();
            if let Ok(res) = head
                && res.status().is_success()
            {
                let size = res
                    .headers()
                    .get("content-length")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                assets.push(Asset { name, size, browser_download_url: Some(url), url: None, digest: None });
            }
        }
        Ok(Release {
            tag_name: tag.clone(),
            name: Some(format!("v{version}")),
            body: Some(String::new()),
            html_url: Some(format!("https://github.com/{REPO}/releases/tag/{tag}")),
            draft: false,
            prerelease: false,
            assets,
        })
    }

    /// Cherche une nouvelle version. `manual` : un échec réseau est signalé au lieu d'être passé sous silence.
    pub fn check(&self, manual: bool) -> Value {
        if matches!(self.phase().as_str(), "downloading" | "installing") {
            return self.status();
        }
        if self.inner.busy.swap(true, AtomicOrdering::SeqCst) {
            return self.status();
        }
        if manual {
            self.set(json!({ "phase": "checking" }));
        }
        if let Err(Fail(code)) = self.check_inner() {
            if manual {
                self.set(json!({ "phase": "error", "code": code, "checkedAt": now_ms() }));
            } else if self.phase() == "checking" {
                self.set(json!({ "phase": "idle" }));
            }
        }
        self.inner.busy.store(false, AtomicOrdering::SeqCst);
        self.status()
    }

    fn check_inner(&self) -> Result<(), Fail> {
        let rel = match if std::env::var_os("CONTROL_UPDATE_WEBONLY").is_some() { Err(Fail("busy")) } else { self.api() } {
            Ok(r) => r,
            // L'API GitHub limite les requêtes anonymes (60 par heure et par adresse) : on passe par les pages publiques.
            Err(Fail("busy")) if !test_override() => self.web()?,
            Err(e) => return Err(e),
        };
        let (state, pending) = evaluate(&rel, &self.inner.o.version, self.inner.o.edition, !self.token().is_empty())?;
        *self.inner.release.lock() = pending;
        let mut state = state;
        state["checkedAt"] = json!(now_ms());
        self.set(state);
        Ok(())
    }

    /// Télécharge le fichier de la mise à jour et vérifie son empreinte.
    pub fn download(&self) -> Value {
        let Some(rel) = self.inner.release.lock().clone() else { return self.status() };
        {
            let state = self.inner.state.lock();
            if state.get("phase").and_then(Value::as_str) == Some("downloading") {
                return state.clone();
            }
        }
        let file = self.inner.o.dir.join(&rel.file_name);
        let mut part = file.as_os_str().to_owned();
        part.push(".part");
        let part = PathBuf::from(part);
        self.set(json!({ "phase": "downloading", "version": rel.version, "size": rel.size, "received": 0 }));
        match self.fetch(&rel, &file, &part) {
            Ok(total) => {
                *self.inner.file.lock() = Some(file);
                self.set(json!({ "phase": "ready", "version": rel.version, "size": total, "verified": rel.digest.is_some() }));
            }
            Err(Fail(code)) => {
                let _ = std::fs::remove_file(&part);
                self.set(json!({ "phase": "error", "code": code, "version": rel.version }));
            }
        }
        self.status()
    }

    fn fetch(&self, rel: &Pending, file: &Path, part: &Path) -> Result<u64, Fail> {
        std::fs::create_dir_all(&self.inner.o.dir)?;
        let mut url = rel.url.clone();
        let mut headers: Vec<(&str, String)> = Vec::new();
        if rel.api {
            // Dépôt privé : l'API répond par une redirection vers une adresse signée. Le jeton ne doit jamais la
            // suivre : on lit la redirection, puis on télécharge sans jeton.
            let mut h = vec![("accept", "application/octet-stream".to_string())];
            h.extend(self.auth());
            match self.redirect(&rel.url, &h)? {
                Some(target) => {
                    if !trusted_redirect(&target) && !test_override() {
                        return Err(Fail("untrusted"));
                    }
                    url = target;
                }
                None => headers = h,
            }
        }
        let res = self.get(&self.inner.agent, &url, &headers, None)?;
        if !res.status().is_success() {
            return Err(Fail("http"));
        }
        let total = res
            .headers()
            .get("content-length")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(rel.size);
        let mut reader = res.into_body().into_reader();
        let mut out = std::fs::File::create(part)?;
        let mut hash = Sha256::new();
        let mut received: u64 = 0;
        let mut last = Instant::now();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(Fail("network")),
            };
            hash.update(&buf[..n]);
            out.write_all(&buf[..n])?;
            received += n as u64;
            if last.elapsed() > Duration::from_millis(200) {
                last = Instant::now();
                self.set(json!({ "phase": "downloading", "version": rel.version, "size": total, "received": received }));
            }
        }
        out.sync_all()?;
        drop(out);
        if rel.size > 0 && received != rel.size {
            return Err(Fail("corrupt"));
        }
        if rel.digest.as_ref().is_some_and(|d| *d != hex::encode(hash.finalize())) {
            return Err(Fail("corrupt"));
        }
        std::fs::rename(part, file)?;
        Ok(total)
    }

    /// Ce qu'il faudrait lancer pour installer le fichier téléchargé (`None` : impossible ici).
    pub fn plan(&self) -> Option<InstallPlan> {
        let file = self.inner.file.lock().clone()?;
        let rel = self.inner.release.lock().clone()?;
        let dry = std::env::var_os("CONTROL_UPDATE_DRYRUN").is_some();
        let writable = |p: &Path| p.parent().is_some_and(dir_writable);
        match self.inner.o.edition {
            Edition::Setup => Some(setup_plan(&file)),
            Edition::Portable => {
                if !cfg!(windows) && !dry {
                    return None;
                }
                // Dossier protégé : mieux vaut le dire que fermer l'application pour rien.
                let old = self.inner.o.self_file.clone().filter(|p| writable(p))?;
                let dest = old.parent()?.join(&rel.file_name);
                let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into());
                Some(portable_plan(&old, &file, &dest, &self.inner.o.dir.join("apply-update.cmd"), &comspec))
            }
            Edition::AppImage => {
                if !cfg!(target_os = "linux") && !dry {
                    return None;
                }
                // Dossier protégé (installé pour tous) : mise à jour manuelle.
                let target = self.inner.o.self_file.clone().filter(|p| writable(p))?;
                Some(appimage_plan(&target, &file, std::process::id()))
            }
            Edition::Deb | Edition::Manual => None,
        }
    }

    /// Lance l'installation : l'application doit ensuite se fermer ([`InstallOutcome::Quit`]) pour que la nouvelle
    /// version prenne sa place et se relance.
    pub fn install(&self) -> InstallOutcome {
        let file = self.inner.file.lock().clone();
        let rel = self.inner.release.lock().clone();
        let (Some(file), true) = (file, self.phase() == "ready") else { return InstallOutcome::Stay };
        let version = rel.as_ref().map(|r| r.version.clone());
        // Le fichier a pu être modifié depuis la vérification : on revérifie avant de l'exécuter.
        if let Some(digest) = rel.as_ref().and_then(|r| r.digest.clone())
            && sha256_file(&file).ok().as_deref() != Some(digest.as_str())
        {
            let _ = std::fs::remove_file(&file);
            self.set(json!({ "phase": "error", "code": "corrupt", "version": version }));
            return InstallOutcome::Stay;
        }
        let Some(plan) = self.plan() else {
            self.set(json!({ "phase": "error", "code": "unsupported", "version": version }));
            return InstallOutcome::Stay;
        };
        self.set(json!({ "phase": "installing", "version": version }));
        if let Ok(dry) = std::env::var("CONTROL_UPDATE_DRYRUN")
            && !dry.is_empty()
        {
            // Essais : on note la commande au lieu de l'exécuter.
            let note = json!({ "command": plan.command, "args": plan.args, "script": plan.script.as_ref().map(|s| &s.1) });
            let _ = std::fs::write(dry, serde_json::to_string_pretty(&note).unwrap_or_default());
            return InstallOutcome::Stay;
        }
        match plan.run() {
            Ok(()) => InstallOutcome::Quit,
            Err(err) => {
                log::warn!("Mise à jour impossible à lancer : {err}");
                self.set(json!({ "phase": "error", "code": "launch", "version": version }));
                InstallOutcome::Stay
            }
        }
    }

    /// Nettoie les anciens téléchargements (au démarrage, une fois la nouvelle version lancée).
    pub fn cleanup(&self) {
        let Ok(entries) = std::fs::read_dir(&self.inner.o.dir) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if stale_download(&name, &self.inner.o.version) {
                let _ = std::fs::remove_file(e.path());
            }
        }
        let _ = std::fs::remove_file(self.inner.o.dir.join("apply-update.cmd"));
    }

    /// Cherche une mise à jour peu après le lancement, puis toutes les 6 heures, si `auto()` le permet
    /// (réglage `update.auto`). Le thread s'arrête quand le dernier `Updater` est libéré.
    pub fn spawn_checks(&self, auto: impl Fn() -> bool + Send + 'static) {
        let weak = Arc::downgrade(&self.inner);
        let spawned = std::thread::Builder::new().name("control-updater".into()).spawn(move || {
            let mut next = Instant::now() + FIRST_CHECK;
            loop {
                while Instant::now() < next {
                    std::thread::sleep(next.saturating_duration_since(Instant::now()).min(Duration::from_secs(30)));
                    if weak.strong_count() == 0 {
                        return;
                    }
                }
                let Some(inner) = weak.upgrade() else { return };
                if auto() {
                    Updater { inner }.check(false);
                }
                next = Instant::now() + EVERY;
            }
        });
        if let Err(err) = spawned {
            log::warn!("Thread des mises à jour impossible : {err}");
        }
    }
}

/// Décide de l'état après une recherche : (état à publier sans `checkedAt`, release à installer).
fn evaluate(rel: &Release, current: &str, edition: Edition, authed: bool) -> Result<(Value, Option<Pending>), Fail> {
    let latest = rel.tag_name.trim().trim_start_matches(['v', 'V']).to_string();
    if latest.is_empty() || rel.draft || rel.prerelease || compare(&latest, current) != Ordering::Greater {
        return Ok((json!({ "phase": "none" }), None));
    }
    let name = rel.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| format!("v{latest}"));
    let notes: String = rel.body.clone().unwrap_or_default().chars().take(4000).collect();
    let page = rel.html_url.clone();
    if edition.asset_suffix().is_none() {
        // Rien à installer tout seul (paquet .deb…) : on signale la nouvelle version et on renvoie vers sa page.
        let state = json!({ "phase": "available", "manual": true, "version": latest, "name": name, "notes": notes, "page": page });
        return Ok((state, None));
    }
    let asset = pick_asset(&rel.assets, edition).ok_or(Fail("asset"))?;
    // Dépôt privé : le téléchargement passe par l'API (avec le jeton) ; sinon par l'adresse publique.
    let url = match (&asset.url, &asset.browser_download_url) {
        (Some(api), _) if authed => api.clone(),
        (_, Some(public)) => public.clone(),
        _ => return Err(Fail("asset")),
    };
    if !trusted_url(&url) && !test_override() {
        return Err(Fail("untrusted"));
    }
    let pending = Pending {
        version: latest.clone(),
        api: asset.url.as_deref() == Some(url.as_str()),
        url,
        size: asset.size,
        digest: asset_sha256(asset.digest.as_deref()),
        file_name: asset.name.clone(),
    };
    let state = json!({ "phase": "available", "version": latest, "name": name, "notes": notes, "page": page, "size": asset.size });
    Ok((state, Some(pending)))
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(hex::encode(h.finalize()))
}

fn dir_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".control-write-{}", std::process::id()));
    let ok = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

#[cfg(test)]
mod tests;

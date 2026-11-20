//! Mise à jour automatique du serveur. Elle cherche la dernière release GitHub, télécharge l'archive du serveur
//! pour cette plateforme (`control-server-<version>-<os>-<arch>.tgz` : le binaire et les fichiers de la page),
//! vérifie son empreinte SHA-256 (et sa signature si des clés de confiance existent), l'installe à côté de la version
//! en cours (`<données>/app/versions/<version>`), puis demande au lanceur (`control-server --launcher`) de redémarrer
//! le serveur sur cette version. Le lanceur revient à la précédente si la nouvelle ne démarre pas.
//!
//! Trois modes : « off » (rien), « notify » (on prévient, l'administrateur installe), « auto » (téléchargement
//! automatique, installation dès que plus personne n'est connecté, ou à l'heure choisie). Mêmes états visibles dans
//! /admin que l'original (`phase`, `code`, `reason`…).

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::appstate;
use crate::tar::{self, Limits};
use crate::util::now_ms;

const REPO: &str = "nildontsleep/control";
const AGENT: &str = "control-web-updater (https://github.com/nildontsleep/control)";
const MAX_ARCHIVE: u64 = 150 * 1024 * 1024;
const CHECK_EVERY_MS: i64 = 6 * 60 * 60 * 1000;
const KEEP_BACKUPS: usize = 3;

/// Plateforme de ce binaire (`linux-x86_64`, `windows-x86_64`…) : l'archive téléchargée doit être la même.
pub fn target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Nom de l'archive d'une version pour cette plateforme.
pub fn asset_name(version: &str) -> String {
    format!("control-server-{version}-{}.tgz", target())
}

/// Réglages de la mise à jour (relus à chaque fois : ils peuvent changer à tout moment).
#[derive(Debug, Clone)]
pub struct UpdateConfig {
    pub mode: String,
    pub hour: i64,
    pub token: String,
    pub locked: bool,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        UpdateConfig { mode: "notify".into(), hour: 4, token: String::new(), locked: false }
    }
}

pub type Callback = Arc<dyn Fn() + Send + Sync>;

pub struct UpdaterOptions {
    pub version: String,
    pub data_dir: PathBuf,
    /// Adresse de `releases/latest` ; sinon `CONTROL_UPDATE_URL`, sinon GitHub. Une adresse imposée lève le contrôle
    /// des hôtes de confiance (essais).
    pub api_url: Option<String>,
    /// Clés publiques de confiance ; sinon celles de `update-key.json`.
    pub public_keys: Option<Vec<String>>,
    pub settings: Arc<dyn Fn() -> UpdateConfig + Send + Sync>,
    /// Demande au lanceur de redémarrer le serveur ; `None` : lancé sans lanceur (on ne peut que prévenir).
    pub restart: Option<Callback>,
    /// Copie de la base du serveur avant de changer de version.
    pub backup: Arc<dyn Fn() -> anyhow::Result<()> + Send + Sync>,
    /// Plus personne n'est connecté ?
    pub idle: Arc<dyn Fn() -> bool + Send + Sync>,
    pub on_state: Arc<dyn Fn(&Value) + Send + Sync>,
}

/// La version trouvée, prête à télécharger.
#[derive(Debug, Clone)]
struct Release {
    info: Map<String, Value>,
    version: String,
    asset: Located,
    size: u64,
    digest: String,
    signature: Option<Located>,
}

#[derive(Debug, Clone)]
struct Located {
    url: String,
    api: bool,
}

struct Failure {
    code: &'static str,
    detail: Option<String>,
}

fn fail(code: &'static str) -> Failure {
    Failure { code, detail: None }
}

fn fail_with(code: &'static str, detail: impl Into<String>) -> Failure {
    Failure { code, detail: Some(detail.into()) }
}

pub struct Updater {
    o: UpdaterOptions,
    keys: Vec<String>,
    state: Mutex<Map<String, Value>>,
    release: Mutex<Option<Release>>,
    busy: AtomicBool,
    stopped: Arc<AtomicBool>,
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn agent(redirects: u32, timeout: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(redirects)
        .timeout_global(timeout)
        .build()
        .into()
}

impl Updater {
    pub fn new(o: UpdaterOptions) -> Updater {
        let keys = o.public_keys.clone().unwrap_or_else(crate::sign::trusted_keys);
        let u = Updater {
            o,
            keys,
            state: Mutex::new(Map::new()),
            release: Mutex::new(None),
            busy: AtomicBool::new(false),
            stopped: Arc::new(AtomicBool::new(false)),
        };
        u.set(json!({ "phase": "idle" }));
        u
    }

    pub fn supervised(&self) -> bool {
        self.o.restart.is_some()
    }

    fn config(&self) -> UpdateConfig {
        let mut c = (self.o.settings)();
        c.hour = c.hour.clamp(0, 23);
        c
    }

    fn set(&self, patch: Value) -> Value {
        let c = self.config();
        let failure = appstate::read_state(&self.o.data_dir).get("failure").cloned().unwrap_or(Value::Null);
        let mut state = json!({
            "current": self.o.version, "mode": c.mode, "hour": c.hour, "locked": c.locked,
            "supervised": self.supervised(), "signed": !self.keys.is_empty(), "failure": failure,
        })
        .as_object()
        .cloned()
        .unwrap_or_default();
        if let Value::Object(p) = patch {
            state.extend(p);
        }
        let value = Value::Object(state.clone());
        *self.state.lock() = state;
        (self.o.on_state)(&value);
        value
    }

    pub fn status(&self) -> Value {
        Value::Object(self.state.lock().clone())
    }

    fn phase(&self) -> String {
        self.state.lock().get("phase").and_then(Value::as_str).unwrap_or("idle").to_string()
    }

    /// Les réglages ont changé : l'état montré aux clients suit.
    pub fn refresh(&self) {
        let mut rest = self.state.lock().clone();
        for k in ["current", "mode", "hour", "locked", "supervised", "failure"] {
            rest.remove(k);
        }
        self.set(Value::Object(rest));
    }

    fn api_url(&self) -> (String, bool) {
        if let Some(u) = &self.o.api_url {
            return (u.clone(), true);
        }
        match std::env::var("CONTROL_UPDATE_URL") {
            Ok(u) if !u.is_empty() => (u, true),
            _ => (format!("https://api.github.com/repos/{REPO}/releases/latest"), false),
        }
    }

    fn auth(&self) -> Option<String> {
        let token = self.config().token.trim().to_string();
        (!token.is_empty()).then(|| format!("Bearer {token}"))
    }

    fn fetch_release(&self) -> Result<Value, Failure> {
        let (url, _) = self.api_url();
        let mut req = agent(5, Some(Duration::from_secs(12)))
            .get(&url)
            .header("user-agent", AGENT)
            .header("accept", "application/vnd.github+json");
        let authed = self.auth();
        if let Some(a) = &authed {
            req = req.header("authorization", a);
        }
        let mut res = req.call().map_err(|_| fail("network"))?;
        let status = res.status().as_u16();
        if !(200..300).contains(&status) {
            // Sans jeton, GitHub répond 404 pour un dépôt privé ; avec un jeton, 401/404 veulent dire qu'il est refusé.
            let code = match status {
                404 if authed.is_some() => "token",
                404 => "private",
                401 => "token",
                403 | 429 if authed.is_some() => "token",
                403 | 429 => "busy",
                _ => "http",
            };
            return Err(fail(code));
        }
        res.body_mut().read_json::<Value>().map_err(|_| fail("network"))
    }

    /// Cherche une nouvelle version. `manual` : demandé par l'administrateur (l'échec est signalé, et une version déjà
    /// abandonnée reste proposée) ; sinon c'est la vérification périodique, silencieuse.
    pub fn check(&self, manual: bool) -> Value {
        let phase = self.phase();
        if self.busy.load(Ordering::SeqCst) || phase == "downloading" || phase == "installing" {
            return self.status();
        }
        if !manual && (self.config().mode == "off" || phase == "ready") {
            return self.status();
        }
        if self.busy.swap(true, Ordering::SeqCst) {
            return self.status();
        }
        if manual {
            self.set(json!({ "phase": "checking" }));
        }
        let out = match self.fetch_release().and_then(|rel| self.read_release(&rel, manual)) {
            Ok(v) => v,
            Err(f) => {
                if manual {
                    self.set(json!({ "phase": "error", "code": f.code, "checkedAt": now_ms() }))
                } else if self.phase() == "checking" {
                    self.set(json!({ "phase": "idle" }))
                } else {
                    self.status()
                }
            }
        };
        self.busy.store(false, Ordering::SeqCst);
        out
    }

    fn read_release(&self, rel: &Value, manual: bool) -> Result<Value, Failure> {
        let latest = rel["tag_name"].as_str().unwrap_or("").trim_start_matches(['v', 'V']).to_string();
        let current = &self.o.version;
        if !appstate::is_version(&latest)
            || rel["draft"] == true
            || rel["prerelease"] == true
            || appstate::compare_versions(&latest, current).is_le()
        {
            *self.release.lock() = None;
            return Ok(self.set(json!({ "phase": "none", "checkedAt": now_ms() })));
        }
        if !manual && appstate::bad_versions(&appstate::read_state(&self.o.data_dir)).contains(&latest) {
            // Cette version a déjà échoué ici : on attend la suivante, ou que l'administrateur la redemande.
            *self.release.lock() = None;
            return Ok(self.set(json!({ "phase": "none", "checkedAt": now_ms(), "skipped": latest })));
        }
        let notes: String = rel["body"].as_str().unwrap_or("").chars().take(4000).collect();
        let info = json!({
            "version": latest,
            "name": rel["name"].as_str().filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| format!("v{latest}")),
            "notes": notes,
            "page": rel["html_url"],
            "checkedAt": now_ms(),
        });
        let info = info.as_object().cloned().unwrap_or_default();
        let assets = rel["assets"].as_array().cloned().unwrap_or_default();
        let wanted = asset_name(&latest);
        let find = |name: &str| {
            assets
                .iter()
                .find(|a| a["name"] == name && a["browser_download_url"].as_str().is_some_and(|s| !s.is_empty()))
                .cloned()
        };
        let asset = find(&wanted);
        let sig_asset = find(&format!("{wanted}.sig"));
        let digest = asset
            .as_ref()
            .and_then(|a| a["digest"].as_str())
            .and_then(|d| d.strip_prefix("sha256:").or_else(|| d.strip_prefix("SHA256:")))
            .filter(|d| d.len() == 64 && d.chars().all(|c| c.is_ascii_hexdigit()))
            .map(str::to_lowercase);
        // Rien à installer tout seul : on signale la version et on renvoie vers sa page.
        let reason = if !self.supervised() {
            Some("unsupervised")
        } else if asset.is_none() {
            Some("asset")
        } else if digest.is_none() {
            Some("nodigest")
        } else if !self.keys.is_empty() && sig_asset.is_none() {
            Some("nosignature")
        } else {
            None
        };
        if let Some(reason) = reason {
            *self.release.lock() = None;
            let mut patch = info.clone();
            patch.insert("phase".into(), json!("available"));
            patch.insert("manual".into(), json!(true));
            patch.insert("reason".into(), json!(reason));
            return Ok(self.set(Value::Object(patch)));
        }
        let (Some(asset), Some(digest)) = (asset, digest) else { return Err(fail("asset")) };
        let asset_loc = self.locate(&asset)?;
        let signature = sig_asset.map(|a| self.locate(&a)).transpose()?;
        let size = asset["size"].as_u64().unwrap_or(0);
        *self.release.lock() =
            Some(Release { info: info.clone(), version: latest, asset: asset_loc, size, digest, signature });
        let mut patch = info;
        patch.insert("phase".into(), json!("available"));
        patch.insert("size".into(), json!(size));
        Ok(self.set(Value::Object(patch)))
    }

    /// Dépôt privé : le téléchargement passe par l'API (avec le jeton) ; sinon par l'adresse publique.
    fn locate(&self, a: &Value) -> Result<Located, Failure> {
        let api_url = a["url"].as_str().unwrap_or("");
        let url = if self.auth().is_some() && !api_url.is_empty() {
            api_url
        } else {
            a["browser_download_url"].as_str().unwrap_or("")
        };
        let trusted = url.starts_with(&format!("https://github.com/{REPO}/releases/download/"))
            || url
                .strip_prefix(&format!("https://api.github.com/repos/{REPO}/releases/assets/"))
                .is_some_and(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()));
        if !trusted && !self.api_url().1 {
            return Err(fail("untrusted"));
        }
        Ok(Located { url: url.to_string(), api: !api_url.is_empty() && url == api_url })
    }

    /// Télécharge un fichier de la release (dépôt privé : l'API répond par une adresse signée, que le jeton ne suit
    /// jamais).
    fn get_asset(
        &self,
        loc: &Located,
        max: u64,
        size: u64,
        progress: &dyn Fn(u64, u64),
    ) -> Result<(Vec<u8>, u64), Failure> {
        let mut target = loc.url.clone();
        let mut auth = None;
        if loc.api {
            let mut req = agent(0, Some(Duration::from_secs(12)))
                .get(&loc.url)
                .header("user-agent", AGENT)
                .header("accept", "application/octet-stream");
            if let Some(a) = self.auth() {
                req = req.header("authorization", a);
            }
            let res = req.call().map_err(|_| fail("network"))?;
            let location = (300..400)
                .contains(&res.status().as_u16())
                .then(|| res.headers().get("location").and_then(|v| v.to_str().ok()).map(str::to_string))
                .flatten();
            match location {
                Some(loc) => {
                    let host = url::Url::parse(&loc)
                        .ok()
                        .filter(|u| u.scheme() == "https")
                        .and_then(|u| u.host_str().map(str::to_lowercase))
                        .unwrap_or_default();
                    let ok = ["github.com", "githubusercontent.com"]
                        .iter()
                        .any(|d| host == *d || host.ends_with(&format!(".{d}")));
                    if !ok && !self.api_url().1 {
                        return Err(fail("untrusted"));
                    }
                    target = loc;
                }
                None => auth = self.auth(),
            }
        }
        let mut req = agent(5, None).get(&target).header("user-agent", AGENT);
        if let Some(a) = auth {
            req = req.header("accept", "application/octet-stream").header("authorization", a);
        }
        let res = req.call().map_err(|_| fail("network"))?;
        if !res.status().is_success() {
            return Err(fail("http"));
        }
        let total = res
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(size);
        let mut reader = res.into_body().into_reader();
        let mut out = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut last = 0i64;
        loop {
            let n = reader.read(&mut buf).map_err(|_| fail("network"))?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            if out.len() as u64 > max {
                return Err(fail("corrupt"));
            }
            if now_ms() - last > 200 {
                last = now_ms();
                progress(out.len() as u64, total);
            }
        }
        Ok((out, total))
    }

    /// Télécharge l'archive, vérifie son empreinte, puis l'installe à côté de la version en cours (sans la lancer).
    pub fn download(&self) -> Value {
        let Some(rel) = self.release.lock().clone() else { return self.status() };
        let phase = self.phase();
        if ["downloading", "installing", "ready"].contains(&phase.as_str()) || self.busy.swap(true, Ordering::SeqCst) {
            return self.status();
        }
        let info = |extra: Value| {
            let mut m = Map::new();
            for k in ["version", "name", "notes", "page"] {
                if let Some(v) = rel.info.get(k) {
                    m.insert(k.into(), v.clone());
                }
            }
            if let Value::Object(e) = extra {
                m.extend(e);
            }
            Value::Object(m)
        };
        self.set(info(json!({ "phase": "downloading", "size": rel.size, "received": 0 })));
        let progress = |received: u64, size: u64| {
            self.set(info(json!({ "phase": "downloading", "size": size, "received": received })));
        };
        let result = (|| -> Result<u64, Failure> {
            let (archive, total) = self.get_asset(&rel.asset, MAX_ARCHIVE, rel.size, &progress)?;
            if rel.size > 0 && archive.len() as u64 != rel.size {
                return Err(fail("corrupt"));
            }
            if sha256_hex(&archive) != rel.digest {
                return Err(fail("corrupt"));
            }
            // Signature (si des clés de confiance sont configurées) : c'est elle qui protège d'un dépôt compromis.
            if !self.keys.is_empty() {
                let sig = rel.signature.as_ref().ok_or_else(|| fail("signature"))?;
                let (sig, _) = self.get_asset(sig, 1024, 0, &|_, _| {})?;
                if !crate::sign::verify(&rel.version, &archive, &String::from_utf8_lossy(&sig), &self.keys) {
                    return Err(fail("signature"));
                }
            }
            stage(&self.o.data_dir, &archive, &rel.version)?;
            Ok(total)
        })();
        let out = match result {
            Ok(total) => self.set(info(
                json!({ "phase": "ready", "size": total, "verified": true, "signed": !self.keys.is_empty() }),
            )),
            Err(f) => self.set(info(json!({ "phase": "error", "code": f.code, "detail": f.detail }))),
        };
        self.busy.store(false, Ordering::SeqCst);
        out
    }

    /// Choisit la nouvelle version pour le prochain démarrage, puis demande au lanceur de redémarrer le serveur.
    pub fn install(self: &Arc<Self>) -> Value {
        let Some(rel) = self.release.lock().clone() else { return self.status() };
        if self.phase() != "ready" {
            return self.status();
        }
        let version = rel.version.clone();
        let Some(restart) = self.o.restart.clone() else {
            return self.set(json!({ "phase": "error", "code": "unsupervised", "version": version }));
        };
        if appstate::version_dir(&self.o.data_dir, &version).is_none() {
            return self.set(json!({ "phase": "error", "code": "stage", "version": version }));
        }
        let res = (|| -> anyhow::Result<()> {
            (self.o.backup)()?;
            let st = appstate::read_state(&self.o.data_dir);
            let bad: Vec<String> = appstate::bad_versions(&st).into_iter().filter(|v| *v != version).collect();
            let previous = std::env::var("CONTROL_STAGED").ok().filter(|v| appstate::is_version(v));
            appstate::write_state(
                &self.o.data_dir,
                json!({ "target": version, "previous": previous, "bad": bad, "failure": null }),
            )?;
            Ok(())
        })();
        if let Err(err) = res {
            return self
                .set(json!({ "phase": "error", "code": "stage", "detail": format!("{err:#}"), "version": version }));
        }
        let out = self.set(json!({ "phase": "installing", "version": version, "name": rel.info.get("name") }));
        let me = Arc::downgrade(self);
        let _ = std::thread::Builder::new().name("control-restart".into()).spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            restart();
            // Le lanceur ne redémarre pas : on le dit plutôt que d'afficher « Installation… » indéfiniment.
            std::thread::sleep(Duration::from_secs(90));
            if let Some(u) = me.upgrade()
                && u.phase() == "installing"
            {
                u.set(json!({ "phase": "error", "code": "launch", "version": version }));
            }
        });
        out
    }

    // ─── Planification ─────────────────────────────────────────────────────

    /// Au lancement (une minute après), puis toutes les 6 heures ; en mode automatique, télécharge et installe.
    pub fn start(self: &Arc<Self>) {
        self.cleanup();
        let me = Arc::downgrade(self);
        let stopped = self.stopped.clone();
        let _ = std::thread::Builder::new().name("control-updater".into()).spawn(move || {
            let started = now_ms();
            let (mut next_check, mut next_auto) = (started + 60_000, started + 60_000);
            while !stopped.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(500));
                let Some(u) = me.upgrade() else { return };
                let now = now_ms();
                if now >= next_check {
                    next_check = now + CHECK_EVERY_MS;
                    u.run();
                } else if now >= next_auto {
                    next_auto = now + 60_000;
                    u.auto();
                }
            }
        });
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    /// Une vérification périodique, suivie de ce que le mode automatique en fait.
    pub fn run(self: &Arc<Self>) {
        self.check(false);
        self.auto();
    }

    /// Mode automatique : télécharge, puis installe quand plus personne n'est connecté (ou à l'heure choisie).
    pub fn auto(self: &Arc<Self>) {
        let c = self.config();
        if c.mode != "auto" || !self.supervised() {
            return;
        }
        let manual = self.state.lock().get("manual").is_some_and(|v| v == true);
        if self.phase() == "available" && !manual {
            self.download();
        }
        let hour = i64::from(control_core::util::local_datetime(now_ms()).hour());
        if self.phase() == "ready" && ((self.o.idle)() || hour == c.hour) {
            self.install();
        }
    }

    /// Ménage au démarrage : téléchargements interrompus, versions dont on n'a plus besoin.
    pub fn cleanup(&self) {
        let dir = &self.o.data_dir;
        let st = appstate::read_state(dir);
        let staged = std::env::var("CONTROL_STAGED").ok().filter(|v| !v.is_empty());
        let target = appstate::str_of(&st, "target");
        // Une version installée autrement plus récente que celle téléchargée : on oublie celle-ci.
        if let Some(t) = &target
            && appstate::compare_versions(t, &self.o.version).is_le()
            && staged.is_none()
        {
            let _ = appstate::write_state(dir, json!({ "target": null, "previous": null }));
        }
        let st = appstate::read_state(dir);
        let keep: Vec<String> = [appstate::str_of(&st, "target"), appstate::str_of(&st, "previous"), staged]
            .into_iter()
            .flatten()
            .collect();
        let paths = appstate::paths(dir);
        if let Ok(entries) = std::fs::read_dir(&paths.versions) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let partial = name
                    .rsplit_once(".tmp-")
                    .is_some_and(|(_, pid)| !pid.is_empty() && pid.chars().all(|c| c.is_ascii_digit()));
                if partial || (appstate::is_version(&name) && !keep.contains(&name)) {
                    let _ = std::fs::remove_dir_all(e.path());
                }
            }
        }
        let _ = std::fs::remove_dir_all(paths.tmp);
    }
}

/// Extrait l'archive vérifiée dans `versions/<version>` : complète ou absente, jamais à moitié.
fn stage(data_dir: &Path, archive: &[u8], version: &str) -> Result<(), Failure> {
    let files = tar::unpack(
        archive,
        Limits { max_file_bytes: 150 * 1024 * 1024, max_total_bytes: 300 * 1024 * 1024, ..Limits::default() },
    )
    .map_err(|e| fail_with("stage", e.to_string()))?;
    let manifest = files
        .iter()
        .find(|f| f.name == "manifest.json")
        .and_then(|f| serde_json::from_slice::<Value>(&f.data).ok())
        .ok_or_else(|| fail_with("stage", "manifest.json absent ou illisible"))?;
    let binary = appstate::binary_name();
    if manifest["version"] != version || !files.iter().any(|f| f.name == binary) {
        return Err(fail_with("stage", "archive inattendue"));
    }
    let wanted = target();
    if manifest["target"].as_str().is_some_and(|t| t != wanted) {
        return Err(fail_with(
            "engine",
            format!("archive pour {} (ce serveur : {wanted})", manifest["target"].as_str().unwrap_or("")),
        ));
    }
    let versions = appstate::paths(data_dir).versions;
    let fin = versions.join(version);
    let work = versions.join(format!("{version}.tmp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    let res = (|| -> std::io::Result<()> {
        for f in &files {
            let dest = work.join(&f.name);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, &f.data)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    &dest,
                    std::fs::Permissions::from_mode(if f.mode == 0 { 0o644 } else { f.mode }),
                )?;
            }
        }
        std::fs::write(work.join(".ok"), format!("{}\n", now_ms()))?;
        let _ = std::fs::remove_dir_all(&fin);
        std::fs::rename(&work, &fin)
    })();
    if let Err(err) = res {
        let _ = std::fs::remove_dir_all(&work);
        return Err(fail_with("stage", err.to_string()));
    }
    Ok(())
}

/// Copie de la base du serveur avant de changer de version (les 3 dernières sont gardées).
pub fn backup_db(db: &rusqlite::Connection, data_dir: &Path, version: &str) -> anyhow::Result<()> {
    let dir = data_dir.join("backups");
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("server-{version}-{}.db", now_ms()));
    crate::serverdb::vacuum_into(db, &file)?;
    let mut all: Vec<String> = std::fs::read_dir(&dir)?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("server-") && n.ends_with(".db"))
        .collect();
    all.sort();
    let extra = all.len().saturating_sub(KEEP_BACKUPS);
    for old in &all[..extra] {
        let _ = std::fs::remove_file(dir.join(old));
    }
    Ok(())
}

#[cfg(test)]
mod tests;

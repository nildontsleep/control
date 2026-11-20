//! Outils externes (yt-dlp, ffmpeg, deno) : téléchargés au premier lancement dans le dossier `bin` des données.
//! Un ffmpeg (ou un yt-dlp) déjà présent dans le `PATH` du système sert tel quel.
//!
//! Événements : `tools:progress` pendant une installation, `tools:status` à la fin d'une installation ou d'une mise
//! à jour, `tools:updated` après la mise à jour automatique quotidienne de yt-dlp.

pub mod archive;
pub mod run;

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::{Emit, Event};
use crate::process::{exe_name, which};
use crate::store::Store;
use crate::util::now_ms;

pub use run::{kill_tree, run};

const GH: &str = "https://github.com";
const DAY_MS: i64 = 24 * 3600 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Ytdlp,
    Ffmpeg,
    Deno,
}

impl Tool {
    pub const ALL: [Tool; 3] = [Tool::Ytdlp, Tool::Ffmpeg, Tool::Deno];

    /// Nom de l'exécutable, sans extension.
    pub fn binary(self) -> &'static str {
        match self {
            Tool::Ytdlp => "yt-dlp",
            Tool::Ffmpeg => "ffmpeg",
            Tool::Deno => "deno",
        }
    }

    /// Nom dans les événements (`ytdlp`, `ffmpeg`, `deno`).
    pub fn key(self) -> &'static str {
        match self {
            Tool::Ytdlp => "ytdlp",
            Tool::Ffmpeg => "ffmpeg",
            Tool::Deno => "deno",
        }
    }

    pub fn parse(s: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.key() == s)
    }
}

/// Adresse de téléchargement d'un outil pour un système (`std::env::consts::OS`) et une architecture
/// (`std::env::consts::ARCH`). `None` : pas de version prête (ffmpeg sous macOS, systèmes non prévus).
pub fn asset_url(tool: Tool, os: &str, arch: &str) -> Option<String> {
    let arm = arch == "aarch64";
    let s = match (os, tool) {
        ("windows", Tool::Ytdlp) => format!("{GH}/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"),
        ("windows", Tool::Ffmpeg) => {
            format!("{GH}/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-win64-gpl.zip")
        }
        ("windows", Tool::Deno) => {
            format!("{GH}/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip")
        }
        ("linux", Tool::Ytdlp) => {
            let name = if arm { "yt-dlp_linux_aarch64" } else { "yt-dlp_linux" };
            format!("{GH}/yt-dlp/yt-dlp/releases/latest/download/{name}")
        }
        ("linux", Tool::Ffmpeg) => {
            let plat = if arm { "linuxarm64" } else { "linux64" };
            format!("{GH}/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-{plat}-gpl.tar.xz")
        }
        ("linux", Tool::Deno) => {
            let a = if arm { "aarch64" } else { "x86_64" };
            format!("{GH}/denoland/deno/releases/latest/download/deno-{a}-unknown-linux-gnu.zip")
        }
        ("macos", Tool::Ytdlp) => format!("{GH}/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos"),
        ("macos", Tool::Ffmpeg) => return None,
        ("macos", Tool::Deno) => {
            let a = if arm { "aarch64" } else { "x86_64" };
            format!("{GH}/denoland/deno/releases/latest/download/deno-{a}-apple-darwin.zip")
        }
        _ => return None,
    };
    Some(s)
}

fn current_asset(tool: Tool) -> Option<String> {
    asset_url(tool, std::env::consts::OS, std::env::consts::ARCH)
}

/// Faut-il lancer la mise à jour quotidienne de yt-dlp ?
pub fn auto_update_due(enabled: bool, available: bool, last_ms: i64, now: i64) -> bool {
    enabled && available && now - last_ms > DAY_MS
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolInfo {
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub supported: bool,
}

/// Contenu de `tools:status`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolsStatus {
    pub ytdlp: ToolInfo,
    pub ffmpeg: ToolInfo,
    pub deno: ToolInfo,
    pub installing: bool,
    /// yt-dlp et ffmpeg sont là : les téléchargements peuvent commencer.
    pub ready: bool,
}

/// Résultat d'une mise à jour de yt-dlp (contenu de `tools:updated`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UpdateResult {
    pub updated: bool,
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct Tools {
    bin_dir: PathBuf,
    tmp_dir: PathBuf,
    emit: Emit,
    install_lock: Mutex<()>,
    installing: AtomicBool,
    versions: Mutex<HashMap<Tool, String>>,
}

impl Tools {
    pub fn new(bin_dir: impl Into<PathBuf>, tmp_dir: impl Into<PathBuf>, emit: Emit) -> Tools {
        let bin_dir = bin_dir.into();
        if let Err(err) = std::fs::create_dir_all(&bin_dir) {
            log::warn!("Dossier des outils impossible à créer ({}) : {err}", bin_dir.display());
        }
        Tools {
            bin_dir,
            tmp_dir: tmp_dir.into(),
            emit,
            install_lock: Mutex::new(()),
            installing: AtomicBool::new(false),
            versions: Mutex::new(HashMap::new()),
        }
    }

    pub fn bin_dir(&self) -> &Path {
        &self.bin_dir
    }

    fn emit_json(&self, channel: &'static str, value: impl Serialize) {
        (self.emit)(Event::Json(channel, serde_json::to_value(value).unwrap_or(Value::Null)));
    }

    /// Exécutable d'un nom donné : celui du dossier `bin`, sinon celui du `PATH`.
    fn find(&self, base: &str) -> Option<PathBuf> {
        let local = self.bin_dir.join(exe_name(base));
        if local.is_file() { Some(local) } else { which(base) }
    }

    pub fn path(&self, tool: Tool) -> Option<PathBuf> {
        self.find(tool.binary())
    }

    /// ffprobe : installé avec ffmpeg, ou celui du système.
    pub fn ffprobe(&self) -> Option<PathBuf> {
        self.find("ffprobe")
    }

    /// Variables d'environnement des programmes lancés : le dossier `bin` (et celui de deno) en tête du `PATH`,
    /// Python en UTF-8 (sinon yt-dlp écrit les titres dans la page de code de Windows).
    pub fn env_vars(&self) -> Vec<(OsString, OsString)> {
        let mut dirs = vec![self.bin_dir.clone()];
        if let Some(dir) = self.path(Tool::Deno).and_then(|d| d.parent().map(Path::to_path_buf))
            && dir != self.bin_dir
        {
            dirs.push(dir);
        }
        if let Some(path) = std::env::var_os("PATH") {
            dirs.extend(std::env::split_paths(&path));
        }
        let mut vars = vec![("PYTHONUTF8".into(), "1".into()), ("PYTHONIOENCODING".into(), "utf-8".into())];
        if let Ok(path) = std::env::join_paths(dirs) {
            vars.push(("PATH".into(), path));
        }
        vars
    }

    /// Version de yt-dlp relue en lançant `yt-dlp --version`.
    pub fn refresh_versions(&self) {
        let version = self
            .path(Tool::Ytdlp)
            .and_then(|p| run(&p, &["--version"], Duration::from_secs(20), &self.env_vars()).ok())
            .filter(|v| !v.is_empty());
        let mut versions = self.versions.lock();
        match version {
            Some(v) => versions.insert(Tool::Ytdlp, v),
            None => versions.remove(&Tool::Ytdlp),
        };
    }

    pub fn version(&self, tool: Tool) -> Option<String> {
        self.versions.lock().get(&tool).cloned()
    }

    pub fn is_installing(&self) -> bool {
        self.installing.load(Ordering::SeqCst)
    }

    pub fn status(&self) -> ToolsStatus {
        let item = |tool: Tool| {
            let path = self.path(tool);
            ToolInfo {
                installed: path.is_some(),
                path: path.map(|p| p.to_string_lossy().into_owned()),
                version: self.version(tool),
                supported: current_asset(tool).is_some(),
            }
        };
        let (ytdlp, ffmpeg, deno) = (item(Tool::Ytdlp), item(Tool::Ffmpeg), item(Tool::Deno));
        let ready = ytdlp.installed && ffmpeg.installed;
        ToolsStatus { ytdlp, ffmpeg, deno, installing: self.is_installing(), ready }
    }

    /// Relit les versions puis envoie `tools:status` (au démarrage).
    pub fn announce(&self) {
        self.refresh_versions();
        self.emit_json("tools:status", self.status());
    }

    /// Installe les outils absents parmi `which` (les déjà présents, même dans le `PATH`, sont laissés tels quels).
    /// Bloquant ; un deuxième appel pendant une installation attend qu'elle finisse. S'arrête au premier échec.
    pub fn install(&self, which: &[Tool]) -> anyhow::Result<()> {
        let _guard = self.install_lock.lock();
        self.installing.store(true, Ordering::SeqCst);
        let res = self.install_locked(which);
        self.refresh_versions();
        self.installing.store(false, Ordering::SeqCst);
        self.emit_json("tools:status", self.status());
        res
    }

    fn progress(&self, tool: Tool, phase: &str, received: u64, total: u64) {
        self.emit_json(
            "tools:progress",
            json!({ "tool": tool.key(), "phase": phase, "received": received, "total": total }),
        );
    }

    fn install_locked(&self, which: &[Tool]) -> anyhow::Result<()> {
        for &tool in which {
            if self.path(tool).is_some() {
                continue;
            }
            let Some(url) = current_asset(tool) else { continue };
            if let Err(err) = self.install_one(tool, &url) {
                let message = format!("{err:#}");
                self.emit_json(
                    "tools:progress",
                    json!({ "tool": tool.key(), "phase": "error", "received": 0, "total": 0, "message": message }),
                );
                return Err(err);
            }
        }
        Ok(())
    }

    fn install_one(&self, tool: Tool, url: &str) -> anyhow::Result<()> {
        self.progress(tool, "download", 0, 0);
        let name = url.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or(tool.binary());
        std::fs::create_dir_all(&self.tmp_dir)?;
        let file = self.tmp_dir.join(name);
        download(url, &file, |r, t| self.progress(tool, "download", r, t))?;
        let res = if tool == Tool::Ytdlp {
            install_file(&file, &self.bin_dir.join(exe_name("yt-dlp")))
        } else {
            self.progress(tool, "extract", 0, 0);
            let kind = archive::archive_kind(name).ok_or_else(|| anyhow!("Archive inconnue : {name}"))?;
            let wanted: Vec<String> = match tool {
                Tool::Ffmpeg => vec![exe_name("ffmpeg"), exe_name("ffprobe")],
                _ => vec![exe_name(tool.binary())],
            };
            archive::extract_wanted(&file, kind, &wanted, &self.bin_dir).map(|_| ())
        };
        let _ = std::fs::remove_file(&file);
        res?;
        self.progress(tool, "done", 0, 0);
        Ok(())
    }

    /// Mise à jour de yt-dlp (YouTube change souvent : sans ça, ça casse en quelques semaines). Seul le yt-dlp du
    /// dossier `bin` est mis à jour, jamais celui du système.
    pub fn update(&self) -> UpdateResult {
        let ytdlp = self.bin_dir.join(exe_name("yt-dlp"));
        let before = self.version(Tool::Ytdlp);
        if !ytdlp.is_file() {
            return UpdateResult { updated: false, version: before, error: None };
        }
        if let Err(err) = run(&ytdlp, &["-U"], Duration::from_secs(120), &self.env_vars()) {
            return UpdateResult { updated: false, version: before, error: Some(err.to_string()) };
        }
        self.refresh_versions();
        self.emit_json("tools:status", self.status());
        let version = self.version(Tool::Ytdlp);
        UpdateResult { updated: before != version, version, error: None }
    }

    /// Mise à jour quotidienne (réglage `dl.autoUpdate`, date de la dernière dans `lastToolUpdate`). Envoie
    /// `tools:updated` quand elle a eu lieu.
    pub fn auto_update(&self, store: &Store) -> Option<UpdateResult> {
        let enabled = store.settings().dl.auto_update;
        let last = store.get("lastToolUpdate").as_f64().unwrap_or(0.0) as i64;
        let now = now_ms();
        if !auto_update_due(enabled, self.path(Tool::Ytdlp).is_some(), last, now) {
            return None;
        }
        store.patch(&json!({ "lastToolUpdate": now }));
        let result = self.update();
        self.emit_json("tools:updated", &result);
        Some(result)
    }
}

/// Copie un exécutable téléchargé à sa place (fichier temporaire renommé, droits 755 sous Unix).
fn install_file(src: &Path, dest: &Path) -> anyhow::Result<()> {
    let mut tmp = dest.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    std::fs::copy(src, &tmp).with_context(|| format!("Copie vers {} impossible", dest.display()))?;
    archive::make_executable(&tmp)?;
    std::fs::rename(&tmp, dest).with_context(|| format!("Copie vers {} impossible", dest.display()))?;
    Ok(())
}

/// Télécharge `url` dans `dest` (redirections suivies), en signalant la progression toutes les 150 ms.
pub fn download(url: &str, dest: &Path, mut on_progress: impl FnMut(u64, u64)) -> anyhow::Result<()> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .build()
        .into();
    let resp = agent.get(url).call().map_err(|e| match e {
        ureq::Error::StatusCode(code) => anyhow!("Téléchargement impossible (HTTP {code})"),
        e => anyhow!("Téléchargement impossible : {e}"),
    })?;
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let mut reader = resp.into_body().into_reader();
    let res = (|| -> anyhow::Result<u64> {
        let mut out = std::io::BufWriter::new(std::fs::File::create(&part)?);
        let mut buf = vec![0u8; 64 * 1024];
        let mut received = 0u64;
        let mut last = Instant::now();
        loop {
            let n = reader.read(&mut buf).context("Téléchargement interrompu")?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            received += n as u64;
            if last.elapsed() > Duration::from_millis(150) {
                last = Instant::now();
                on_progress(received, total);
            }
        }
        out.flush()?;
        Ok(received)
    })();
    match res {
        Ok(received) => {
            on_progress(received, total);
            std::fs::rename(&part, dest)?;
            Ok(())
        }
        Err(err) => {
            let _ = std::fs::remove_file(&part);
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::emit_nothing;

    #[test]
    fn assets_per_platform() {
        let u = |t, os, arch| asset_url(t, os, arch);
        assert_eq!(
            u(Tool::Ytdlp, "windows", "x86_64").unwrap(),
            "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
        );
        assert!(u(Tool::Ffmpeg, "windows", "x86_64").unwrap().ends_with("ffmpeg-master-latest-win64-gpl.zip"));
        assert!(u(Tool::Ffmpeg, "linux", "x86_64").unwrap().ends_with("ffmpeg-master-latest-linux64-gpl.tar.xz"));
        assert!(u(Tool::Ffmpeg, "linux", "aarch64").unwrap().ends_with("ffmpeg-master-latest-linuxarm64-gpl.tar.xz"));
        assert!(u(Tool::Ytdlp, "linux", "aarch64").unwrap().ends_with("/yt-dlp_linux_aarch64"));
        assert!(u(Tool::Deno, "linux", "x86_64").unwrap().ends_with("deno-x86_64-unknown-linux-gnu.zip"));
        assert!(u(Tool::Deno, "macos", "aarch64").unwrap().ends_with("deno-aarch64-apple-darwin.zip"));
        assert!(u(Tool::Ytdlp, "macos", "x86_64").unwrap().ends_with("/yt-dlp_macos"));
        assert_eq!(u(Tool::Ffmpeg, "macos", "aarch64"), None);
        assert_eq!(u(Tool::Ytdlp, "freebsd", "x86_64"), None);
        for t in Tool::ALL {
            if let Some(url) = u(t, "linux", "x86_64").filter(|_| t != Tool::Ytdlp) {
                assert!(archive::archive_kind(&url).is_some(), "{url}");
            }
        }
    }

    #[test]
    fn auto_update_once_a_day() {
        let now = 10 * DAY_MS;
        assert!(auto_update_due(true, true, 0, now));
        assert!(auto_update_due(true, true, now - DAY_MS - 1, now));
        assert!(!auto_update_due(true, true, now - 1000, now));
        assert!(!auto_update_due(false, true, 0, now));
        assert!(!auto_update_due(true, false, 0, now));
    }

    #[test]
    fn tool_names() {
        assert_eq!(Tool::parse("ytdlp"), Some(Tool::Ytdlp));
        assert_eq!(Tool::parse("yt-dlp"), None);
        assert_eq!(serde_json::to_value(Tool::Ffmpeg).unwrap(), json!("ffmpeg"));
        assert_eq!(Tool::Ytdlp.binary(), "yt-dlp");
    }

    #[test]
    fn local_tools_win_and_status_reports_them() {
        let dir = tempfile::tempdir().unwrap();
        let tools = Tools::new(dir.path().join("bin"), dir.path().join("tmp"), emit_nothing());
        let fake = dir.path().join("bin").join(exe_name("deno"));
        std::fs::write(&fake, b"").unwrap();
        assert_eq!(tools.path(Tool::Deno), Some(fake.clone()));
        let st = tools.status();
        assert!(st.deno.installed);
        assert_eq!(st.deno.path.as_deref(), Some(fake.to_string_lossy().as_ref()));
        assert!(!st.installing);
        let v = serde_json::to_value(&st).unwrap();
        assert!(v["ytdlp"].get("supported").is_some() && v.get("ready").is_some());
        let env = tools.env_vars();
        let path = env.iter().find(|(k, _)| k == "PATH").unwrap();
        assert_eq!(std::env::split_paths(&path.1).next().unwrap(), dir.path().join("bin"));
        assert_eq!(
            serde_json::to_value(UpdateResult { updated: false, version: None, error: None }).unwrap(),
            json!({"updated": false, "version": null})
        );
    }

    #[cfg(unix)]
    #[test]
    fn version_and_update_with_a_fake_ytdlp() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let tools = Tools::new(&bin, dir.path().join("tmp"), emit_nothing());
        let script = bin.join("yt-dlp");
        // `-U` réécrit le fichier de version : la version lue ensuite change.
        let state = dir.path().join("v");
        std::fs::write(&state, "2026.01.01").unwrap();
        let body = format!(
            "#!/bin/sh\nif [ \"$1\" = -U ]; then echo 2026.11.01 > '{0}'; exit 0; fi\ncat '{0}'\n",
            state.display()
        );
        std::fs::write(&script, body).unwrap();
        archive::make_executable(&script).unwrap();
        tools.refresh_versions();
        assert_eq!(tools.version(Tool::Ytdlp).as_deref(), Some("2026.01.01"));
        let r = tools.update();
        assert_eq!(r, UpdateResult { updated: true, version: Some("2026.11.01".into()), error: None });

        let store = Store::open(dir.path().join("settings.json"));
        let r = tools.auto_update(&store).unwrap();
        assert!(!r.updated);
        assert!(store.get("lastToolUpdate").as_i64().unwrap() > 0);
        assert_eq!(tools.auto_update(&store), None, "une fois par jour");
    }
}

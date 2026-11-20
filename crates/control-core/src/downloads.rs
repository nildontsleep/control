//! File de téléchargements yt-dlp : analyse d'un lien, recherche, progression, ajout à la bibliothèque.
//!
//! Chaque téléchargement en cours a son thread, qui lance yt-dlp et lit ses lignes de progression. L'état de la
//! file est gardé dans `downloads.json` (mêmes champs que la version d'origine) et envoyé à l'interface :
//! `dl:update` (un téléchargement, à chaque changement) et `dl:reset` (la liste entière, après un retrait).

pub mod cookies;
pub mod info;
pub mod job;
pub mod ytdlp;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use crossbeam_channel::RecvTimeoutError;
use parking_lot::Mutex;
use serde_json::Value;

use crate::library::Library;
use crate::model::{Emit, Event, PlaylistKind};
use crate::store::Store;
use crate::tools::{self, Tool, Tools};
use crate::util::{now_ms, write_atomic};

pub use info::{Info, SearchResult};
pub use job::{AddOptions, Job, JobId, JobStatus};
use ytdlp::Line;

const NOT_INSTALLED: &str = "yt-dlp n’est pas installé.";
const NOT_INSTALLED_LONG: &str =
    "yt-dlp n’est pas installé : ouvrez l’onglet Téléchargements pour installer les outils.";
/// Délai laissé à yt-dlp pour s'arrêter après une annulation, avant de l'arrêter de force.
const KILL_GRACE: Duration = Duration::from_secs(5);

/// Téléchargements simultanés d'après le réglage `dl.concurrency` (1 à 4 ; 2 si absent ou nul).
pub fn concurrency_limit(value: f64) -> usize {
    if !value.is_finite() || value == 0.0 { 2 } else { value.clamp(1.0, 4.0).ceil() as usize }
}

pub struct DownloadsConfig {
    pub tools: Arc<Tools>,
    pub library: Arc<Library>,
    pub store: Store,
    /// `downloads.json`.
    pub file: PathBuf,
    pub emit: Emit,
    /// cookies.txt imposé (serveur web : envoyé par le compte, voir [`cookies`]) ; prioritaire sur les réglages.
    pub cookies_file: Option<PathBuf>,
    /// Dossier de téléchargement quand le réglage `downloadDir` est vide.
    pub default_dir: PathBuf,
}

struct Proc {
    child: Arc<Mutex<Child>>,
    cancel_at: Option<Instant>,
}

#[derive(Default)]
struct State {
    jobs: Vec<Job>,
    procs: HashMap<JobId, Proc>,
    seq: i64,
    dirty: bool,
    saver_running: bool,
}

struct Inner {
    cfg: DownloadsConfig,
    state: Mutex<State>,
    write_lock: Mutex<()>,
    closed: AtomicBool,
}

/// File de téléchargements (clonable, partagée entre threads).
#[derive(Clone)]
pub struct Downloads {
    inner: Arc<Inner>,
}

impl Downloads {
    pub fn new(cfg: DownloadsConfig) -> Downloads {
        let jobs = std::fs::read_to_string(&cfg.file).map(|t| job::load_jobs(&t)).unwrap_or_default();
        let seq = jobs.iter().map(|j| j.id).max().unwrap_or(0).max(now_ms());
        let state = State { jobs, seq, ..State::default() };
        Downloads {
            inner: Arc::new(Inner {
                cfg,
                state: Mutex::new(state),
                write_lock: Mutex::new(()),
                closed: AtomicBool::new(false),
            }),
        }
    }

    pub fn list(&self) -> Vec<Job> {
        self.inner.state.lock().jobs.clone()
    }

    pub fn job(&self, id: JobId) -> Option<Job> {
        self.inner.state.lock().jobs.iter().find(|j| j.id == id).cloned()
    }

    /// Dossier où yt-dlp écrit : réglage `downloadDir`, sinon le dossier par défaut.
    pub fn download_dir(&self) -> PathBuf {
        match self.inner.cfg.store.get("downloadDir") {
            Value::String(s) if !s.trim().is_empty() => PathBuf::from(s),
            _ => self.inner.cfg.default_dir.clone(),
        }
    }

    fn cookie_args(&self) -> Vec<std::ffi::OsString> {
        let dl = self.inner.cfg.store.settings().dl;
        ytdlp::cookie_args(self.inner.cfg.cookies_file.as_deref(), &dl, Path::is_file)
    }

    fn ytdlp(&self) -> anyhow::Result<PathBuf> {
        self.inner.cfg.tools.path(Tool::Ytdlp).ok_or_else(|| anyhow!(NOT_INSTALLED))
    }

    fn run_json(&self, args: &[std::ffi::OsString], timeout: Duration) -> anyhow::Result<Value> {
        let text = tools::run(&self.ytdlp()?, args, timeout, &self.inner.cfg.tools.env_vars())?;
        serde_json::from_str(&text).map_err(|_| anyhow!("Réponse de yt-dlp illisible."))
    }

    /// Analyse d'un lien (`yt-dlp -J`). Bloquant (jusqu'à 90 s).
    pub fn info(&self, url: &str) -> anyhow::Result<Info> {
        self.ytdlp()?;
        let data = self.run_json(&ytdlp::info_args(url, self.cookie_args()), Duration::from_secs(90))?;
        Ok(info::parse_info(&data, url))
    }

    /// Recherche sur `soundcloud` ou `youtube` (`limit` : 1 à 25). Bloquant (jusqu'à 60 s).
    pub fn search(&self, source: &str, query: &str, limit: usize) -> anyhow::Result<Vec<SearchResult>> {
        self.ytdlp()?;
        let Some(target) = ytdlp::search_target(source, query, limit) else { return Ok(Vec::new()) };
        let cookies = if source == "youtube" { self.cookie_args() } else { Vec::new() };
        let data = self.run_json(&ytdlp::search_args(&target, cookies), Duration::from_secs(60))?;
        Ok(info::parse_search(&data, source))
    }

    // ─── File ─────────────────────────────────────────────────────────────────

    fn emit(&self, channel: &'static str, value: Value) {
        (self.inner.cfg.emit)(Event::Json(channel, value));
    }

    fn emit_job(&self, job: &Job) {
        self.emit("dl:update", serde_json::to_value(job).unwrap_or(Value::Null));
    }

    fn emit_reset(&self) {
        let jobs = self.list();
        self.emit("dl:reset", serde_json::to_value(jobs).unwrap_or(Value::Null));
    }

    /// Modifie un téléchargement, l'envoie à l'interface et prévoit l'écriture du fichier.
    fn update(&self, id: JobId, f: impl FnOnce(&mut Job)) -> Option<Job> {
        let job = {
            let mut st = self.inner.state.lock();
            let job = st.jobs.iter_mut().find(|j| j.id == id)?;
            f(job);
            let job = job.clone();
            st.dirty = true;
            job
        };
        self.emit_job(&job);
        self.schedule_save();
        Some(job)
    }

    /// Ajoute un téléchargement à la file et le lance si une place est libre.
    pub fn add(&self, opts: AddOptions) -> Job {
        let default_create = self.inner.cfg.store.settings().dl.create_playlist;
        let job = {
            let mut st = self.inner.state.lock();
            st.seq += 1;
            let job = opts.into_job(st.seq, now_ms(), default_create);
            st.jobs.push(job.clone());
            st.dirty = true;
            job
        };
        self.emit_job(&job);
        self.schedule_save();
        self.pump();
        job
    }

    /// Lance les téléchargements en file tant que la limite de `dl.concurrency` le permet.
    fn pump(&self) {
        if self.inner.closed.load(Ordering::SeqCst) {
            return;
        }
        let limit = concurrency_limit(self.inner.cfg.store.settings().dl.concurrency);
        let started: Vec<Job> = {
            let mut st = self.inner.state.lock();
            let running = st.jobs.iter().filter(|j| j.status.is_running()).count();
            let free = limit.saturating_sub(running);
            let mut started = Vec::new();
            for job in st.jobs.iter_mut().filter(|j| j.status == JobStatus::Queued).take(free) {
                job.status = JobStatus::Downloading;
                job.reset_run();
                started.push(job.clone());
            }
            if !started.is_empty() {
                st.dirty = true;
            }
            started
        };
        for job in started {
            self.emit_job(&job);
            let this = self.clone();
            let id = job.id;
            let spawned = std::thread::Builder::new().name(format!("control-dl-{id}")).spawn(move || this.run(id));
            if let Err(err) = spawned {
                self.update(id, |j| {
                    j.status = JobStatus::Error;
                    j.error = Some(format!("Téléchargement impossible à lancer : {err}"));
                });
            }
        }
        self.schedule_save();
    }

    /// Annule un téléchargement : yt-dlp est arrêté (avec ce qu'il a lancé), ou le téléchargement retiré de la file.
    pub fn cancel(&self, id: JobId) {
        let queued = {
            let mut st = self.inner.state.lock();
            let State { jobs, procs, .. } = &mut *st;
            let Some(job) = jobs.iter_mut().find(|j| j.id == id) else { return };
            if let Some(proc) = procs.get_mut(&id) {
                job.canceled = true;
                proc.cancel_at.get_or_insert_with(Instant::now);
                tools::kill_tree(&mut proc.child.lock(), false);
                false
            } else if job.status.is_running() {
                // Lancé mais pas encore démarré : le thread s'en apercevra.
                job.canceled = true;
                false
            } else {
                job.status == JobStatus::Queued
            }
        };
        if queued {
            self.update(id, |j| j.status = JobStatus::Canceled);
        }
    }

    pub fn retry(&self, id: JobId) {
        let ok = self
            .update(id, |j| {
                if !j.status.is_active() {
                    j.status = JobStatus::Queued;
                    j.reset_run();
                    j.playlist_id = None;
                    j.skipped = None;
                }
            })
            .is_some();
        if ok {
            self.pump();
        }
    }

    /// Retire de la liste tout ce qui n'est plus en file ni en cours.
    pub fn clear_finished(&self) {
        {
            let mut st = self.inner.state.lock();
            st.jobs.retain(|j| j.status.is_active());
            st.dirty = true;
        }
        self.emit_reset();
        self.schedule_save();
    }

    /// Retire un téléchargement qui ne tourne pas.
    pub fn remove(&self, id: JobId) {
        {
            let mut st = self.inner.state.lock();
            let running = st.procs.contains_key(&id) || st.jobs.iter().any(|j| j.id == id && j.status.is_running());
            if running || !st.jobs.iter().any(|j| j.id == id) {
                return;
            }
            st.jobs.retain(|j| j.id != id);
            st.dirty = true;
        }
        self.emit_reset();
        self.schedule_save();
    }

    /// Fermeture de l'application : tout yt-dlp en cours est arrêté et la liste écrite. Les téléchargements coupés
    /// passeront en erreur (« Interrompu… ») au prochain lancement.
    pub fn kill_all(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        {
            let st = self.inner.state.lock();
            for proc in st.procs.values() {
                tools::kill_tree(&mut proc.child.lock(), true);
            }
        }
        self.flush();
    }

    // ─── Fichier ─────────────────────────────────────────────────────────────

    /// Écrit `downloads.json` si la liste a changé.
    pub fn flush(&self) {
        let _w = self.inner.write_lock.lock();
        let text = {
            let mut st = self.inner.state.lock();
            if !st.dirty {
                return;
            }
            st.dirty = false;
            job::save_jobs(&st.jobs)
        };
        if let Err(err) = write_atomic(&self.inner.cfg.file, text.as_bytes()) {
            log::warn!("Impossible d’enregistrer les téléchargements : {err}");
        }
    }

    /// Écriture groupée : au plus une toutes les 400 ms, sur un thread qui s'arrête quand il n'y a plus rien à écrire.
    fn schedule_save(&self) {
        {
            let mut st = self.inner.state.lock();
            if !st.dirty || st.saver_running {
                return;
            }
            st.saver_running = true;
        }
        let weak = Arc::downgrade(&self.inner);
        let spawned = std::thread::Builder::new().name("control-dl-save".into()).spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(400));
                let Some(inner) = weak.upgrade() else { return };
                let this = Downloads { inner };
                this.flush();
                let mut st = this.inner.state.lock();
                if !st.dirty {
                    st.saver_running = false;
                    return;
                }
            }
        });
        if spawned.is_err() {
            self.inner.state.lock().saver_running = false;
            self.flush();
        }
    }

    // ─── Exécution ───────────────────────────────────────────────────────────

    fn fail(&self, id: JobId, message: String) {
        self.update(id, |j| {
            j.status = JobStatus::Error;
            j.error = Some(message);
            j.speed = None;
            j.eta = None;
        });
    }

    /// Thread d'un téléchargement : lance yt-dlp, suit sa sortie, ajoute les fichiers à la bibliothèque.
    fn run(&self, id: JobId) {
        self.run_inner(id);
        self.pump();
    }

    fn run_inner(&self, id: JobId) {
        let Some(job) = self.job(id) else { return };
        let cfg = &self.inner.cfg;
        let Some(ytdlp) = cfg.tools.path(Tool::Ytdlp) else {
            return self.fail(id, NOT_INSTALLED_LONG.into());
        };
        let out_dir = self.download_dir();
        if let Err(err) = std::fs::create_dir_all(&out_dir) {
            return self.fail(id, format!("Dossier de téléchargement inaccessible ({}) : {err}", out_dir.display()));
        }
        let ffmpeg = cfg.tools.path(Tool::Ffmpeg);
        let deno = cfg.tools.path(Tool::Deno);
        let extras = ytdlp::Extras { cookies: self.cookie_args(), ffmpeg: ffmpeg.as_deref(), deno: deno.as_deref() };
        let args = ytdlp::download_args(&job.spec(), &out_dir, extras);

        let mut cmd = Command::new(&ytdlp);
        cmd.args(&args).envs(cfg.tools.env_vars());
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        tools::run::configure(&mut cmd);

        let mut errors: Vec<String> = Vec::new();
        let code = match cmd.spawn() {
            Err(err) => {
                errors.push(format!("{} : {err}", ytdlp.display()));
                -1
            }
            Ok(child) => self.follow(&job, child, &mut errors),
        };
        if self.inner.closed.load(Ordering::SeqCst) {
            return;
        }

        let Some(job) = self.job(id) else { return };
        let mut playlist_id = None;
        if !job.canceled
            && let Some(name) = job.playlist_name.as_deref()
            && !job.track_ids.is_empty()
        {
            playlist_id = cfg.library.create_playlist(name, PlaylistKind::Manual, None);
            if let Some(pl) = playlist_id {
                cfg.library.add_to_playlist(pl, &job.track_ids);
            }
        }
        let last = errors.pop();
        self.update(id, |j| j.finish(code, last, playlist_id));
    }

    /// Suit yt-dlp jusqu'à sa fin et renvoie son code de sortie (-1 s'il a été tué).
    fn follow(&self, job: &Job, mut child: Child, errors: &mut Vec<String>) -> i32 {
        let id = job.id;
        let (tx, rx) = crossbeam_channel::unbounded::<String>();
        for stream in [
            child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
            child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream);
                let mut buf = Vec::new();
                while matches!(reader.read_until(b'\n', &mut buf), Ok(n) if n > 0) {
                    let line = String::from_utf8_lossy(&buf);
                    let line = line.trim_end_matches(['\n', '\r']);
                    if !line.is_empty() && tx.send(line.to_string()).is_err() {
                        return;
                    }
                    buf.clear();
                }
            });
        }
        drop(tx);

        let child = Arc::new(Mutex::new(child));
        {
            let mut st = self.inner.state.lock();
            let canceled = st.jobs.iter().any(|j| j.id == id && j.canceled);
            let cancel_at = canceled.then(Instant::now);
            if canceled {
                tools::kill_tree(&mut child.lock(), false);
            }
            st.procs.insert(id, Proc { child: child.clone(), cancel_at });
        }

        let mut last_emit: Option<Instant> = None;
        let mut forced = false;
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(line) => self.handle_line(job, &line, errors, &mut last_emit),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.escalate(id, &child, &mut forced);
        }
        let status = loop {
            match child.lock().try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {}
                Err(_) => break None,
            }
            self.escalate(id, &child, &mut forced);
            std::thread::sleep(Duration::from_millis(50));
        };
        self.inner.state.lock().procs.remove(&id);
        status.and_then(|s| s.code()).unwrap_or(-1)
    }

    /// yt-dlp annulé qui tarde à s'arrêter : arrêt forcé.
    fn escalate(&self, id: JobId, child: &Arc<Mutex<Child>>, forced: &mut bool) {
        if *forced {
            return;
        }
        let due =
            self.inner.state.lock().procs.get(&id).and_then(|p| p.cancel_at).is_some_and(|t| t.elapsed() > KILL_GRACE);
        if due {
            *forced = true;
            tools::kill_tree(&mut child.lock(), true);
        }
    }

    fn handle_line(&self, job: &Job, line: &str, errors: &mut Vec<String>, last_emit: &mut Option<Instant>) {
        let id = job.id;
        match ytdlp::parse_line(line) {
            Line::Progress(p) => {
                let due = last_emit.is_none_or(|t| t.elapsed() > Duration::from_millis(250));
                if p.status == "finished" || due {
                    *last_emit = Some(Instant::now());
                    self.update(id, |j| j.apply_progress(&p));
                }
            }
            Line::Post(title) => {
                self.update(id, |j| j.apply_post(title));
            }
            Line::File { path, url } => {
                self.update(id, |j| {
                    j.files.push(path.clone());
                    j.status = JobStatus::Downloading;
                });
                let source = url.unwrap_or_else(|| job.url.clone());
                let ids = self.inner.cfg.library.add_paths(std::slice::from_ref(&path), Some(&source));
                if !ids.is_empty() {
                    self.update(id, |j| j.track_ids.extend(ids));
                }
            }
            Line::Error(msg) => errors.push(msg),
            Line::Other => {}
        }
    }
}

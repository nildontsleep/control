//! Discord Rich Presence : dialogue avec le client Discord installé sur l'ordinateur par son canal local
//! (tube nommé sous Windows, socket Unix ailleurs). Aucun compte, aucun jeton : seul l'identifiant public de
//! l'application Discord de control est nécessaire.
//!
//! [`DiscordClient`] garde la connexion sur son propre thread (`control-discord`) : il se reconnecte tout seul,
//! regroupe les mises à jour (Discord en accepte 5 par 20 s), n'envoie rien qui n'ait changé, et cherche une
//! adresse publique pour la pochette sur un second thread. L'activité elle-même se construit avec
//! [`build_activity`], fonction pure.

mod activity;
mod covers;
mod ipc;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};

pub use activity::{
    Activity, ActivityKind, CoverRef, IDLE_CLEAR_MS, Icon, NowPlaying, PresenceFeed, Streaming, Texts, activity_json,
    build_activity, clip,
};
pub use covers::{CoverCache, CoverEntry, CoverFileFn, multipart, sniff_image};
pub use ipc::{Decoder, Frame, OP_CLOSE, OP_FRAME, OP_HANDSHAKE, OP_PING, OP_PONG, encode, pipe_paths};

use crate::model::{Emit, Event};
use crate::util::now_ms;
use ipc::Conn;

/// Identifiant de l'application créée sur https://discord.com/developers/applications (public, ce n'est pas un
/// secret). `CONTROL_DISCORD_ID` le remplace.
pub const APP_ID: &str = "1563182510202949733";
pub const STATUS_CHANNEL: &str = "discord:status";

/// Discord limite à 5 mises à jour par 20 s.
const MIN_GAP: Duration = Duration::from_millis(3500);
const RETRY: Duration = Duration::from_secs(15);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(6);
const REPLY_TIMEOUT: Duration = Duration::from_secs(3);
const DEBOUNCE: Duration = Duration::from_millis(120);
/// Vérification que Discord n'a pas fermé le canal (sous Unix).
const POLL: Duration = Duration::from_secs(5);
/// En passant vite d'un morceau à l'autre, on n'envoie pas toutes les pochettes traversées.
const COVER_DEBOUNCE: Duration = Duration::from_millis(1200);

static CLIENT_ID_MSG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)client.?id").expect("regex constante"));

/// Ce que devient la pochette de la lecture en cours (affiché dans les réglages).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoverStatus {
    /// `none`, `uploading`, `ok` ou `failed`.
    pub state: String,
    pub detail: String,
}

/// Payload de `discord:status` et réponse de `test`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// `off`, `missing-id`, `connecting`, `no-discord`, `connected`, `error` ou `bad-id`.
    pub state: String,
    /// Nom de l'utilisateur Discord une fois connecté, ou raison de l'erreur.
    pub detail: String,
    pub client_id: String,
    pub cover: CoverStatus,
}

/// Paramètres de [`DiscordClient::new`].
pub struct DiscordOptions {
    pub emit: Emit,
    /// Fichier image d'une pochette ou d'une image de vidéo (déjà rangées par la bibliothèque).
    pub cover_file: Option<CoverFileFn>,
    /// Fichier JSON où garder les adresses déjà obtenues (`discord-covers.json`).
    pub cache_file: Option<PathBuf>,
    /// Dossier du canal de Discord, à la place de la recherche habituelle (essais ; `CONTROL_DISCORD_IPC_DIR`).
    pub ipc_dir: Option<PathBuf>,
}

enum Cmd {
    Configure { enabled: bool, client_id: String },
    Activity(Option<Activity>),
    Test(Sender<Status>),
    Shutdown,
}

struct CoverJob {
    cover: CoverRef,
    token: u64,
    debounce: bool,
}

struct CoverDone {
    token: u64,
    result: covers::CoverResult,
}

/// Client Rich Presence. Les copies pilotent le même thread ; quand la dernière est libérée, la présence est
/// effacée et le thread s'arrête.
#[derive(Clone)]
pub struct DiscordClient {
    tx: Sender<Cmd>,
    status: Arc<Mutex<Status>>,
}

impl DiscordClient {
    pub fn new(o: DiscordOptions) -> DiscordClient {
        let app_id = std::env::var("CONTROL_DISCORD_ID").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| APP_ID.into());
        let status = Arc::new(Mutex::new(Status {
            state: "off".into(),
            detail: String::new(),
            client_id: app_id.clone(),
            cover: CoverStatus { state: "none".into(), detail: String::new() },
        }));
        let (tx, rx) = crossbeam_channel::unbounded();
        let (jobs_tx, jobs_rx) = crossbeam_channel::unbounded::<CoverJob>();
        let (done_tx, done_rx) = crossbeam_channel::unbounded::<CoverDone>();
        let cache = Arc::new(Mutex::new(CoverCache::load(o.cache_file)));
        let token = Arc::new(AtomicU64::new(0));
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        {
            let (cache, token, cover_file) = (cache.clone(), token.clone(), o.cover_file);
            let spawned = std::thread::Builder::new()
                .name("control-discord-covers".into())
                .spawn(move || cover_worker(jobs_rx, done_tx, token, cache, cover_file, agent));
            if let Err(err) = spawned {
                log::warn!("Thread des pochettes Discord impossible : {err}");
            }
        }
        let env = |k: &str| std::env::var(k).ok();
        let paths = pipe_paths(cfg!(windows), &env, o.ipc_dir.as_deref());
        let worker = Worker {
            app_id,
            override_id: String::new(),
            enabled: false,
            conn: None,
            status: status.clone(),
            emit: o.emit,
            paths,
            activity: None,
            cover: None,
            cover_key: String::new(),
            cover_token: token,
            cover_retry: None,
            cover_jobs: jobs_tx,
            cache,
            last_base: None,
            last_start: None,
            last_send_at: None,
            flush_at: None,
            retry_at: None,
            poll_at: None,
            nonce: 0,
        };
        let spawned = std::thread::Builder::new().name("control-discord".into()).spawn(move || worker.run(rx, done_rx));
        if let Err(err) = spawned {
            log::warn!("Thread Discord impossible : {err}");
        }
        DiscordClient { tx, status }
    }

    /// Activation et identifiant d'application (celui des réglages prime sur celui du programme).
    pub fn configure(&self, enabled: bool, client_id: &str) {
        let _ = self.tx.send(Cmd::Configure { enabled, client_id: client_id.to_string() });
    }

    /// Depuis les réglages `discord`.
    pub fn configure_from(&self, d: &crate::settings::Discord) {
        self.configure(d.enabled, &d.client_id);
    }

    /// `None` efface la présence.
    pub fn set_activity(&self, activity: Option<Activity>) {
        let _ = self.tx.send(Cmd::Activity(activity.and_then(Activity::sanitized)));
    }

    pub fn status(&self) -> Status {
        self.status.lock().clone()
    }

    /// Essaie de se connecter maintenant et dit ce qui s'est passé (bouton « Tester » des réglages). Bloquant
    /// (quelques secondes au plus).
    pub fn test(&self) -> Status {
        let (tx, rx) = crossbeam_channel::bounded(1);
        if self.tx.send(Cmd::Test(tx)).is_err() {
            return self.status();
        }
        rx.recv_timeout(HANDSHAKE_TIMEOUT + Duration::from_secs(4)).unwrap_or_else(|_| self.status())
    }

    /// Efface la présence et ferme la connexion (fin de l'application).
    pub fn shutdown(&self) {
        let _ = self.tx.send(Cmd::Shutdown);
    }
}

fn valid_client_id(id: &str) -> bool {
    (15..=25).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

/// Ce qu'une trame reçue demande de faire.
enum Incoming {
    Ready(String),
    Closed { bad_id: bool, message: String },
    Error(String),
    Reply(Option<String>),
    Ignore,
}

fn classify(op: i32, data: Option<&Value>) -> Incoming {
    if op == OP_CLOSE {
        let message = data.and_then(|d| d.get("message")).and_then(Value::as_str).unwrap_or_default().to_string();
        let code = data.and_then(|d| d.get("code")).and_then(Value::as_i64);
        return Incoming::Closed { bad_id: code == Some(4000) || CLIENT_ID_MSG.is_match(&message), message };
    }
    let Some(d) = data else { return Incoming::Ignore };
    let evt = d.get("evt").and_then(Value::as_str);
    if d.get("cmd").and_then(Value::as_str) == Some("DISPATCH") && evt == Some("READY") {
        let user = d.pointer("/data/user/username").and_then(Value::as_str).unwrap_or_default();
        return Incoming::Ready(user.to_string());
    }
    if evt == Some("ERROR") {
        let msg = d
            .pointer("/data/message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| d.pointer("/data/code").map(|c| c.to_string()))
            .unwrap_or_default();
        return Incoming::Error(msg);
    }
    Incoming::Reply(d.get("nonce").and_then(Value::as_str).map(str::to_string))
}

struct Worker {
    app_id: String,
    override_id: String,
    enabled: bool,
    /// Connexion prête (poignée de main faite).
    conn: Option<Conn>,
    status: Arc<Mutex<Status>>,
    emit: Emit,
    paths: Vec<PathBuf>,
    activity: Option<Activity>,
    /// Adresse publique de la pochette de l'activité en cours.
    cover: Option<String>,
    cover_key: String,
    cover_token: Arc<AtomicU64>,
    cover_retry: Option<(Instant, u64)>,
    cover_jobs: Sender<CoverJob>,
    cache: Arc<Mutex<CoverCache>>,
    last_base: Option<String>,
    last_start: Option<i64>,
    last_send_at: Option<Instant>,
    flush_at: Option<Instant>,
    retry_at: Option<Instant>,
    poll_at: Option<Instant>,
    nonce: u64,
}

impl Worker {
    fn run(mut self, rx: Receiver<Cmd>, mut done_rx: Receiver<CoverDone>) {
        loop {
            let next = [self.flush_at, self.retry_at, self.cover_retry.map(|(at, _)| at), self.poll_at]
                .into_iter()
                .flatten()
                .min();
            let wait = next.map(|t| t.saturating_duration_since(Instant::now())).unwrap_or(Duration::from_secs(3600));
            crossbeam_channel::select! {
                recv(rx) -> msg => match msg {
                    Ok(Cmd::Configure { enabled, client_id }) => self.configure(enabled, &client_id),
                    Ok(Cmd::Activity(a)) => self.set_activity(a),
                    Ok(Cmd::Test(reply)) => {
                        let s = self.test();
                        let _ = reply.send(s);
                    }
                    Ok(Cmd::Shutdown) | Err(_) => {
                        self.shutdown();
                        return;
                    }
                },
                recv(done_rx) -> msg => match msg {
                    Ok(done) => self.cover_done(done),
                    Err(_) => done_rx = crossbeam_channel::never(),
                },
                default(wait) => {}
            }
            self.timers();
        }
    }

    fn timers(&mut self) {
        let now = Instant::now();
        if self.retry_at.is_some_and(|t| t <= now) {
            self.retry_at = None;
            self.connect();
        }
        if self.poll_at.is_some_and(|t| t <= now) {
            self.poll_at = None;
            self.poll();
        }
        if let Some((at, token)) = self.cover_retry
            && at <= now
        {
            self.cover_retry = None;
            if token == self.cover_token.load(Ordering::SeqCst) {
                self.cover_key.clear();
                self.sync_cover();
            }
        }
        if self.flush_at.is_some_and(|t| t <= now) {
            self.flush_at = None;
            self.flush();
        }
    }

    fn id(&self) -> String {
        if self.override_id.is_empty() { self.app_id.clone() } else { self.override_id.clone() }
    }

    fn publish(&self) {
        let s = self.status.lock().clone();
        (self.emit)(Event::Json(STATUS_CHANNEL, serde_json::to_value(s).unwrap_or(Value::Null)));
    }

    fn set(&mut self, state: &str, detail: &str) {
        {
            let mut s = self.status.lock();
            let id = self.id();
            if s.state == state && s.detail == detail && s.client_id == id {
                return;
            }
            s.state = state.into();
            s.detail = detail.into();
            s.client_id = id;
        }
        self.publish();
    }

    fn set_cover(&mut self, state: &str, detail: &str) {
        {
            let mut s = self.status.lock();
            if s.cover.state == state && s.cover.detail == detail {
                return;
            }
            s.cover = CoverStatus { state: state.into(), detail: detail.into() };
        }
        self.publish();
    }

    fn state(&self) -> String {
        self.status.lock().state.clone()
    }

    fn configure(&mut self, enabled: bool, client_id: &str) {
        let id = client_id.trim();
        let id = if valid_client_id(id) { id.to_string() } else { String::new() };
        let changed = id != self.override_id || enabled != self.enabled;
        self.override_id = id;
        self.enabled = enabled;
        if !changed {
            return;
        }
        self.drop_conn();
        self.retry_at = None;
        if !self.enabled {
            self.set("off", "");
        } else if self.id().is_empty() {
            self.set("missing-id", "");
        } else {
            self.connect();
        }
    }

    fn set_activity(&mut self, a: Option<Activity>) {
        self.activity = a;
        self.sync_cover();
        self.schedule();
    }

    // ─── Pochette ────────────────────────────────────────────────────────────

    /// Cherche la pochette de l'activité en cours si elle a changé (ou si on l'a oubliée pour réessayer).
    fn sync_cover(&mut self) {
        let r = self.activity.as_ref().and_then(|a| a.cover.clone());
        // Rien n'est envoyé à un hébergeur tant que Discord n'est pas là pour l'afficher : la recherche reprend à la
        // connexion (écart volontaire avec l'original, qui envoyait la pochette même sans Discord ouvert).
        if r.is_some() && self.state() != "connected" {
            self.cover_key.clear();
            self.cover = None;
            return;
        }
        let key = r.as_ref().map(CoverRef::key).unwrap_or_default();
        if key == self.cover_key {
            return;
        }
        self.cover_key = key.clone();
        self.cover = None;
        self.cover_retry = None;
        let token = self.cover_token.fetch_add(1, Ordering::SeqCst) + 1;
        let Some(r) = r else {
            self.set_cover("none", "");
            return;
        };
        let hit = self.cache.lock().hit(&key, now_ms()).cloned();
        if let Some(e) = hit {
            self.cover = Some(e.url);
            self.set_cover("ok", &e.host.unwrap_or_else(|| "hébergeur".into()));
            return;
        }
        self.set_cover("uploading", "");
        let _ = self.cover_jobs.send(CoverJob { cover: r, token, debounce: true });
    }

    fn cover_done(&mut self, done: CoverDone) {
        if done.token != self.cover_token.load(Ordering::SeqCst) {
            return;
        }
        match done.result {
            Ok((url, host)) => {
                self.cover = Some(url);
                self.set_cover("ok", &host);
                self.schedule();
            }
            Err(detail) => {
                self.set_cover("failed", if detail.is_empty() { "réseau" } else { &detail });
                // Un échec n'est pas définitif (réseau coupé, hébergeur occupé…) : on réessaie un peu plus tard.
                let wait = Duration::from_millis(covers::COVER_RETRY_MS as u64 + 500);
                self.cover_retry = Some((Instant::now() + wait, done.token));
            }
        }
    }

    // ─── Connexion ───────────────────────────────────────────────────────────

    fn retry_later(&mut self) {
        self.retry_at = self.enabled.then(|| Instant::now() + RETRY);
    }

    fn connect(&mut self) {
        if self.conn.is_some() || !self.enabled || self.id().is_empty() {
            return;
        }
        self.set("connecting", "");
        let Some(mut conn) = self.paths.iter().find_map(|p| Conn::open(p).ok()) else {
            self.set("no-discord", "");
            self.retry_later();
            return;
        };
        if conn.write_frame(OP_HANDSHAKE, &json!({ "v": 1, "client_id": self.id() })).is_err() {
            self.set("no-discord", "");
            self.retry_later();
            return;
        }
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match conn.read_frame(left) {
                Ok(None) => {
                    self.set("error", "timeout");
                    let _ = conn.write_frame(OP_CLOSE, &json!({}));
                    self.retry_later();
                    return;
                }
                Err(_) => {
                    self.set("no-discord", "");
                    self.retry_later();
                    return;
                }
                Ok(Some((op, data))) => {
                    if op == OP_PING {
                        let _ = conn.write_frame(OP_PONG, &data.unwrap_or(Value::Null));
                        continue;
                    }
                    match classify(op, data.as_ref()) {
                        Incoming::Ready(user) => {
                            self.conn = Some(conn);
                            self.last_base = None;
                            self.poll_at = Some(Instant::now() + POLL);
                            self.set("connected", &user);
                            self.sync_cover();
                            self.flush();
                            return;
                        }
                        Incoming::Closed { bad_id, message } => {
                            self.set(if bad_id { "bad-id" } else { "error" }, &message);
                            if !bad_id {
                                self.retry_later();
                            }
                            return;
                        }
                        Incoming::Error(msg) => self.set("error", &msg),
                        Incoming::Reply(_) | Incoming::Ignore => {}
                    }
                }
            }
        }
    }

    /// Discord a fermé le canal.
    fn lost(&mut self) {
        self.conn = None;
        self.poll_at = None;
        if self.enabled && self.state() != "bad-id" {
            self.set("no-discord", "");
            self.retry_later();
        }
    }

    /// Traite une trame reçue hors poignée de main. Renvoie le nonce d'une réponse.
    fn incoming(&mut self, op: i32, data: Option<Value>) -> Option<String> {
        if op == OP_PING {
            let ok = self.conn.as_mut().is_some_and(|c| c.write_frame(OP_PONG, &data.unwrap_or(Value::Null)).is_ok());
            if !ok {
                self.lost();
            }
            return None;
        }
        match classify(op, data.as_ref()) {
            Incoming::Closed { bad_id, message } => {
                self.set(if bad_id { "bad-id" } else { "error" }, &message);
                self.drop_conn();
                if !bad_id {
                    self.retry_later();
                }
                None
            }
            Incoming::Error(msg) => {
                self.set("error", &msg);
                None
            }
            Incoming::Reply(nonce) => nonce,
            Incoming::Ready(_) | Incoming::Ignore => None,
        }
    }

    fn poll(&mut self) {
        let Some(conn) = self.conn.as_mut() else { return };
        match conn.poll() {
            Ok(frames) => {
                for (op, data) in frames {
                    self.incoming(op, data);
                }
                if self.conn.is_some() {
                    self.poll_at = Some(Instant::now() + POLL);
                }
            }
            Err(_) => self.lost(),
        }
    }

    fn drop_conn(&mut self) {
        self.flush_at = None;
        self.poll_at = None;
        if let Some(mut c) = self.conn.take() {
            let _ = c.write_frame(OP_CLOSE, &json!({}));
        }
    }

    // ─── Envoi ───────────────────────────────────────────────────────────────

    fn schedule(&mut self) {
        let now = Instant::now();
        let gap = self.last_send_at.map(|t| (t + MIN_GAP).saturating_duration_since(now)).unwrap_or_default();
        self.flush_at = Some(now + gap.max(DEBOUNCE));
    }

    /// Écrit une commande et attend sa réponse (sans quoi le canal finirait par se remplir).
    fn command(&mut self, frame: &Value, nonce: &str) {
        let Some(conn) = self.conn.as_mut() else { return };
        if conn.write_frame(OP_FRAME, frame).is_err() {
            self.lost();
            return;
        }
        let deadline = Instant::now() + REPLY_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let Some(conn) = self.conn.as_mut() else { return };
            match conn.read_frame(left) {
                Ok(None) => return,
                Err(_) => {
                    self.lost();
                    return;
                }
                Ok(Some((op, data))) => {
                    if self.incoming(op, data).as_deref() == Some(nonce) {
                        return;
                    }
                }
            }
        }
    }

    fn flush(&mut self) {
        if self.conn.is_none() {
            return;
        }
        let now = now_ms();
        let built = self.activity.as_ref().map(|a| activity_json(a, self.cover.as_deref(), now));
        // Ne rien renvoyer si rien n'a changé : l'application rappelle souvent la même position.
        let (base, start) = match &built {
            Some(Value::Object(m)) => {
                let mut rest = m.clone();
                let start = rest.remove("timestamps").and_then(|t| t.get("start").and_then(Value::as_i64));
                (Value::Object(rest).to_string(), start)
            }
            _ => ("null".to_string(), None),
        };
        let same = self.last_base.as_deref() == Some(base.as_str())
            && match (start, self.last_start) {
                (None, None) => true,
                (Some(a), Some(b)) => (a - b).abs() < 3000,
                _ => false,
            };
        if same {
            return;
        }
        self.last_base = Some(base);
        self.last_start = start;
        self.last_send_at = Some(Instant::now());
        self.nonce += 1;
        let nonce = self.nonce.to_string();
        let mut args = json!({ "pid": std::process::id() });
        if let Some(a) = built {
            args["activity"] = a;
        }
        self.command(&json!({ "cmd": "SET_ACTIVITY", "args": args, "nonce": nonce }), &nonce);
    }

    fn test(&mut self) -> Status {
        if !self.enabled {
            let mut s = self.status.lock().clone();
            s.state = "off".into();
            s.detail.clear();
            s.client_id = self.id();
            return s;
        }
        if self.id().is_empty() {
            return self.status.lock().clone();
        }
        if self.conn.is_some() {
            self.poll();
        }
        if self.state() != "connected" || self.conn.is_none() {
            self.retry_at = None;
            self.connect();
        }
        self.status.lock().clone()
    }

    fn shutdown(&mut self) {
        self.enabled = false;
        self.retry_at = None;
        if self.conn.is_some() {
            let frame = json!({ "cmd": "SET_ACTIVITY", "args": { "pid": std::process::id() }, "nonce": "bye" });
            self.command(&frame, "bye");
        }
        self.drop_conn();
    }
}

/// Thread des pochettes : une recherche à la fois, abandonnée si l'activité a changé entre-temps.
fn cover_worker(
    jobs: Receiver<CoverJob>,
    done: Sender<CoverDone>,
    token: Arc<AtomicU64>,
    cache: Arc<Mutex<CoverCache>>,
    cover_file: Option<CoverFileFn>,
    agent: ureq::Agent,
) {
    let current = |t: u64| token.load(Ordering::SeqCst) == t;
    for job in jobs {
        if !current(job.token) {
            continue;
        }
        if job.debounce && cache.lock().hit(&job.cover.key(), now_ms()).is_none() {
            std::thread::sleep(COVER_DEBOUNCE);
            if !current(job.token) {
                continue;
            }
        }
        let result = covers::cover_url(&cache, cover_file.as_ref(), &agent, &job.cover);
        if !current(job.token) {
            continue;
        }
        if done.send(CoverDone { token: job.token, result }).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests;

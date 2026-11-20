//! Scrobbling : envoie ce qu'on écoute à ListenBrainz et à Last.fm (« en écoute » au début d'un morceau, puis
//! l'écoute complète quand elle compte : au moins 30 s de morceau, et la moitié écoutée ou 4 minutes).
//!
//! Les écoutes qui n'ont pas pu partir (réseau coupé…) restent dans une file, enregistrée sur le disque, et sont
//! renvoyées plus tard. Partagé par le bureau et le serveur web ; les jetons vivent dans un fichier à part
//! (`scrobble.json`, même format que la version d'origine), jamais dans les réglages que la page reçoit.
//!
//! Les envois réseau se font sur un thread à part (`control-scrobble`) : [`Scrobbler::started`] et
//! [`Scrobbler::ended`] ne bloquent jamais la lecture. Les autres méthodes publiques qui parlent au réseau
//! (connexion, [`Scrobbler::flush`]) sont bloquantes et s'appellent depuis un thread de travail.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use parking_lot::Mutex;
use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::library::SharedDb;
use crate::model::{Emit, Event, TrackId};
use crate::util::now_ms;

pub const LB_DEFAULT: &str = "https://api.listenbrainz.org";
pub const LASTFM: &str = "https://ws.audioscrobbler.com/2.0/";
const MAX_QUEUE: usize = 500;
/// Last.fm refuse les écoutes plus vieilles.
const MAX_AGE_S: i64 = 14 * 24 * 3600;
const RETRY: Duration = Duration::from_secs(5 * 60);
const BATCH: usize = 50;
const MAX_PLAYS: usize = 50;
const AGENT: &str = "control (https://github.com/nildontsleep/control)";
pub const CHANGED: &str = "scrobble:changed";

/// L'application Last.fm (clé et secret) : celle du serveur, ou celle que l'utilisateur a saisie.
pub type KeysFn = Arc<dyn Fn() -> Option<LastfmApp> + Send + Sync>;
/// Refuse les adresses internes (serveur web) : renvoie l'adresse à utiliser, ou une erreur.
pub type GuardFn = Arc<dyn Fn(&str) -> anyhow::Result<String> + Send + Sync>;

mod data;
mod protocol;

pub use data::*;
use protocol::params;
pub use protocol::*;

// ─── Scrobbler ───────────────────────────────────────────────────────────────

/// Paramètres de [`Scrobbler::new`].
pub struct ScrobblerOptions {
    /// Dossier du compte (`scrobble.json`).
    pub dir: PathBuf,
    /// Base de la bibliothèque, pour décrire un morceau.
    pub db: SharedDb,
    pub emit: Emit,
    /// L'application Last.fm du serveur (réglée par l'administrateur). `None` : celle de l'utilisateur.
    pub keys: Option<KeysFn>,
    /// Garde des adresses (serveur web). `None` : toute adresse est acceptée.
    pub guard: Option<GuardFn>,
}

enum Job {
    NowPlaying(Play),
    Flush,
    Schedule(Instant),
    Stop,
}

/// Fonction d'envoi d'un lot à un service (voir [`Scrobbler::flush_with`]).
pub type SendFn<'a> = dyn Fn(&[Queued], &Creds) -> Result<(), SendError> + 'a;

/// Ce qu'il faut pour envoyer à un service (copié sous verrou, utilisé sans).
#[derive(Debug, Clone)]
pub enum Creds {
    ListenBrainz { token: String, base: Option<String> },
    Lastfm { session: String, app: Option<LastfmApp> },
}

struct State {
    data: ScrobbleData,
    /// Écoutes en cours : identifiant de lecture → (morceau, début en secondes).
    plays: Vec<(i64, Play, i64)>,
}

struct Inner {
    file: PathBuf,
    db: SharedDb,
    emit: Emit,
    keys: Option<KeysFn>,
    guard: Option<GuardFn>,
    agent: ureq::Agent,
    state: Mutex<State>,
    seq: AtomicU64,
    flushing: Mutex<()>,
    closed: AtomicBool,
    jobs: Sender<Job>,
}

/// Envoi des écoutes à ListenBrainz et Last.fm. Clonable : toutes les copies partagent le même état.
#[derive(Clone)]
pub struct Scrobbler {
    inner: Arc<Inner>,
}

impl Scrobbler {
    pub fn new(o: ScrobblerOptions) -> Scrobbler {
        let file = o.dir.join("scrobble.json");
        let data = std::fs::read_to_string(&file).map(|t| ScrobbleData::parse(&t)).unwrap_or_default();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .user_agent(AGENT)
            .timeout_global(Some(Duration::from_secs(20)))
            .build()
            .into();
        let (tx, rx) = crossbeam_channel::unbounded();
        let inner = Arc::new(Inner {
            file,
            db: o.db,
            emit: o.emit,
            keys: o.keys,
            guard: o.guard,
            agent,
            state: Mutex::new(State { data, plays: Vec::new() }),
            seq: AtomicU64::new(1),
            flushing: Mutex::new(()),
            closed: AtomicBool::new(false),
            jobs: tx,
        });
        {
            let mut st = inner.state.lock();
            for q in &mut st.data.queue {
                q.seq = inner.seq.fetch_add(1, Ordering::Relaxed);
            }
        }
        let weak = Arc::downgrade(&inner);
        let spawned = std::thread::Builder::new().name("control-scrobble".into()).spawn(move || worker(weak, rx));
        if let Err(err) = spawned {
            log::warn!("Thread de scrobbling impossible : {err}");
        }
        Scrobbler { inner }
    }

    /// L'application Last.fm : celle du serveur, sinon celle que l'utilisateur a saisie.
    pub fn keys(&self) -> Option<LastfmApp> {
        self.server_keys().or_else(|| self.inner.state.lock().data.lastfm_app.clone())
    }

    fn server_keys(&self) -> Option<LastfmApp> {
        self.inner.keys.as_ref().and_then(|f| f()).filter(|k| !k.key.is_empty() && !k.secret.is_empty())
    }

    pub fn set_lastfm_keys(&self, key: &str, secret: &str) -> Status {
        let (k, s) = (key.trim(), secret.trim());
        self.inner.state.lock().data.lastfm_app =
            (!k.is_empty() && !s.is_empty()).then(|| LastfmApp { key: k.into(), secret: s.into() });
        self.changed();
        self.status()
    }

    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::Relaxed);
        let _ = self.inner.jobs.send(Job::Stop);
    }

    pub fn status(&self) -> Status {
        let server = self.inner.keys.as_ref().and_then(|f| f());
        let keys = self.keys();
        let st = self.inner.state.lock();
        let d = &st.data;
        let lb = d.listenbrainz.as_ref();
        let fm = d.lastfm.as_ref();
        Status {
            listenbrainz: ListenBrainzStatus {
                connected: lb.is_some_and(|a| !a.token.is_empty()),
                user: lb.and_then(|a| a.user.clone()).filter(|u| !u.is_empty()),
                url: lb.and_then(|a| a.url.clone()).filter(|u| !u.is_empty()),
                error: lb.and_then(|a| a.error.clone()).filter(|e| !e.is_empty()),
            },
            lastfm: LastfmStatus {
                connected: fm.is_some_and(|a| !a.session.is_empty()),
                user: fm.and_then(|a| a.user.clone()).filter(|u| !u.is_empty()),
                error: fm.and_then(|a| a.error.clone()).filter(|e| !e.is_empty()),
                configured: keys.is_some_and(|k| !k.key.is_empty() && !k.secret.is_empty()),
                keys_from_server: server.is_some_and(|k| !k.key.is_empty()),
            },
            pending: d.queue.len(),
        }
    }

    /// Copie des données (tests, diagnostic).
    pub fn data(&self) -> ScrobbleData {
        self.inner.state.lock().data.clone()
    }

    fn write(&self) {
        let text = {
            let st = self.inner.state.lock();
            serde_json::to_string(&st.data).unwrap_or_else(|_| "{}".into())
        };
        // Disque plein ou en lecture seule : la file reste en mémoire.
        if let Err(err) = write_private(&self.inner.file, text.as_bytes()) {
            log::warn!("Impossible d’enregistrer {} : {err}", self.inner.file.display());
        }
    }

    fn changed(&self) {
        self.write();
        let status = serde_json::to_value(self.status()).unwrap_or(Value::Null);
        (self.inner.emit)(Event::Json(CHANGED, status));
    }

    fn schedule(&self, after: Duration) {
        if self.inner.closed.load(Ordering::Relaxed) {
            return;
        }
        let _ = self.inner.jobs.send(Job::Schedule(Instant::now() + after));
    }

    fn guarded(&self, url: &str) -> anyhow::Result<String> {
        match &self.inner.guard {
            Some(g) => g(url),
            None => Ok(url.to_string()),
        }
    }

    // ─── ListenBrainz ────────────────────────────────────────────────────────

    /// Vérifie un jeton ListenBrainz et l'enregistre. `url` : serveur compatible (facultatif). Bloquant.
    pub fn connect_listenbrainz(&self, token: &str, url: Option<&str>) -> anyhow::Result<Status> {
        let clean = token.trim();
        if !valid_lb_token(clean) {
            bail!("Jeton invalide.");
        }
        let base = url.map(str::trim).filter(|u| !u.is_empty()).map(str::to_string);
        let target = match &base {
            Some(b) => self.guarded(b)?,
            None => LB_DEFAULT.to_string(),
        };
        let mut res = self
            .inner
            .agent
            .get(format!("{}/1/validate-token", target.trim_end_matches('/')))
            .header("authorization", format!("Token {clean}"))
            .config()
            .timeout_global(Some(Duration::from_secs(15)))
            .build()
            .call()
            .map_err(|_| anyhow!("Jeton refusé par ListenBrainz."))?;
        let status = res.status().as_u16();
        let body = res.body_mut().read_to_string().unwrap_or_default();
        let user = lb_validate(status, &body)?;
        self.inner.state.lock().data.listenbrainz =
            Some(ListenBrainzAccount { token: clean.into(), user, url: base, error: None });
        self.changed();
        self.schedule(Duration::from_secs(1));
        Ok(self.status())
    }

    fn lb_submit(&self, token: &str, base: Option<&str>, body: &Value) -> Result<(), SendError> {
        let base = self.guarded(base.unwrap_or(LB_DEFAULT)).map_err(|e| SendError::permanent(e.to_string()))?;
        let res = self
            .inner
            .agent
            .post(format!("{}/1/submit-listens", base.trim_end_matches('/')))
            .header("authorization", format!("Token {token}"))
            .send_json(body)
            .map_err(|e| SendError::temporary(format!("ListenBrainz : {e}")))?;
        let reset = res.headers().get("x-ratelimit-reset-in").and_then(|h| h.to_str().ok()).map(str::to_string);
        lb_result(res.status().as_u16(), reset.as_deref())
    }

    // ─── Last.fm ─────────────────────────────────────────────────────────────

    fn lastfm_call(&self, app: Option<&LastfmApp>, params: &BTreeMap<String, String>) -> Result<Value, SendError> {
        let Some(app) = app.filter(|k| !k.key.is_empty() && !k.secret.is_empty()) else {
            return Err(SendError::permanent("Clés Last.fm absentes."));
        };
        let form = lastfm_form(params, app, true);
        let mut res = self
            .inner
            .agent
            .post(LASTFM)
            .send_form(form)
            .map_err(|e| SendError::temporary(format!("Last.fm : {e}")))?;
        let status = res.status().as_u16();
        let body = res.body_mut().read_to_string().unwrap_or_default();
        lastfm_result(status, &body)
    }

    /// Première étape : un jeton et l'adresse où l'utilisateur autorise control. Bloquant.
    pub fn lastfm_start(&self) -> anyhow::Result<LastfmAuth> {
        let app = self.keys();
        let v = self.lastfm_call(app.as_ref(), &params(&[("method", "auth.getToken".into())]))?;
        let token = v.get("token").and_then(Value::as_str).context("Réponse inattendue de Last.fm.")?.to_string();
        let key = app.map(|a| a.key).unwrap_or_default();
        Ok(LastfmAuth { url: lastfm_auth_url(&key, &token), token })
    }

    /// Seconde étape, une fois l'autorisation donnée sur le site de Last.fm. Bloquant.
    pub fn lastfm_finish(&self, token: &str) -> anyhow::Result<Status> {
        let app = self.keys();
        let v =
            self.lastfm_call(app.as_ref(), &params(&[("method", "auth.getSession".into()), ("token", token.into())]))?;
        let session = v.get("session").context("Réponse inattendue de Last.fm.")?;
        let key = session.get("key").and_then(Value::as_str).context("Réponse inattendue de Last.fm.")?;
        let user = session.get("name").and_then(Value::as_str).map(str::to_string);
        self.inner.state.lock().data.lastfm = Some(LastfmAccount { session: key.into(), user, error: None });
        self.changed();
        self.schedule(Duration::from_secs(1));
        Ok(self.status())
    }

    /// Oublie un service (`listenbrainz` ou `lastfm`) et ses écoutes en attente.
    pub fn disconnect(&self, service: &str) -> Status {
        {
            let mut st = self.inner.state.lock();
            match Service::parse(service) {
                Some(Service::ListenBrainz) => st.data.listenbrainz = None,
                Some(Service::Lastfm) => st.data.lastfm = None,
                None => {}
            }
            for q in &mut st.data.queue {
                q.todo.retain(|s| s != service);
            }
            st.data.queue.retain(|q| !q.todo.is_empty());
        }
        self.changed();
        self.status()
    }

    // ─── Écoutes ─────────────────────────────────────────────────────────────

    fn has_services(&self) -> bool {
        !self.inner.state.lock().data.services().is_empty()
    }

    /// Un morceau de la bibliothèque, s'il peut être scrobblé (pas de vidéo, pas de morceau sans titre ni artiste).
    pub fn describe(&self, track: TrackId) -> Option<Play> {
        describe(&self.inner.db, track)
    }

    /// Un morceau commence : « en écoute » partout, sans attendre ni bloquer la lecture.
    pub fn started(&self, play_id: i64, track: TrackId) {
        if !self.has_services() {
            return;
        }
        let Some(play) = self.describe(track) else { return };
        {
            let mut st = self.inner.state.lock();
            st.plays.retain(|(id, ..)| *id != play_id);
            st.plays.push((play_id, play.clone(), now_ms() / 1000));
            if st.plays.len() > MAX_PLAYS {
                st.plays.remove(0);
            }
        }
        let _ = self.inner.jobs.send(Job::NowPlaying(play));
    }

    /// Pour l'API Subsonic : un lecteur externe annonce un morceau en cours.
    pub fn now_playing(&self, track: TrackId) {
        if let Some(play) = self.describe(track)
            && self.has_services()
        {
            let _ = self.inner.jobs.send(Job::NowPlaying(play));
        }
    }

    /// Pour l'API Subsonic : une écoute terminée à telle date (secondes Unix).
    pub fn submit(&self, track: TrackId, listened_at: i64) {
        if let Some(play) = self.describe(track)
            && counts(play.duration, play.duration)
        {
            self.enqueue(&play, listened_at);
        }
    }

    /// Un morceau s'arrête après `seconds` d'écoute : si l'écoute compte, elle est mise en file puis envoyée.
    pub fn ended(&self, play_id: i64, seconds: f64) {
        let rec = {
            let mut st = self.inner.state.lock();
            let pos = st.plays.iter().position(|(id, ..)| *id == play_id);
            pos.map(|i| st.plays.remove(i))
        };
        let Some((_, play, started_at)) = rec else { return };
        let seconds = if seconds.is_finite() { seconds } else { 0.0 };
        if counts(play.duration, seconds) {
            self.enqueue(&play, started_at);
        }
    }

    /// Une écoute à envoyer (aussi utilisée par l'API Subsonic : `scrobble`).
    pub fn enqueue(&self, play: &Play, listened_at: i64) {
        {
            let mut st = self.inner.state.lock();
            let todo: Vec<String> = st.data.services().iter().map(|s| s.as_str().to_string()).collect();
            if todo.is_empty() {
                return;
            }
            let play = Play {
                title: play.title.clone(),
                artist: play.artist.clone(),
                album: play.album.clone(),
                duration: if play.duration.is_finite() { play.duration.round().max(0.0) } else { 0.0 },
                track_no: play.track_no.filter(|n| *n != 0),
            };
            let seq = self.inner.seq.fetch_add(1, Ordering::Relaxed);
            st.data.queue.push(Queued { at: listened_at, play, todo, seq });
            let len = st.data.queue.len();
            if len > MAX_QUEUE {
                st.data.queue.drain(..len - MAX_QUEUE);
            }
        }
        self.changed();
        let _ = self.inner.jobs.send(Job::Flush);
    }

    /// Au démarrage : reprend l'envoi de ce qui attendait.
    pub fn resume(&self) {
        let pending = {
            let st = self.inner.state.lock();
            !st.data.queue.is_empty() && !st.data.services().is_empty()
        };
        if pending {
            self.schedule(Duration::from_secs(5));
        }
    }

    // ─── Envoi ───────────────────────────────────────────────────────────────

    fn send_now_playing(&self, play: &Play) {
        let data = self.inner.state.lock().data.clone();
        if let Some(lb) = data.listenbrainz.as_ref().filter(|a| !a.token.is_empty()) {
            let _ = self.lb_submit(&lb.token, lb.url.as_deref(), &lb_body(&[(None, play)]));
        }
        if let Some(fm) = data.lastfm.as_ref().filter(|a| !a.session.is_empty()) {
            let app = self.keys();
            let _ = self.lastfm_call(app.as_ref(), &lastfm_now_playing_params(play, &fm.session));
        }
    }

    fn send(&self, items: &[Queued], creds: &Creds) -> Result<(), SendError> {
        match creds {
            Creds::ListenBrainz { token, base } => {
                let list: Vec<(Option<i64>, &Play)> = items.iter().map(|q| (Some(q.at), &q.play)).collect();
                self.lb_submit(token, base.as_deref(), &lb_body(&list))
            }
            Creds::Lastfm { session, app } => {
                self.lastfm_call(app.as_ref(), &lastfm_scrobble_params(items, session)).map(|_| ())
            }
        }
    }

    /// Envoie ce qui attend, service par service ; ce qui échoue sans remède est abandonné, le reste est retenté
    /// plus tard (5 minutes). Bloquant.
    pub fn flush(&self) {
        self.flush_with(&|items, creds| self.send(items, creds));
    }

    /// [`Scrobbler::flush`] avec une fonction d'envoi donnée (tests). Le verrou n'est jamais gardé pendant un envoi.
    pub fn flush_with(&self, send: &SendFn<'_>) {
        let _one_at_a_time = self.inner.flushing.lock();
        let now = now_ms() / 1000;
        self.inner.state.lock().data.queue.retain(|q| now - q.at < MAX_AGE_S);
        let mut retry = false;
        for service in [Service::ListenBrainz, Service::Lastfm] {
            let name = service.as_str();
            let (items, creds) = {
                let st = self.inner.state.lock();
                let d = &st.data;
                let creds = match service {
                    Service::ListenBrainz => d
                        .listenbrainz
                        .as_ref()
                        .filter(|a| !a.token.is_empty())
                        .map(|a| Creds::ListenBrainz { token: a.token.clone(), base: a.url.clone() }),
                    Service::Lastfm => d
                        .lastfm
                        .as_ref()
                        .filter(|a| !a.session.is_empty())
                        .map(|a| Creds::Lastfm { session: a.session.clone(), app: None }),
                };
                let items: Vec<Queued> = d.queue.iter().filter(|q| q.todo.iter().any(|s| s == name)).cloned().collect();
                (items, creds)
            };
            let Some(mut creds) = creds else { continue };
            if items.is_empty() {
                continue;
            }
            if let Creds::Lastfm { app, .. } = &mut creds {
                *app = self.keys();
            }
            for batch in items.chunks(BATCH) {
                let result = send(batch, &creds);
                let mut st = self.inner.state.lock();
                let done = |st: &mut State| {
                    for q in st.data.queue.iter_mut().filter(|q| batch.iter().any(|b| b.seq == q.seq)) {
                        q.todo.retain(|s| s != name);
                    }
                };
                match result {
                    Ok(()) => {
                        done(&mut st);
                        st.data.set_error(service, None);
                    }
                    Err(err) if err.permanent => {
                        // Jeton ou session refusés : inutile d'insister, l'utilisateur doit se reconnecter.
                        st.data.set_error(service, Some(err.message));
                        done(&mut st);
                        break;
                    }
                    Err(_) => {
                        retry = true;
                        break;
                    }
                }
            }
        }
        self.inner.state.lock().data.queue.retain(|q| !q.todo.is_empty());
        self.changed();
        if retry {
            self.schedule(RETRY);
        }
    }
}

fn describe(db: &SharedDb, track: TrackId) -> Option<Play> {
    let db = db.lock();
    let row = db
        .query_row("SELECT title, artist, album, track_no, duration, kind FROM tracks WHERE id = ?", [track], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, f64>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .optional()
        .ok()
        .flatten()?;
    let (title, artist, album, track_no, duration, kind) = row;
    if kind != "audio" || title.is_empty() || artist.is_empty() {
        return None;
    }
    Some(Play { title, artist, album, duration, track_no })
}

/// Thread d'envoi : « en écoute », envois demandés et envoi programmé (un seul à la fois, le dernier l'emporte).
fn worker(weak: std::sync::Weak<Inner>, rx: Receiver<Job>) {
    let mut deadline: Option<Instant> = None;
    loop {
        let job = match deadline {
            Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                Ok(job) => job,
                Err(RecvTimeoutError::Timeout) => {
                    deadline = None;
                    Job::Flush
                }
                Err(RecvTimeoutError::Disconnected) => return,
            },
            None => match rx.recv() {
                Ok(job) => job,
                Err(_) => return,
            },
        };
        let Some(inner) = weak.upgrade() else { return };
        let s = Scrobbler { inner };
        if s.inner.closed.load(Ordering::Relaxed) {
            return;
        }
        match job {
            Job::NowPlaying(play) => s.send_now_playing(&play),
            Job::Flush => s.flush(),
            Job::Schedule(at) => deadline = Some(at),
            Job::Stop => return,
        }
    }
}

/// Écriture atomique d'un fichier lisible par son seul propriétaire (il contient des jetons).
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp);
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests;

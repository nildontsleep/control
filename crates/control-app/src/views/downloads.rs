//! Téléchargements yt-dlp (porté de `src/renderer/js/views/downloads.js`) : outils, analyse d'un lien ou
//! recherche d'un titre, options de format, file des téléchargements.
//!
//! La file vit dans le noyau (`control_core::downloads`) ; l'interface en garde une copie tenue à jour par les
//! événements `dl:update` (un téléchargement) et `dl:reset` (toute la liste). L'état des outils arrive par
//! `tools:status` et `tools:progress`.

mod composer;
mod errors;
mod queue;
mod thumbs;
mod tools;

use std::cell::Cell;

use rune::{Cx, El, ScrollSpec, TextEdit, col};
use serde_json::{Value, json};

use control_core::downloads::{AddOptions, Info, Job, JobStatus, SearchResult};
use control_core::tools::Tool;

use crate::i18n::{t, tv};
use crate::icons;
use crate::state::Ui;
use crate::theme::Palette;
use crate::ui::common::{button, kit, page_head};

pub use composer::popup;
pub use errors::friendly_error;
pub use tools::ToolsInfo;

/// Résultats demandés à une recherche (comme l'original).
const SEARCH_LIMIT: usize = 12;
/// Délai après la frappe avant d'analyser le champ (ms).
const DEBOUNCE_MS: u64 = 450;

/// Analyse du lien saisi.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Probe {
    #[default]
    Idle,
    Loading,
    Error(String),
    /// Le lien analysé et ce qu'il contient.
    Ready(Box<Info>, String),
}

/// Recherche d'un titre (texte qui n'est pas un lien).
#[derive(Debug, Clone, PartialEq)]
pub enum Found {
    Loading,
    Error(String),
    Items(Vec<SearchResult>),
}

/// Installation des outils en cours (dernier `tools:progress`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Installing {
    pub tool: String,
    pub phase: String,
    pub received: u64,
    pub total: u64,
}

#[derive(Default)]
pub struct DownloadsState {
    /// La file, dans l'ordre d'ajout.
    pub jobs: Vec<Job>,
    /// État des outils (`None` tant qu'il n'est pas connu).
    pub tools: Option<ToolsInfo>,
    pub installing: Option<Installing>,
    pub url: TextEdit,
    pub probe: Probe,
    /// Texte cherché et résultats.
    pub search: Option<(String, Found)>,
    /// Adresses des résultats déjà envoyés à la file (leur bouton est grisé).
    pub sent: Vec<String>,
    /// Liste déroulante ouverte (format, qualité, cookies…).
    pub popup: Option<&'static str>,
    /// Numéro de la dernière frappe (anti-rebond de l'analyse).
    edits: u64,
    /// Liste et outils demandés au noyau (une fois).
    started: Cell<bool>,
    thumbs: thumbs::Thumbs,
}

impl DownloadsState {
    /// Téléchargements en cours (pastille de la barre latérale).
    pub fn active(&self) -> usize {
        self.jobs.iter().filter(|j| j.status.is_active()).count()
    }

    /// yt-dlp est installé.
    pub fn ytdlp_ready(&self) -> bool {
        self.tools.as_ref().is_some_and(|s| s.ytdlp.installed)
    }

    /// Remplace un téléchargement (ou l'ajoute) ; renvoie son état précédent.
    fn patch_job(&mut self, job: Job) -> Option<JobStatus> {
        match self.jobs.iter_mut().find(|j| j.id == job.id) {
            Some(slot) => Some(std::mem::replace(slot, job).status),
            None => {
                self.jobs.push(job);
                None
            }
        }
    }
}

/// Demande au noyau la file et l'état des outils, puis lance la mise à jour quotidienne de yt-dlp (réglage
/// `dl.autoUpdate`). Une seule fois : à la première ouverture de la page, au premier événement `dl:*`/`tools:*`
/// ou au premier affichage de la vue (l'application peut s'ouvrir directement sur cette page).
fn ensure_started(ui: &Ui) {
    if ui.downloads.started.replace(true) {
        return;
    }
    ui.spawn(
        |b| {
            b.tools.refresh_versions();
            let status = serde_json::to_value(b.tools.status()).unwrap_or(Value::Null);
            // La liste en dernier : elle est la plus fraîche possible quand elle arrive.
            (b.downloads.list(), status)
        },
        |s, (jobs, status), _| {
            s.downloads.jobs = jobs;
            s.downloads.tools = Some(ToolsInfo::parse(&status));
            // `tools:updated` (et `tools:status`) arrivent par les événements si yt-dlp a été mis à jour.
            s.spawn_quiet(|b| {
                let _ = b.tools.auto_update(&b.store);
            });
        },
    );
}

pub fn on_open(ui: &mut Ui) {
    ensure_started(ui);
}

/// Événements `dl:*` et `tools:*` du noyau.
pub fn on_event(ui: &mut Ui, channel: &str, payload: &Value) {
    ensure_started(ui);
    match channel {
        "dl:update" => {
            let Ok(job) = serde_json::from_value::<Job>(payload.clone()) else { return };
            let done = job.status == JobStatus::Done;
            let title = job.title.clone();
            let current = job.current.clone();
            let before = ui.downloads.patch_job(job);
            // Comme `app.js` : un mot quand un téléchargement suivi se termine.
            if done && before.is_some_and(|b| b != JobStatus::Done) {
                let name = if title.is_empty() { current.unwrap_or_default() } else { title };
                let msg = if name.is_empty() { t("dl.doneGeneric") } else { tv("dl.doneToast", &[("title", &name)]) };
                ui.toast(msg, "circle-check");
            }
        }
        "dl:reset" => {
            if let Ok(jobs) = serde_json::from_value::<Vec<Job>>(payload.clone()) {
                ui.downloads.jobs = jobs;
            }
        }
        "tools:status" => ui.downloads.tools = Some(ToolsInfo::parse(payload)),
        "tools:progress" => {
            let phase = payload.get("phase").and_then(Value::as_str).unwrap_or_default();
            if phase == "done" || phase == "error" {
                return;
            }
            let num = |k: &str| payload.get(k).and_then(Value::as_u64).unwrap_or(0);
            ui.downloads.installing = Some(Installing {
                tool: payload.get("tool").and_then(Value::as_str).unwrap_or("ytdlp").to_string(),
                phase: phase.to_string(),
                received: num("received"),
                total: num("total"),
            });
        }
        // Mise à jour quotidienne : un mot seulement si quelque chose a changé.
        "tools:updated" if payload.get("updated").and_then(Value::as_bool) == Some(true) => {
            let v = payload.get("version").and_then(Value::as_str).unwrap_or_default().to_string();
            ui.toast(tv("tools.updated", &[("v", &v)]), "check");
        }
        _ => {}
    }
}

// ─── Actions ─────────────────────────────────────────────────────────────────

/// Installe yt-dlp, ffmpeg et deno (ceux qui manquent), sur un thread.
pub fn install_tools(ui: &mut Ui) {
    if ui.downloads.installing.is_some() {
        return;
    }
    ui.downloads.installing = Some(Installing { tool: "ytdlp".into(), phase: "download".into(), ..Default::default() });
    ui.spawn(
        |b| {
            let res = b.tools.install(&Tool::ALL).map_err(|e| format!("{e:#}"));
            (res, serde_json::to_value(b.tools.status()).unwrap_or(Value::Null))
        },
        |s, (res, status), _| {
            s.downloads.installing = None;
            s.downloads.tools = Some(ToolsInfo::parse(&status));
            match res {
                Ok(()) => s.toast(t("tools.installed"), "circle-check"),
                Err(e) if e.is_empty() => s.toast(t("tools.failed"), "triangle-alert"),
                Err(e) => s.toast(e, "triangle-alert"),
            }
            if !s.downloads.url.text.trim().is_empty() {
                analyze(s);
            }
        },
    );
}

/// Met yt-dlp à jour (bouton), avec un mot sur le résultat.
pub fn update_tools(ui: &mut Ui) {
    ui.toast(t("tools.updating"), "refresh-cw");
    ui.spawn(
        |b| b.tools.update(),
        |s, r, _| {
            if r.error.is_some() {
                s.toast(t("tools.updateFailed"), "triangle-alert");
            } else if r.updated {
                s.toast(tv("tools.updated", &[("v", &r.version.unwrap_or_default())]), "check");
            } else {
                s.toast(t("tools.upToDate"), "check");
            }
        },
    );
}

fn is_url(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

/// `scheme://…` d'un autre protocole (refusé, pas cherché).
fn has_scheme(s: &str) -> bool {
    s.split_once("://").is_some_and(|(a, _)| !a.is_empty() && a.chars().all(|c| c.is_ascii_alphabetic()))
}

/// Le texte du champ a changé : analyse après un court délai (la frappe continue peut-être).
pub fn url_changed(ui: &mut Ui) {
    ui.downloads.edits += 1;
    let seq = ui.downloads.edits;
    ui.spawn(
        |_| std::thread::sleep(std::time::Duration::from_millis(DEBOUNCE_MS)),
        move |s, (), _| {
            if s.downloads.edits == seq {
                analyze(s);
            }
        },
    );
}

/// Entrée dans le champ : télécharge ce qui est analysé, sinon analyse tout de suite.
pub fn url_submitted(ui: &mut Ui) {
    ui.downloads.edits += 1;
    if matches!(ui.downloads.probe, Probe::Ready(..)) {
        start(ui);
    } else {
        analyze(ui);
    }
}

/// Analyse le champ : un lien est examiné par yt-dlp, un autre texte cherché sur SoundCloud ou YouTube.
pub fn analyze(ui: &mut Ui) {
    let text = ui.downloads.url.text.trim().to_string();
    ui.downloads.edits += 1;
    ui.downloads.popup = None;
    if text.chars().count() >= 2 && !is_url(&text) && !has_scheme(&text) {
        ui.downloads.probe = Probe::Idle;
        search(ui, text);
        return;
    }
    ui.downloads.search = None;
    if !is_url(&text) {
        ui.downloads.probe = if text.is_empty() { Probe::Idle } else { Probe::Error(t("dl.invalidUrl")) };
        return;
    }
    if !ui.downloads.ytdlp_ready() {
        ui.downloads.probe = Probe::Error(t("dl.needTools"));
        return;
    }
    ui.downloads.probe = Probe::Loading;
    let url = text.clone();
    ui.spawn(
        move |b| b.downloads.info(&url).map_err(|e| e.to_string()),
        move |s, res, _| {
            if s.downloads.url.text.trim() != text {
                return;
            }
            s.downloads.probe = match res {
                Ok(info) => {
                    let probe = Probe::Ready(Box::new(info), text);
                    thumbs::prefetch(s, probe_thumbnail(&probe));
                    probe
                }
                Err(e) => Probe::Error(error_text(&e, "dl.analyzeFailed")),
            };
        },
    );
}

/// Dernière ligne d'une erreur de yt-dlp, rendue lisible (ou le message par défaut).
fn error_text(raw: &str, fallback: &str) -> String {
    let last = raw.lines().last().unwrap_or_default();
    let msg = friendly_error(last);
    if msg.trim().is_empty() { t(fallback) } else { msg }
}

fn probe_thumbnail(p: &Probe) -> Option<&str> {
    match p {
        Probe::Ready(info, _) => match info.as_ref() {
            Info::Video(v) => v.thumbnail.as_deref(),
            Info::Playlist(pl) => pl.thumbnail.as_deref(),
        },
        _ => None,
    }
}

/// Source de recherche choisie (`dl.searchSource`).
pub fn search_source(ui: &Ui) -> &'static str {
    if ui.settings.dl.search_source == "youtube" { "youtube" } else { "soundcloud" }
}

/// Cherche `text` sur la source choisie.
pub fn search(ui: &mut Ui, text: String) {
    if !ui.downloads.ytdlp_ready() {
        ui.downloads.search = Some((text, Found::Error(t("dl.needTools"))));
        return;
    }
    ui.downloads.search = Some((text.clone(), Found::Loading));
    let source = search_source(ui);
    let query = text.clone();
    ui.spawn(
        move |b| b.downloads.search(source, &query, SEARCH_LIMIT).map_err(|e| e.to_string()),
        move |s, res, _| {
            let still =
                s.downloads.search.as_ref().is_some_and(|(q, _)| *q == text) && s.downloads.url.text.trim() == text;
            if !still || search_source(s) != source {
                return;
            }
            let found = match res {
                Ok(items) => Found::Items(items),
                Err(e) => Found::Error(error_text(&e, "dl.searchFailed")),
            };
            s.downloads.search = Some((text, found));
        },
    );
}

/// Format et qualité choisis, pour `downloads.add`.
fn format_options(ui: &Ui) -> Value {
    let d = &ui.settings.dl;
    let audio = d.kind != "video";
    json!({
        "kind": if audio { "audio" } else { "video" },
        "format": d.audio_format,
        "quality": if audio { &d.audio_quality } else { &d.video_quality },
    })
}

fn add(ui: &mut Ui, mut opts: Value) {
    if let (Some(o), Value::Object(f)) = (opts.as_object_mut(), format_options(ui)) {
        o.extend(f);
    }
    let opts = AddOptions::from_value(&opts);
    ui.spawn_quiet(move |b| {
        b.downloads.add(opts);
    });
    ui.toast(t("dl.added"), "arrow-down-to-line");
}

/// Télécharge ce que l'analyse a trouvé, puis vide le champ.
pub fn start(ui: &mut Ui) {
    if !matches!(ui.downloads.probe, Probe::Ready(..)) {
        return;
    }
    let Probe::Ready(info, url) = std::mem::take(&mut ui.downloads.probe) else { return };
    let opts = match *info {
        Info::Video(v) => json!({
            "url": url,
            "title": v.title,
            "thumbnail": v.thumbnail,
            "isPlaylist": false,
            "count": 1,
            "parseTitle": !v.music,
            "createPlaylist": false,
            "playlistName": v.title,
        }),
        Info::Playlist(pl) => json!({
            "url": url,
            "title": pl.title,
            "thumbnail": pl.thumbnail,
            "isPlaylist": true,
            "count": pl.count.max(1),
            "parseTitle": !pl.music,
            "createPlaylist": ui.settings.dl.create_playlist,
            "playlistName": pl.title,
        }),
    };
    add(ui, opts);
    ui.downloads.url = TextEdit::new("");
    ui.downloads.edits += 1;
}

/// Télécharge un résultat de recherche.
pub fn start_result(ui: &mut Ui, item: &SearchResult) {
    // SoundCloud donne déjà artiste et titre séparés ; sur YouTube, « Artiste - Titre » est à découper.
    let opts = json!({
        "url": item.url,
        "title": item.title,
        "thumbnail": item.thumbnail,
        "isPlaylist": false,
        "count": 1,
        "parseTitle": item.source == "youtube",
        "createPlaylist": false,
    });
    ui.downloads.sent.push(item.url.clone());
    add(ui, opts);
}

/// Change la source de recherche et relance la recherche en cours.
pub fn set_search_source(ui: &mut Ui, source: String) {
    ui.set(json!({ "dl": { "searchSource": source } }));
    if let Some((q, _)) = ui.downloads.search.clone() {
        search(ui, q);
    }
}

// ─── Vue ─────────────────────────────────────────────────────────────────────

pub fn view(ui: &Ui, p: &Palette, _cx: &mut Cx<Ui>) -> El {
    ensure_started(ui);
    let k = kit(p);
    let mut layer: Vec<El> = Vec::new();
    let open =
        button(p, "dl-open-folder", &t("dl.openFolder"), Some(&icons::FOLDER_OPEN), false).on_press(|s: &mut Ui| {
            let dir = s.backend.downloads.download_dir();
            let _ = std::fs::create_dir_all(&dir);
            crate::os::open_path(&dir.to_string_lossy());
        });
    let body = col()
        .w_full()
        .px(32.0)
        .pb(120.0)
        .child(page_head(p, &t("nav.downloads"), Some(t("dl.subtitle")), Some(open)))
        .child(composer::composer(ui, p, &k, &mut layer))
        .child(tools::tools_box(ui, p))
        .child(queue::queue(ui, p));
    col()
        .key("downloads")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(
            col()
                .key("dl-scroll")
                .w_full()
                .h_full()
                .min_h(0.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .scroll_edge_fade(8.0)
                .child(body),
        )
        .children(layer)
}

/// Rendus sans fenêtre : remplit la page d'exemples (`--scene downloads-…`).
pub fn scene(ui: &mut Ui, which: &str) {
    scenes::apply(ui, which);
}

mod scenes;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_schemes() {
        assert!(is_url("https://youtu.be/x"));
        assert!(is_url("HTTP://a"));
        assert!(!is_url("daft punk"));
        assert!(has_scheme("ftp://x"));
        assert!(!has_scheme("daft punk"));
        assert!(!has_scheme("a b://c"));
    }

    #[test]
    fn active_counts_running_and_queued() {
        let mut st = DownloadsState::default();
        for (id, status) in
            [(1, JobStatus::Queued), (2, JobStatus::Downloading), (3, JobStatus::Done), (4, JobStatus::Error)]
        {
            st.jobs.push(Job { id, status, ..Job::default() });
        }
        assert_eq!(st.active(), 2);
        let before = st.patch_job(Job { id: 2, status: JobStatus::Done, ..Job::default() });
        assert_eq!(before, Some(JobStatus::Downloading));
        assert_eq!(st.active(), 1);
        assert_eq!(st.patch_job(Job { id: 9, status: JobStatus::Queued, ..Job::default() }), None);
        assert_eq!(st.active(), 2);
    }
}

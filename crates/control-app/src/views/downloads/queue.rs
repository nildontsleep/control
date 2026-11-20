//! File des téléchargements : en cours (avec progression, vitesse, temps restant), puis terminés.

use rune::{Cursor, El, SurfaceFit, col, el, icon, row, text};

use control_core::downloads::{Job, JobStatus};
use control_core::model::Kind;

use super::tools::{bar, error_color, link_button};
use super::{friendly_error, thumbs};
use crate::format;
use crate::i18n::{t, tn, tv};
use crate::icons;
use crate::state::{Route, Ui};
use crate::theme::Palette;
use crate::ui::common::ty;

const THUMB_W: f32 = 76.0;

/// Vignette d'un téléchargement (16:9), ou l'icône de son genre.
pub fn thumb(ui: &Ui, p: &Palette, url: Option<&str>, w: f32, h: f32, radius: f32, glyph: &'static rune::Icon) -> El {
    let base = el().size(w, h).radius(p.r(radius)).clip_rounded(p.r(radius)).fill(p.fill);
    match url.and_then(|u| thumbs::get(ui, u)) {
        Some(img) => base.child(el().w_full().h_full().pixels_fit(img.pixels(), SurfaceFit::Fill)),
        None => base.center().child(icon(glyph, (h * 0.45).clamp(14.0, 26.0)).color(p.text3)),
    }
}

/// Ligne d'état sous le titre (comme `meta()` de l'original).
fn meta(job: &Job) -> String {
    match job.status {
        JobStatus::Downloading => {
            let mut parts = Vec::new();
            if let (Some(n), Some(i)) = (job.count, job.index)
                && n > 1
                && i > 0
            {
                parts.push(tv("dl.itemOf", &[("i", &i), ("n", &n)]));
            }
            if let Some(s) = job.speed.filter(|s| *s > 0.0) {
                parts.push(format!("{}/s", format::bytes(s)));
            }
            if let Some(e) = job.eta.filter(|e| *e > 0.0) {
                parts.push(tv("dl.eta", &[("t", &format::time(e))]));
            }
            if parts.is_empty() { t("dl.starting") } else { parts.join(" · ") }
        }
        JobStatus::Processing => t("dl.processing"),
        JobStatus::Queued | JobStatus::Info => t("dl.queued"),
        JobStatus::Done => match job.skipped.filter(|n| *n > 0) {
            Some(n) => {
                tv("dl.partial", &[("n", &n), ("reason", &friendly_error(job.error.as_deref().unwrap_or_default()))])
            }
            None => tn("dl.doneTracks", job.files.len()),
        },
        JobStatus::Canceled => t("dl.canceled"),
        JobStatus::Error => {
            let msg = friendly_error(job.error.as_deref().unwrap_or_default());
            if msg.trim().is_empty() { t("dl.failed") } else { msg }
        }
    }
}

fn action(p: &Palette, key: String, glyph: &'static rune::Icon, tip: String) -> El {
    row()
        .key(key)
        .size(30.0, 30.0)
        .radius(15.0)
        .center()
        .cursor(Cursor::Pointer)
        .hover(p.fill_hover)
        .role("button")
        .label(tip.clone())
        .help(tip)
        .child(icon(glyph, 15.0).color(p.text2))
}

fn job_row(ui: &Ui, p: &Palette, job: &Job) -> El {
    let id = job.id;
    let active = job.status.is_active();
    let pct = (job.progress.clamp(0.0, 1.0) * 100.0).round() as i64;
    let glyph = if job.kind == Kind::Video { &icons::FILM } else { &icons::MUSIC_2 };
    let title = job.current.clone().filter(|c| !c.is_empty()).unwrap_or_else(|| job.title.clone());
    let sub = match &job.current {
        Some(c) if !c.is_empty() && *c != job.title && job.is_playlist => format!("{} · {}", job.title, meta(job)),
        _ => meta(job),
    };
    let sub_color = if job.status == JobStatus::Error { error_color() } else { p.text2 };
    let mut main = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(text(title).font(ty::BODY_MEDIUM).color(p.text).truncate())
        .child(text(sub).font(ty::SMALL).color(sub_color).truncate());
    if active {
        let frac = match job.status {
            JobStatus::Processing => None,
            _ => Some(job.progress as f32),
        };
        main = main.child(bar(p, frac).mt(6.0));
    }
    let status = match job.status {
        JobStatus::Done => icon(&icons::CIRCLE_CHECK, 18.0).color(p.success),
        JobStatus::Error => icon(&icons::CIRCLE_ALERT, 18.0).color(error_color()),
        JobStatus::Downloading => text(format!("{pct}%")).font(ty::NUMBERS).color(p.text2),
        _ => el(),
    };
    let mut actions = row().gap(2.0).items_center();
    if active {
        actions = actions.child(
            action(p, format!("dl-cancel-{id}"), &icons::X, t("common.cancel"))
                .on_press(move |s: &mut Ui| s.spawn_quiet(move |b| b.downloads.cancel(id))),
        );
    }
    if matches!(job.status, JobStatus::Error | JobStatus::Canceled) {
        actions = actions.child(
            action(p, format!("dl-retry-{id}"), &icons::REFRESH_CW, t("dl.retry"))
                .on_press(move |s: &mut Ui| s.spawn_quiet(move |b| b.downloads.retry(id))),
        );
    }
    if job.status == JobStatus::Done && !job.track_ids.is_empty() {
        let ids = job.track_ids.clone();
        actions = actions.child(action(p, format!("dl-play-{id}"), &icons::PLAY, t("common.play")).on_press(
            move |s: &mut Ui| {
                let found: Vec<i64> = ids.iter().copied().filter(|i| s.cat.by_id.contains_key(i)).collect();
                if found.is_empty() {
                    s.toast(t("dl.notYetInLibrary"), "info");
                    return;
                }
                let (cat, b) = (s.cat.clone(), s.backend.clone());
                s.player.play_list(&found, 0, Some(false), &cat, &b);
            },
        ));
        if let Some(pl) = job.playlist_id {
            actions = actions.child(
                action(p, format!("dl-pl-{id}"), &icons::LIST_MUSIC, t("dl.openPlaylist"))
                    .on_press(move |s: &mut Ui| s.navigate(Route::Playlist(pl))),
            );
        }
    }
    if !active {
        actions = actions.child(
            action(p, format!("dl-remove-{id}"), &icons::TRASH_2, t("common.remove"))
                .on_press(move |s: &mut Ui| s.spawn_quiet(move |b| b.downloads.remove(id))),
        );
    }
    row()
        .key(format!("dl-job-{id}"))
        .w_full()
        .gap(14.0)
        .px(12.0)
        .py(10.0)
        .items_center()
        .radius(p.r(14.0))
        .hover(p.fill)
        .child(thumb(ui, p, job.thumbnail.as_deref(), THUMB_W, (THUMB_W * 9.0 / 16.0).round(), 8.0, glyph))
        .child(main)
        .child(row().w(40.0).justify_end().child(status))
        .child(actions)
}

fn block_title(p: &Palette, label: String) -> El {
    text(label).font(ty::TITLE).color(p.text).mb(14.0)
}

pub fn queue(ui: &Ui, p: &Palette) -> El {
    let jobs = &ui.downloads.jobs;
    let q = col().key("dl-queue").w_full().mt(8.0);
    if jobs.is_empty() {
        return q.child(
            col()
                .w_full()
                .py(56.0)
                .gap(10.0)
                .items_center()
                .child(icon(&icons::CLOUD_DOWNLOAD, 28.0).color(p.text3))
                .child(text(t("dl.emptyQueue")).font(ty::BODY).color(p.text3)),
        );
    }
    let (active, finished): (Vec<&Job>, Vec<&Job>) = jobs.iter().partition(|j| j.status.is_active());
    let mut q = q;
    if !active.is_empty() {
        q = q.child(block_title(p, t("dl.inProgress")).mt(28.0));
        for j in &active {
            q = q.child(job_row(ui, p, j).mb(2.0));
        }
    }
    if !finished.is_empty() {
        let clear = link_button(p, "dl-clear", t("dl.clear"), None)
            .on_press(|s: &mut Ui| s.spawn_quiet(|b| b.downloads.clear_finished()))
            .mb(14.0);
        q = q.child(
            row()
                .w_full()
                .mt(28.0)
                .items_center()
                .justify_between()
                .child(block_title(p, t("dl.finished")))
                .child(clear),
        );
        for j in finished.iter().rev() {
            q = q.child(job_row(ui, p, j).mb(2.0));
        }
    }
    q
}

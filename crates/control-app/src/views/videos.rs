//! Vidéos de la bibliothèque : grille de vignettes 16:9.

use std::rc::Rc;

use rune::{Cx, El, SurfaceFit, el, row, text};
use serde_json::json;

use control_core::catalog::sort_videos;
use control_core::model::TrackId;

use crate::format;
use crate::i18n::{t, tn};
use crate::icons;
use crate::state::{MenuTarget, OpenMenu, Route, Ui};
use crate::theme::Palette;
use crate::ui::common::{CoverKind, button, cover, empty_state, page_head, ty};
use crate::ui::grid::{card, grid};
use crate::views::segmented;
use crate::views::songs::play_buttons;

const SORTS: &[&str] = &["addedAt", "title", "duration", "playCount"];

/// Vignette d'une vidéo (image extraite par ffmpeg, sinon dégradé).
pub fn thumb(ui: &Ui, p: &Palette, id: TrackId, w: f32, h: f32) -> El {
    let Some(e) = ui.cat.entry(id) else { return el().size(w, h) };
    let tr = &e.track;
    let base = el().size(w, h).radius(p.r(10.0)).clip_rounded(p.r(10.0));
    let inner = match ui.video_thumb(id) {
        Some(img) => el().w_full().h_full().pixels_fit(img.pixels(), SurfaceFit::Fill),
        None => cover(ui, None, &tr.title, w.max(h), 0.0, CoverKind::Video).size(w, h),
    };
    base.child(inner).child(
        row()
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .items_end()
            .justify_end()
            .padding(8.0)
            .child(
                text(format::time(tr.duration))
                    .font(ty::CAPTION)
                    .color(rune::Color::WHITE)
                    .px(6.0)
                    .py(2.0)
                    .fill(rune::Color::BLACK.alpha(0.55))
                    .radius(5.0),
            ),
    )
}

pub fn view(ui: &Ui, p: &Palette, cx: &mut Cx<Ui>) -> El {
    if ui.ready && ui.cat.videos.is_empty() {
        return rune::col().w_full().h_full().px(32.0).child(page_head(p, &t("nav.videos"), None, None)).child(empty_state(
            p,
            &icons::FILM,
            &t("empty.videosTitle"),
            &t("empty.videosText"),
            vec![button(p, "empty-dl", &t("empty.download"), Some(&icons::CLOUD_DOWNLOAD), true).on_press(|s: &mut Ui| s.navigate(Route::Downloads))],
        ));
    }
    let mut order = ui.cat.videos.clone();
    sort_videos(&ui.cat, &mut order, &ui.settings.sort.videos);
    let ids: Rc<Vec<TrackId>> = Rc::new(order.iter().map(|&i| ui.cat.entries[i].track.id).collect());
    let n = ids.len();
    let head_ids = ids.clone();
    grid(
        cx,
        "videos",
        n,
        260.0,
        52.0 - 260.0 * (1.0 - 9.0 / 16.0),
        move |s, p| {
            let secs: f64 = head_ids.iter().filter_map(|id| s.cat.track(*id)).map(|t| t.duration).sum();
            let picker = segmented(
                p,
                "videos-sort",
                SORTS.iter().map(|k| (k.to_string(), t(&format!("sort.{k}")))).collect(),
                &s.settings.sort.videos,
                |s: &mut Ui, v: String| s.set(json!({ "sort": { "videos": v } })),
            );
            page_head(
                p,
                &t("nav.videos"),
                Some(format!("{} · {}", tn("count.videos", head_ids.len()), format::long(secs))),
                Some(row().gap(10.0).items_center().child(picker).child(play_buttons(p, head_ids.clone()))),
            )
        },
        move |s, p, i, w| {
            let id = ids[i];
            let Some(e) = s.cat.entry(id) else { return el() };
            let tr = &e.track;
            let sub = [tr.height.map(|h| format!("{h}p")), tr.codec.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
            let list = ids.clone();
            let list2 = ids.clone();
            card(p, format!("v-{id}"), thumb(s, p, id, w, w * 9.0 / 16.0), w, tr.title.clone(), sub, false, false, move |s: &mut Ui| {
                let (cat, b) = (s.cat.clone(), s.backend.clone());
                s.player.play_list(&list, i, Some(false), &cat, &b);
                s.now_playing = true;
            })
            .on_press(move |s: &mut Ui| {
                let (cat, b) = (s.cat.clone(), s.backend.clone());
                s.player.play_list(&list2, i, Some(false), &cat, &b);
                s.now_playing = true;
            })
            .on_secondary_press(move |s: &mut Ui, at, _| {
                s.menu = Some(OpenMenu { at, target: MenuTarget::Tracks { ids: vec![id], entries: vec![], playlist: None }, submenu: false });
            })
        },
        p,
    )
}

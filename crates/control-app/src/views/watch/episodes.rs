//! Épisodes de la fiche : saisons (séries TMDB) ou groupes de 50 (animés), avec la progression de chacun.

use rune::{Color, Cursor, El, SurfaceFit, TextStyle, col, el, icon, row, text};

use control_core::media::Details;

use super::cards::{self, error_state};
use super::title::{BLOCK_TITLE, EP_GROUP, short_date};
use super::{Load, load_season, play};
use crate::format;
use crate::i18n::{t, tv};
use crate::icons;
use crate::state::Ui;
use crate::theme::Palette;
use crate::ui::common::ty;

/// Un épisode à afficher, quel que soit le catalogue.
struct Ep {
    number: i64,
    name: String,
    overview: String,
    still: Option<String>,
    air_date: String,
    runtime: Option<i64>,
}

pub fn episodes(ui: &Ui, p: &Palette, kind: &str, id: &str, d: &Details) -> El {
    let page_state = &ui.watch.title;
    let mut c = col()
        .key("episodes")
        .w_full()
        .mt(34.0)
        .child(text(t("title.episodesHeading")).font(BLOCK_TITLE).color(p.text).mb(14.0));
    let list: Vec<El> = match d {
        Details::Tmdb(td) => {
            if td.seasons.is_empty() {
                return el();
            }
            let current = page_state.season.or_else(|| td.seasons.first().map(|s| s.number)).unwrap_or(1);
            let many = td.seasons.len() > 12;
            let tabs = td
                .seasons
                .iter()
                .map(|s| {
                    let label = if many {
                        format!("S{}", s.number)
                    } else if s.name.is_empty() {
                        format!("{} {}", t("season.word"), s.number)
                    } else {
                        s.name.clone()
                    };
                    let n = s.number;
                    let press: Box<dyn Fn(&mut Ui)> = Box::new(move |s: &mut Ui| {
                        s.watch.title.season = Some(n);
                        load_season(s, n);
                    });
                    (label, n == current, press)
                })
                .collect();
            c = c.child(cards::chips(p, "season-tabs", tabs).mb(14.0));
            match page_state.seasons.get(&current) {
                None | Some(Load::Loading) => (0..4).map(|i| skeleton_row(p, i)).collect(),
                Some(Load::Failed(err)) => {
                    vec![error_state(p, "season-retry", err, move |s: &mut Ui| {
                        s.watch.title.seasons.remove(&current);
                        load_season(s, current);
                    })]
                }
                Some(Load::Ready(season)) => season
                    .episodes
                    .iter()
                    .map(|e| Ep {
                        number: e.number,
                        name: e.name.clone(),
                        overview: e.overview.clone(),
                        still: e.still.clone(),
                        air_date: e.air_date.clone(),
                        runtime: e.runtime,
                    })
                    .map(|e| episode_row(ui, p, kind, id, Some(current), &e))
                    .collect(),
            }
        }
        Details::Anime(a) => {
            if a.episodes.is_empty() {
                return el();
            }
            let groups = a.episodes.len().div_ceil(EP_GROUP);
            let group = page_state.group.min(groups - 1);
            if groups > 1 {
                let tabs = (0..groups)
                    .map(|g| {
                        let label = format!("{}–{}", g * EP_GROUP + 1, a.episodes.len().min((g + 1) * EP_GROUP));
                        let press: Box<dyn Fn(&mut Ui)> = Box::new(move |s: &mut Ui| s.watch.title.group = g);
                        (label, g == group, press)
                    })
                    .collect();
                c = c.child(cards::chips(p, "ep-groups", tabs).mb(14.0));
            }
            a.episodes
                .iter()
                .skip(group * EP_GROUP)
                .take(EP_GROUP)
                .map(|e| Ep {
                    number: e.number,
                    name: e.name.clone(),
                    overview: String::new(),
                    still: e.still.clone(),
                    air_date: String::new(),
                    runtime: a.runtime,
                })
                .map(|e| episode_row(ui, p, kind, id, None, &e))
                .collect()
        }
    };
    c.child(col().w_full().children(list))
}

fn skeleton_row(p: &Palette, i: usize) -> El {
    row()
        .key(format!("ep-sk-{i}"))
        .w_full()
        .gap(18.0)
        .padding(12.0)
        .child(el().size(176.0, 99.0).radius(p.r(10.0)).fill(p.fill))
        .child(el().w(240.0).h(16.0).mt(4.0).radius(p.r(6.0)).fill(p.fill))
}

fn episode_row(ui: &Ui, p: &Palette, kind: &str, id: &str, season: Option<i64>, ep: &Ep) -> El {
    let done = ui.watch.title.progress.iter().find(|r| r.season == season && r.episode == Some(ep.number));
    let frac = done.filter(|r| r.duration > 0.0).map_or(0.0, |r| (r.position / r.duration).clamp(0.0, 1.0));
    let (w, h) = (176.0, 99.0);
    let r = p.r(10.0);
    let mut art_box = el().size(w, h).radius(r).clip_rounded(r).fill(p.fill_strong);
    art_box = match ui.watch.images.get(ui, ep.still.as_deref()) {
        Some(img) => art_box.child(el().w_full().h_full().pixels_fit(img.pixels(), SurfaceFit::Fill)),
        None => art_box.child(
            row()
                .w_full()
                .h_full()
                .center()
                .child(text(ep.number.to_string()).font(TextStyle::new(30.0, 760.0)).color(p.text3)),
        ),
    };
    art_box = art_box.child(
        row()
            .absolute(0.0, 0.0)
            .size(w, h)
            .center()
            .fill(Color::BLACK.alpha(0.35))
            .group_opacity(0.0, 1.0)
            .child(icon(&icons::PLAY, 18.0).color(Color::WHITE)),
    );
    if let Some(row_) = done
        && !row_.finished
        && frac > 0.0
    {
        art_box = art_box.child(cards::progress_bar(p, frac, w, h, false));
    }
    let name = if ep.name.is_empty() { format!("{} {}", t("ep.word"), ep.number) } else { ep.name.clone() };
    let mut title = row()
        .gap(6.0)
        .items_center()
        .child(text(format!("{}.", ep.number)).font(TextStyle::new(14.0, 640.0).features("'tnum' 1")).color(p.text3))
        .child(text(name).font(TextStyle::new(14.0, 640.0)).color(p.text).truncate().shrink().min_w(0.0));
    if done.is_some_and(|r| r.finished) {
        title = title.child(icon(&icons::CIRCLE_CHECK, 15.0).color(p.success).ml(2.0));
    }
    let meta = [ep.runtime.map(|m| format!("{m} min")), short_date(&ep.air_date)]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let mut txt = col().grow(1.0).min_w(0.0).pt(2.0).child(title);
    if !meta.is_empty() {
        txt = txt.child(text(meta).font(ty::SMALL).color(p.text3).mt(1.0));
    }
    if !ep.overview.is_empty() {
        txt = txt.child(
            text(ep.overview.clone())
                .font(TextStyle::new(12.5, 400.0).lh(18.0))
                .color(p.text2)
                .max_lines(2)
                .max_w(760.0)
                .mt(6.0),
        );
    }
    if let Some(r) = done.filter(|r| !r.finished && r.position >= 30.0) {
        txt = txt.child(
            text(tv("ep.resumeAt", &[("t", &format::time(r.position))])).font(ty::SMALL_MEDIUM).color(p.accent).mt(6.0),
        );
    }
    let (k, i, n) = (kind.to_string(), id.to_string(), ep.number);
    row()
        .key(format!("ep-{}-{}", season.unwrap_or(0), ep.number))
        .w_full()
        .gap(18.0)
        .items_start()
        .px(12.0)
        .py(10.0)
        .radius(p.r(14.0))
        .hover(p.fill)
        .hover_group()
        .cursor(Cursor::Pointer)
        .role("button")
        .child(art_box)
        .child(txt)
        .on_press(move |s: &mut Ui| play::open(s, &k, &i, season, Some(n)))
}

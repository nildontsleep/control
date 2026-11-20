//! « Ma liste » : reprise, titres enregistrés et historique de visionnage (port de `views/mylist.js`).

use rune::{Cursor, Cx, El, SurfaceFit, TextStyle, col, el, icon, row, text};

use control_core::media::Progress;

use super::cards::{Item, RESUME_W, episode_tag, kind_label, resume_card, shelf, title_route};
use super::home::content_width;
use super::shelf::posters;
use super::{WatchRoute, load_local, page};
use crate::i18n::{self, t};
use crate::icons;
use crate::state::{Route, Ui};
use crate::theme::Palette;
use crate::ui::common::{button, empty_state, page_head, ty};

const BLOCK_TITLE: TextStyle = TextStyle::new(20.0, 700.0).tracking(-0.4);

/// « il y a 3 jours », « 3 days ago » (comme `Intl.RelativeTimeFormat` avec `numeric: 'auto'`).
pub fn relative(ms: i64, now: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    let ago = (now - ms).max(0);
    let minutes = ago / 60_000;
    let hours = ago / 3_600_000;
    let days = ago / 86_400_000;
    let fr = i18n::current() == i18n::Lang::Fr;
    let (n, unit_fr, unit_en) = if days < 1 {
        if hours < 1 { (minutes.max(1), "minute", "minute") } else { (hours, "heure", "hour") }
    } else if days < 30 {
        if days == 1 {
            return if fr { "hier".into() } else { "yesterday".into() };
        }
        (days, "jour", "day")
    } else if days < 365 {
        if days / 30 == 1 {
            return if fr { "le mois dernier".into() } else { "last month".into() };
        }
        (days / 30, "mois", "month")
    } else {
        if days / 365 == 1 {
            return if fr { "l’année dernière".into() } else { "last year".into() };
        }
        (days / 365, "an", "year")
    };
    if fr {
        let plural = if n > 1 && unit_fr != "mois" { "s" } else { "" };
        format!("il y a {n} {unit_fr}{plural}")
    } else {
        let plural = if n > 1 { "s" } else { "" };
        format!("{n} {unit_en}{plural} ago")
    }
}

fn history_row(ui: &Ui, p: &Palette, i: usize, e: &Progress) -> El {
    let label = episode_tag(e.season, e.episode);
    let sub = [
        Some(kind_label(&e.kind)),
        label,
        Some(relative(e.updated_at, control_core::util::now_ms())).filter(|s| !s.is_empty()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let pct = if e.duration > 0.0 { ((e.position / e.duration) * 100.0).round().min(100.0) as i64 } else { 0 };
    let r = p.r(6.0);
    let mut art = el().size(44.0, 66.0).radius(r).clip_rounded(r).fill(p.fill_strong);
    if let Some(img) = ui.watch.images.get(ui, e.poster.as_deref()) {
        art = art.child(el().w_full().h_full().pixels_fit(img.pixels(), SurfaceFit::Fill));
    }
    let status = if e.finished {
        row()
            .gap(5.0)
            .items_center()
            .child(icon(&icons::CIRCLE_CHECK, 16.0).color(p.success))
            .child(text(t("hist.watched")).font(ty::SMALL_MEDIUM).color(p.success))
    } else {
        row().child(text(format!("{pct}%")).font(ty::NUMBERS).color(p.text2))
    };
    let (rk, rid) = (e.kind.clone(), e.id.clone());
    let remove = row()
        .key(format!("hist-rm-{i}"))
        .size(28.0, 28.0)
        .radius(14.0)
        .center()
        .cursor(Cursor::Pointer)
        .hover(p.fill_hover)
        .role("button")
        .label(t("common.remove"))
        .child(icon(&icons::X, 15.0).color(p.text2))
        .on_press(move |s: &mut Ui| {
            let (k, id) = (rk.clone(), rid.clone());
            s.spawn(move |b| b.media.watch_remove(&k, &id), |s, _, _| load_local(s));
        });
    let route = title_route(&e.kind, &e.id);
    row()
        .key(format!("hist-{i}"))
        .w_full()
        .gap(14.0)
        .items_center()
        .px(10.0)
        .py(8.0)
        .radius(p.r(12.0))
        .hover(p.fill)
        .cursor(Cursor::Pointer)
        .child(art)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(
                    text(if e.title.is_empty() { "—".into() } else { e.title.clone() })
                        .font(ty::BODY_MEDIUM)
                        .color(p.text)
                        .truncate(),
                )
                .child(text(sub).font(ty::SMALL).color(p.text2).truncate()),
        )
        .child(status)
        .child(remove)
        .on_press(move |s: &mut Ui| s.navigate(route.clone()))
}

pub fn view(ui: &Ui, p: &Palette, cx: &mut Cx<Ui>) -> El {
    let width = content_width(cx);
    let w = &ui.watch;
    let clear = (!w.history.is_empty()).then(|| {
        let label = if w.confirm_clear { format!("{} ?", t("hist.clear")) } else { t("hist.clear") };
        let mut b = button(p, "hist-clear", &label, Some(&icons::TRASH_2), false);
        if w.confirm_clear {
            b = b.fill(p.danger);
        }
        b.on_press(|s: &mut Ui| {
            // Deux clics : le premier demande confirmation (l'original ouvrait une feuille).
            if !s.watch.confirm_clear {
                s.watch.confirm_clear = true;
                s.toast(t("hist.clearTitle"), "trash-2");
                return;
            }
            s.watch.confirm_clear = false;
            s.spawn(
                |b| b.media.watch_clear(),
                |s, res, _| {
                    if res.is_ok() {
                        s.toast(t("hist.cleared"), "trash-2");
                    }
                    load_local(s);
                },
            );
        })
    });
    let mut parts = vec![page_head(p, &t("nav.mylist"), Some(t("mylist.subtitle")), clear)];

    if w.local_loaded && w.continuing.is_empty() && w.list.is_empty() && w.history.is_empty() {
        let movies = button(p, "ml-movies", &t("nav.movies"), Some(&icons::CLAPPERBOARD), true)
            .on_press(|s: &mut Ui| s.navigate(Route::Watch(WatchRoute::Movies)));
        let anime = button(p, "ml-anime", &t("nav.anime"), Some(&icons::TV_MINIMAL), false)
            .on_press(|s: &mut Ui| s.navigate(Route::Watch(WatchRoute::Anime)));
        parts.push(empty_state(
            p,
            &icons::BOOKMARK,
            &t("mylist.emptyTitle"),
            &t("mylist.emptyText"),
            vec![movies, anime],
        ));
        return page("watch-mylist".into(), parts);
    }
    if !w.continuing.is_empty() {
        let cards =
            w.continuing.iter().enumerate().map(|(i, e)| resume_card(ui, p, format!("ml-resume-{i}"), e)).collect();
        parts.push(shelf(ui, p, "ml-resume", &t("shelf.continue"), None, width, 8.0 + RESUME_W * 9.0 / 32.0, cards));
    }
    if !w.list.is_empty() {
        let items: Vec<Item> = w.list.iter().map(Item::from_saved).collect();
        parts.push(text(t("shelf.myList")).font(BLOCK_TITLE).color(p.text).mt(30.0).mb(14.0));
        parts.push(posters(ui, p, "ml-saved", &items, width, true));
    }
    if !w.history.is_empty() {
        parts.push(text(t("hist.title")).font(BLOCK_TITLE).color(p.text).mt(30.0).mb(10.0));
        let mut list = col().w_full();
        for (i, e) in w.history.iter().enumerate() {
            list = list.child(history_row(ui, p, i, e));
        }
        parts.push(list);
    }
    page("watch-mylist".into(), parts)
}

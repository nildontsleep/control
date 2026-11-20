//! Accueils « Films & séries » (TMDB) et « Animés » (AniList) : bannière, genres, reprise, ma liste, rangées
//! (port de `views/watch.js`).

use rune::{Cx, El, row};
use rune_kit::TextInput;

use control_core::media::Home;

use super::cards::{
    self, Item, RESUME_W, chips, error_state, hero, key_card, poster_shelf, resume_card, shelf, skeleton_shelf,
};
use super::{Load, Tab, WatchRoute, is_no_key, page};
use crate::i18n::t;
use crate::icons;
use crate::state::{Route, Ui};
use crate::theme::Palette;
use crate::ui::common::{kit, page_head};
use crate::views::segmented;

/// Largeur utile des pages (sans les marges de 32 points).
pub fn content_width(cx: &mut Cx<Ui>) -> f32 {
    cx.layout_size("view").map(|[w, _]| w).unwrap_or(1100.0) - 64.0
}

fn view_height(cx: &mut Cx<Ui>) -> f32 {
    cx.layout_size("view").map(|[_, h]| h).unwrap_or(800.0)
}

/// Champ de recherche dans les catalogues (Entrée ouvre les résultats).
pub fn search_field(ui: &Ui, p: &Palette) -> El {
    TextInput::new(&ui.watch.query)
        .placeholder(t("watch.searchPlaceholder"))
        .leading_icon(&icons::SEARCH)
        .on_edit(|s: &mut Ui| &mut s.watch.query)
        .on_submit(|s: &mut Ui, _| {
            let q = s.watch.query.text.trim().to_string();
            if q.chars().count() >= 2 {
                s.navigate(Route::Watch(WatchRoute::Search(q)));
            }
        })
        .build(&kit(p))
        .key("watch-search")
        .w(240.0)
}

pub fn view(ui: &Ui, p: &Palette, cx: &mut Cx<Ui>, anime: bool) -> El {
    let width = content_width(cx);
    let kind: &'static str = if anime { "anime" } else { ui.watch.tab.as_str() };
    let tab = if anime { None } else { Some(ui.watch.tab) };

    let mut actions = row().gap(10.0).items_center().child(search_field(ui, p));
    if let Some(tab) = tab {
        actions = actions.child(segmented(
            p,
            "watch-tabs",
            Tab::ALL.iter().map(|t| (t.as_str().to_string(), crate::i18n::t(&format!("tab.{}", t.as_str())))).collect(),
            tab.as_str(),
            |s: &mut Ui, v: String| {
                s.watch.tab = Tab::parse(&v);
                s.watch.hero = 0;
                let tab = s.watch.tab.as_str();
                super::load_home(s, tab);
            },
        ));
    }
    let (title, subtitle) =
        if anime { (t("nav.anime"), t("anime.subtitle")) } else { (t("nav.movies"), t("movies.subtitle")) };
    let mut parts = vec![page_head(p, &title, Some(subtitle), Some(actions))];

    match ui.watch.homes.get(kind).unwrap_or(&Load::Loading) {
        Load::Loading => {
            for i in 0..3 {
                parts.push(skeleton_shelf(p, &format!("sk-{i}"), 9));
            }
        }
        Load::Failed(err) if is_no_key(err) => parts.push(key_card(ui, p, &kit(p))),
        Load::Failed(err) => parts.push(error_state(p, "home-retry", err, move |s: &mut Ui| super::load_home(s, kind))),
        Load::Ready(home) => parts.extend(home_rows(ui, p, home, kind, tab, width, view_height(cx))),
    }
    page(format!("watch-home-{kind}"), parts)
}

fn home_rows(
    ui: &Ui,
    p: &Palette,
    home: &Home,
    kind: &'static str,
    tab: Option<Tab>,
    width: f32,
    height: f32,
) -> Vec<El> {
    let anime = tab.is_none();
    let accepts = |k: &str| match tab {
        None => k == "anime",
        Some(tab) => tab.accepts(k),
    };
    let show_kind = tab == Some(Tab::All);
    let mut parts = Vec::new();

    // Bannière : les titres avec un fond de la première rangée qui en a.
    let slides: Vec<Item> = home
        .shelves
        .iter()
        .map(|s| s.items.iter().map(Item::from_catalog).filter(|i| i.backdrop.is_some()).collect::<Vec<_>>())
        .find(|v| !v.is_empty())
        .unwrap_or_default();
    if !slides.is_empty() {
        parts.push(hero(ui, p, &slides, width, (height * 0.44).clamp(300.0, 440.0)));
    }

    if !home.genres.is_empty() {
        let items = home
            .genres
            .iter()
            .map(|g| {
                let id = match &g.id {
                    serde_json::Value::String(s) => s.clone(),
                    v => v.to_string(),
                };
                let label = if anime { cards::anime_genre_label(&g.name) } else { g.name.clone() };
                let source = if anime { format!("anime:genre:{id}") } else { format!("tmdb:discover/{kind}/{id}") };
                let title = label.clone();
                let press: Box<dyn Fn(&mut Ui)> = Box::new(move |s: &mut Ui| {
                    s.navigate(Route::Watch(WatchRoute::Shelf { source: source.clone(), title: title.clone() }))
                });
                (label, false, press)
            })
            .collect();
        parts.push(chips(p, "genres", items).mt(4.0).mb(14.0));
    }

    // Reprise et « Ma liste » (locales).
    let resume: Vec<El> = ui
        .watch
        .continuing
        .iter()
        .filter(|e| accepts(&e.kind))
        .enumerate()
        .map(|(i, e)| resume_card(ui, p, format!("resume-{i}"), e))
        .collect();
    if !resume.is_empty() {
        parts.push(shelf(ui, p, "resume", &t("shelf.continue"), None, width, 8.0 + RESUME_W * 9.0 / 32.0, resume));
    }
    let saved: Vec<Item> = ui.watch.list.iter().filter(|e| accepts(&e.kind)).take(24).map(Item::from_saved).collect();
    if !saved.is_empty() {
        parts.push(poster_shelf(
            ui,
            p,
            "saved",
            &t("shelf.myList"),
            Some(Route::Watch(WatchRoute::MyList)),
            width,
            &saved,
            show_kind,
        ));
    }

    for s in &home.shelves {
        let title = t(&format!("shelf.{}", s.id));
        let items: Vec<Item> = s.items.iter().map(Item::from_catalog).collect();
        let more = Route::Watch(WatchRoute::Shelf { source: s.source.clone(), title: title.clone() });
        parts.push(poster_shelf(ui, p, &format!("shelf-{}", s.id), &title, Some(more), width, &items, show_kind));
    }
    parts
}

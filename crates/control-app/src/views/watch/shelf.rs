//! « Tout voir » : une rangée en grille, chargée page par page quand on arrive au bout (port de `views/shelf.js`) ;
//! et la recherche dans les catalogues en ligne (la partie « search.online » de `views/search.js`).

use rune::{Cx, El, col, el, icon, row, text};

use super::cards::{Item, error_state, error_text, open_streaming_settings, poster_card, poster_sub};
use super::home::{content_width, search_field};
use super::{Load, load_shelf_more, page};
use crate::i18n::{t, tv};
use crate::icons;
use crate::state::Ui;
use crate::theme::Palette;
use crate::ui::common::{button, empty_state, page_head, section_title, ty};
use crate::ui::grid::{GAP, card_width, columns, grid};

/// Largeur minimale d'une affiche dans les grilles.
const MIN_POSTER: f32 = 150.0;

pub fn view(ui: &Ui, p: &Palette, cx: &mut Cx<Ui>, source: &str, title: &str) -> El {
    let sp = &ui.watch.shelf;
    if sp.source != source {
        return page("watch-shelf-wait".into(), vec![page_head(p, title, None, None)]);
    }
    let show_kind = source == "tmdb:trending";
    let n = sp.items.len();
    let more = sp.loading || sp.error.is_some() || sp.page < sp.total_pages || sp.page == 0;
    let count = n + usize::from(more);
    let head_title = if title.is_empty() { t("shelf.all") } else { title.to_string() };
    grid(
        cx,
        "watch-shelf",
        count,
        MIN_POSTER,
        MIN_POSTER * 0.5 + 70.0,
        move |s, p| {
            let sp = &s.watch.shelf;
            let mut head = col().w_full().child(page_head(p, &head_title, None, None));
            if sp.items.is_empty() && !sp.loading && sp.error.is_none() && sp.page > 0 {
                head = head.child(empty_state(p, &icons::SEARCH, &t("search.noneTitle"), "", vec![]));
            }
            head
        },
        move |s, p, i, w| {
            let sp = &s.watch.shelf;
            match sp.items.get(i) {
                Some(it) => {
                    let item = Item::from_catalog(it);
                    poster_card(s, p, format!("sh-{i}"), &item, w, poster_sub(&item, show_kind), None)
                }
                None => status_cell(p, w, sp.error.as_ref()),
            }
        },
        p,
    )
}

/// Case qui suit la dernière affiche : elle demande la page suivante dès qu'elle paraît, ou montre l'erreur.
fn status_cell(p: &Palette, w: f32, error: Option<&control_core::media::MediaError>) -> El {
    let h = w * 1.5;
    let base = col().key("sh-status").w(w).h(h).radius(p.r(12.0)).fill(p.fill).center().gap(10.0).padding(12.0);
    match error {
        Some(err) => base
            .child(icon(&icons::TRIANGLE_ALERT, 22.0).color(p.text3))
            .child(text(error_text(err)).font(ty::SMALL).color(p.text2).wrap().align(rune::Align::Center).w_full())
            .child(button(p, "sh-retry", &t("action.retry"), Some(&icons::REFRESH_CW), true).on_press(load_shelf_more)),
        None => base.child(icon(&icons::LOADER, 22.0).color(p.text3)).on_visible(|s: &mut Ui, on, _| {
            if on {
                load_shelf_more(s);
            }
        }),
    }
}

/// Grille simple (sans virtualisation) pour quelques dizaines d'affiches.
fn poster_grid(ui: &Ui, p: &Palette, key: &str, items: &[Item], width: f32, show_kind: bool) -> El {
    let cols = columns(width + 64.0, MIN_POSTER);
    let w = card_width(width + 64.0, cols);
    let mut out = col().key(key.to_string()).w_full().gap(26.0);
    for (r, chunk) in items.chunks(cols).enumerate() {
        let mut line = row().w_full().gap(GAP).items_start();
        for (c, it) in chunk.iter().enumerate() {
            line = line.child(poster_card(
                ui,
                p,
                format!("{key}-{}", r * cols + c),
                it,
                w,
                poster_sub(it, show_kind),
                None,
            ));
        }
        out = out.child(line);
    }
    out
}

/// Grille d'affiches pour d'autres pages (« Ma liste »).
pub fn posters(ui: &Ui, p: &Palette, key: &str, items: &[Item], width: f32, show_kind: bool) -> El {
    poster_grid(ui, p, key, items, width, show_kind)
}

/// Résultats de recherche dans TMDB et AniList.
pub fn search_view(ui: &Ui, p: &Palette, cx: &mut Cx<Ui>, query: &str) -> El {
    let width = content_width(cx);
    let mut parts = vec![page_head(p, &tv("search.resultsFor", &[("q", &query)]), None, Some(search_field(ui, p)))];
    match ui.watch.search.as_ref().filter(|(q, _)| q == query.trim()).map(|(_, l)| l) {
        None | Some(Load::Loading) => parts.push(
            row()
                .gap(10.0)
                .items_center()
                .py(12.0)
                .child(icon(&icons::LOADER, 16.0).color(p.text2))
                .child(text(t("search.online")).font(ty::BODY).color(p.text2)),
        ),
        Some(Load::Failed(err)) => {
            let q = query.to_string();
            parts.push(error_state(p, "search-retry", err, move |s: &mut Ui| super::load_search(s, &q)));
        }
        Some(Load::Ready(res)) => {
            if !res.tmdb.is_empty() {
                let items: Vec<Item> = res.tmdb.iter().map(Item::from_tmdb).collect();
                parts.push(section_title(p, &t("nav.movies")));
                parts.push(poster_grid(ui, p, "sr-tmdb", &items, width, true));
            }
            if !res.anime.is_empty() {
                let items: Vec<Item> = res.anime.iter().map(Item::from_anime).collect();
                parts.push(section_title(p, &t("nav.anime")).mt(24.0));
                parts.push(poster_grid(ui, p, "sr-anime", &items, width, false));
            }
            if res.tmdb_error.as_deref() == Some("nokey") {
                parts.push(
                    row()
                        .w_full()
                        .gap(10.0)
                        .items_center()
                        .justify_between()
                        .my(24.0)
                        .px(16.0)
                        .py(12.0)
                        .radius(p.r(14.0))
                        .fill(p.fill)
                        .child(text(t("search.nokey")).font(ty::BODY).color(p.text2).shrink().min_w(0.0).wrap())
                        .child(
                            button(p, "sr-addkey", &t("search.addKey"), Some(&icons::KEY_ROUND), false)
                                .on_press(open_streaming_settings),
                        ),
                );
            }
            if res.tmdb.is_empty() && res.anime.is_empty() {
                parts.push(empty_state(
                    p,
                    &icons::SEARCH,
                    &t("search.noneTitle"),
                    &tv("search.noneText", &[("q", &query)]),
                    vec![],
                ));
            }
        }
    }
    parts.push(el().h(8.0));
    page(format!("watch-search-{query}"), parts)
}

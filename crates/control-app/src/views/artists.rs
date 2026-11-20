//! Artistes (grille de portraits ronds) et fiche d'un artiste : ses albums, puis ses morceaux.

use std::rc::Rc;

use rune::{Cx, El, col, row, text};

use control_core::model::TrackId;
use control_core::util::natural_cmp;

use crate::format;
use crate::i18n::{t, tn};
use crate::state::{MenuTarget, OpenMenu, Route, Ui};
use crate::theme::Palette;
use crate::ui::common::{CoverKind, cover, page_head, section_title, ty};
use crate::ui::grid::{GAP, card, card_width, columns, grid};
use crate::ui::tracklist::{Layout, TrackList, track_list};
use crate::views::albums::album_card;
use crate::views::empty_library;
use crate::views::songs::play_buttons;

fn artist_tracks(ui: &Ui, key: &str) -> Vec<TrackId> {
    ui.cat
        .artist_by_key
        .get(key)
        .map(|&i| ui.cat.artists[i].tracks.iter().map(|&e| ui.cat.entries[e].track.id).collect())
        .unwrap_or_default()
}

pub fn view(ui: &Ui, p: &Palette, cx: &mut Cx<Ui>) -> El {
    if ui.ready && ui.cat.artists.is_empty() {
        return empty_library(ui, p);
    }
    let mut order: Vec<usize> = (0..ui.cat.artists.len()).collect();
    order.sort_by(|&a, &b| natural_cmp(&ui.cat.artists[a].name, &ui.cat.artists[b].name));
    let order = Rc::new(order);
    let n = order.len();
    grid(
        cx,
        "artists",
        n,
        160.0,
        52.0,
        |s, p| page_head(p, &t("nav.artists"), Some(tn("count.artists", s.cat.artists.len())), None),
        move |s, p, i, w| {
            let idx = order[i];
            let a = &s.cat.artists[idx];
            let (k1, k2, k3) = (a.key.clone(), a.key.clone(), a.key.clone());
            card(
                p,
                format!("ar-{idx}"),
                cover(s, a.cover.as_deref(), &a.name, w, w / 2.0, CoverKind::Artist),
                w,
                a.name.clone(),
                tn("artist.count", a.tracks.len()),
                true,
                true,
                move |s: &mut Ui| {
                    let ids = artist_tracks(s, &k1);
                    let (cat, b) = (s.cat.clone(), s.backend.clone());
                    s.player.play_list(&ids, 0, Some(true), &cat, &b);
                },
            )
            .on_press(move |s: &mut Ui| s.navigate(Route::Artist(k2.clone())))
            .on_secondary_press(move |s: &mut Ui, at, _| {
                s.menu = Some(OpenMenu { at, target: MenuTarget::Artist(k3.clone()), submenu: false });
            })
        },
        p,
    )
}

pub fn artist_view(ui: &Ui, p: &Palette, key: &str, cx: &mut Cx<Ui>) -> El {
    if !ui.cat.artist_by_key.contains_key(key) {
        return crate::views::not_found(p);
    }
    let ids: Rc<Vec<TrackId>> = Rc::new(artist_tracks(ui, key));
    let head_ids = ids.clone();
    let key_owned = key.to_string();
    let width = cx.layout_size("artist").map(|[w, _]| w).unwrap_or(1000.0);
    track_list(
        ui,
        p,
        TrackList {
            key: "artist",
            layout: Layout::Artist,
            ids,
            entries: None,
            playlist: None,
            sort: None,
            on_sort: None,
            track_numbers: false,
        },
        move |s, p| {
            let Some(&idx) = s.cat.artist_by_key.get(&key_owned) else { return col() };
            let a = &s.cat.artists[idx];
            let meta = [tn("count.albums", a.albums.len()), tn("count.tracks", a.tracks.len()), format::long(a.duration)].join(" · ");
            let hero = row()
                .w_full()
                .gap(28.0)
                .items_center()
                .pt(12.0)
                .pb(20.0)
                .child(cover(s, a.cover.as_deref(), &a.name, 200.0, 100.0, CoverKind::Artist).shadow(14.0, 40.0, rune::Color::BLACK.alpha(0.35)))
                .child(
                    col()
                        .gap(6.0)
                        .min_w(0.0)
                        .shrink()
                        .child(text(t("artist.kind")).font(ty::CAPTION).color(p.text3))
                        .child(text(a.name.clone()).font(ty::LARGE_TITLE).color(p.text).max_lines(2))
                        .child(text(meta).font(ty::SMALL).color(p.text2))
                        .child(play_buttons(p, head_ids.clone()).mt(10.0)),
                );
            let mut out = col().w_full().child(hero);
            if !a.albums.is_empty() {
                let cols = columns(width, 170.0);
                let w = card_width(width, cols);
                let mut grid_rows = col().w_full();
                for chunk in a.albums.chunks(cols) {
                    let mut r = row().w_full().gap(GAP).pb(GAP).items_start();
                    for &al in chunk {
                        r = r.child(album_card(s, p, al, w, false));
                    }
                    grid_rows = grid_rows.child(r);
                }
                out = out.child(section_title(p, &t("nav.albums"))).child(grid_rows);
            }
            out.child(section_title(p, &t("artist.songs")))
        },
    )
}

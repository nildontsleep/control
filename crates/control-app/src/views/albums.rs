//! Albums (grille, taille réglable) et fiche d'un album.

use std::rc::Rc;

use rune::{Cursor, Cx, El, col, row, text};
use serde_json::json;

use control_core::catalog::sort_albums;
use control_core::model::TrackId;

use crate::i18n::{t, tn};
use crate::state::{MenuTarget, OpenMenu, Route, Ui};
use crate::theme::Palette;
use crate::ui::common::{CoverKind, cover, page_head, ty};
use crate::ui::grid::{card, grid};
use crate::ui::tracklist::{Layout, TrackList, track_list};
use crate::views::songs::{play_buttons, stat_line};
use crate::views::{empty_library, segmented};

const SORTS: &[&str] = &["title", "artist", "year", "addedAt", "playCount"];

fn album_tracks(ui: &Ui, key: &str) -> Vec<TrackId> {
    ui.cat
        .album_by_key
        .get(key)
        .map(|&i| ui.cat.albums[i].tracks.iter().map(|&e| ui.cat.entries[e].track.id).collect())
        .unwrap_or_default()
}

/// Carte d'album (grille des albums, fiche d'artiste).
pub fn album_card(ui: &Ui, p: &Palette, index: usize, w: f32, show_artist: bool) -> El {
    let a = &ui.cat.albums[index];
    let key = a.key.clone();
    let sub = [
        if show_artist { if a.various { t("album.various") } else { a.artist.clone() } } else { String::new() },
        a.year.map(|y| y.to_string()).unwrap_or_default(),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join(" · ");
    let play_key = key.clone();
    let menu_key = key.clone();
    let nav_key = key.clone();
    card(
        p,
        format!("al-{}", index),
        cover(ui, a.cover.as_deref(), &a.title, w, p.r(10.0), CoverKind::Music),
        w,
        a.title.clone(),
        sub,
        false,
        false,
        move |s: &mut Ui| {
            let ids = album_tracks(s, &play_key);
            let (cat, b) = (s.cat.clone(), s.backend.clone());
            s.player.play_list(&ids, 0, Some(false), &cat, &b);
        },
    )
    .on_press(move |s: &mut Ui| s.navigate(Route::Album(nav_key.clone())))
    .on_secondary_press(move |s: &mut Ui, at, _| {
        s.menu = Some(OpenMenu { at, target: MenuTarget::Album(menu_key.clone()), submenu: false });
    })
}

pub fn view(ui: &Ui, p: &Palette, cx: &mut Cx<Ui>) -> El {
    if ui.ready && ui.cat.albums.is_empty() {
        return empty_library(ui, p);
    }
    let mut order: Vec<usize> = (0..ui.cat.albums.len()).collect();
    sort_albums(&ui.cat, &mut order, &ui.settings.sort.albums);
    let order = Rc::new(order);
    let size = ui.settings.album_size.clamp(140.0, 280.0) as f32;
    let n = order.len();
    let card_order = order.clone();
    grid(
        cx,
        "albums",
        n,
        size,
        52.0,
        move |s, p| {
            let current = s.settings.sort.albums.clone();
            let picker = segmented(
                p,
                "albums-sort",
                SORTS.iter().map(|k| (k.to_string(), t(&format!("sort.{k}")))).collect(),
                &current,
                |s: &mut Ui, v: String| s.set(json!({ "sort": { "albums": v } })),
            );
            let sizes = row()
                .gap(4.0)
                .child(size_button(p, "albums-smaller", "−", -30.0))
                .child(size_button(p, "albums-larger", "+", 30.0));
            page_head(p, &t("nav.albums"), Some(tn("count.albums", s.cat.albums.len())), Some(row().gap(10.0).items_center().child(picker).child(sizes)))
        },
        move |s, p, i, w| album_card(s, p, card_order[i], w, true),
        p,
    )
}

fn size_button(p: &Palette, key: &str, label: &str, delta: f64) -> El {
    row()
        .key(key.to_string())
        .size(28.0, 28.0)
        .radius(p.r(7.0))
        .center()
        .fill(p.fill)
        .hover(p.fill_hover)
        .cursor(Cursor::Pointer)
        .role("button")
        .label(t("albums.size"))
        .child(text(label.to_string()).font(ty::BODY_MEDIUM).color(p.text))
        .on_press(move |s: &mut Ui| {
            let v = (s.settings.album_size + delta).clamp(140.0, 280.0);
            s.set(json!({ "albumSize": v }));
        })
}

/// Fiche d'un album : grande pochette, titre, artiste, année, puis les morceaux dans l'ordre du disque.
pub fn album_view(ui: &Ui, p: &Palette, key: &str) -> El {
    let Some(&idx) = ui.cat.album_by_key.get(key) else {
        return crate::views::not_found(p);
    };
    let ids: Rc<Vec<TrackId>> = Rc::new(album_tracks(ui, key));
    let head_ids = ids.clone();
    let key_owned = key.to_string();
    let discs = ui.cat.albums[idx]
        .tracks
        .iter()
        .filter_map(|&e| ui.cat.entries[e].track.disc_no)
        .max()
        .unwrap_or(1);
    track_list(
        ui,
        p,
        TrackList {
            key: "album",
            layout: Layout::Album,
            ids,
            entries: None,
            playlist: None,
            sort: None,
            on_sort: None,
            track_numbers: true,
        },
        move |s, p| {
            let Some(&idx) = s.cat.album_by_key.get(&key_owned) else { return col() };
            let a = &s.cat.albums[idx];
            let genre = a.tracks.first().map(|&e| s.cat.entries[e].track.genre.clone()).unwrap_or_default();
            let meta = [
                genre,
                a.year.map(|y| y.to_string()).unwrap_or_default(),
                if discs > 1 { tn("album.discs", discs) } else { String::new() },
                stat_line(s, &head_ids),
            ]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
            let artist_label = if a.various { t("album.various") } else if a.artist.is_empty() { t("album.unknown") } else { a.artist.clone() };
            let mut artist = text(artist_label).font(ty::TITLE).color(p.accent);
            if let Some(ak) = a.artist_key.clone() {
                artist = artist.cursor(Cursor::Pointer).on_press(move |s: &mut Ui| s.navigate(Route::Artist(ak.clone())));
            }
            row()
                .w_full()
                .gap(28.0)
                .items_end()
                .pt(12.0)
                .pb(24.0)
                .child(cover(s, a.cover.as_deref(), &a.title, 220.0, p.r(14.0), CoverKind::Music).shadow(14.0, 40.0, rune::Color::BLACK.alpha(0.35)))
                .child(
                    col()
                        .gap(6.0)
                        .min_w(0.0)
                        .shrink()
                        .child(text(t("album.kind")).font(ty::CAPTION).color(p.text3))
                        .child(text(a.title.clone()).font(ty::LARGE_TITLE).color(p.text).max_lines(2))
                        .child(artist)
                        .child(text(meta).font(ty::SMALL).color(p.text2))
                        .child(play_buttons(p, head_ids.clone()).mt(10.0)),
                )
        },
    )
}


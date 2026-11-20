//! Fiche d'une playlist (classique ou intelligente).

use std::rc::Rc;

use rune::{Cursor, El, col, icon, row, text};

use control_core::model::{PlaylistId, PlaylistKind, TrackId};
use control_core::util::now_ms;

use crate::format;
use crate::i18n::{t, tn};
use crate::icons;
use crate::state::{MenuTarget, OpenMenu, Ui};
use crate::theme::Palette;
use crate::ui::common::{CoverKind, cover, ty};
use crate::ui::tracklist::{Layout, TrackList, track_list};
use crate::views::songs::play_buttons;

/// Pochette d'une playlist : celle choisie, sinon une mosaïque des quatre premières pochettes différentes.
pub fn playlist_cover(ui: &Ui, p: &Palette, id: PlaylistId, size: f32) -> El {
    let Some(pl) = ui.cat.playlist(id) else { return rune::el().size(size, size) };
    if pl.cover.is_some() {
        return cover(ui, pl.cover.as_deref(), &pl.name, size, p.r(14.0), CoverKind::Playlist);
    }
    let rows = ui.cat.playlist_rows(pl, now_ms());
    let mut hashes: Vec<String> = Vec::new();
    for r in &rows {
        if let Some(c) = &ui.cat.entries[r.entry].track.cover
            && !hashes.contains(c)
        {
            hashes.push(c.clone());
            if hashes.len() == 4 {
                break;
            }
        }
    }
    if hashes.len() < 4 {
        return cover(ui, hashes.first().map(String::as_str), &pl.name, size, p.r(14.0), CoverKind::Playlist);
    }
    let half = size / 2.0;
    let tile = |h: &String| cover(ui, Some(h), "", half, 0.0, CoverKind::Music);
    col()
        .size(size, size)
        .radius(p.r(14.0))
        .clip_rounded(p.r(14.0))
        .child(row().child(tile(&hashes[0])).child(tile(&hashes[1])))
        .child(row().child(tile(&hashes[2])).child(tile(&hashes[3])))
}

pub fn view(ui: &Ui, p: &Palette, id: PlaylistId) -> El {
    let Some(pl) = ui.cat.playlist(id) else { return crate::views::not_found(p) };
    let rows = ui.cat.playlist_rows(pl, now_ms());
    let ids: Rc<Vec<TrackId>> = Rc::new(rows.iter().map(|r| ui.cat.entries[r.entry].track.id).collect());
    let entries = (pl.kind == PlaylistKind::Manual).then(|| Rc::new(rows.iter().filter_map(|r| r.item_id).collect::<Vec<_>>()));
    let head_ids = ids.clone();
    track_list(
        ui,
        p,
        TrackList { key: "playlist", layout: Layout::Playlist, ids, entries, playlist: Some(id), sort: None, on_sort: None, track_numbers: false },
        move |s, p| {
            let Some(pl) = s.cat.playlist(id) else { return col() };
            let smart = pl.kind == PlaylistKind::Smart;
            let secs: f64 = head_ids.iter().filter_map(|i| s.cat.track(*i)).map(|t| t.duration).sum();
            let meta = if head_ids.is_empty() { t("pl.empty") } else { format!("{} · {}", tn("count.tracks", head_ids.len()), format::long(secs)) };
            let kind = if smart {
                row().gap(4.0).items_center().child(icon(&icons::SPARKLES, 12.0).color(p.accent)).child(text(t("pl.smart")).font(ty::CAPTION).color(p.accent))
            } else {
                row().child(text(t("pl.kind")).font(ty::CAPTION).color(p.text3))
            };
            let more = row()
                .key("pl-more")
                .size(30.0, 30.0)
                .radius(p.r(8.0))
                .center()
                .fill(p.fill_strong)
                .cursor(Cursor::Pointer)
                .role("button")
                .label(t("common.more"))
                .child(icon(&icons::ELLIPSIS, 18.0).color(p.text))
                .on_secondary_press(move |s: &mut Ui, at, _| s.menu = Some(OpenMenu { at, target: MenuTarget::Playlist(id), submenu: false }))
                .on_press(move |s: &mut Ui| {
                    // Le menu s'ouvre sous le bouton (sa place est relevée à chaque passage de la vue).
                    let [x, y, _, h] = s.more_rect.get();
                    s.menu = Some(OpenMenu { at: rune::PointerPress { x, y: y + h + 4.0 }, target: MenuTarget::Playlist(id), submenu: false });
                });
            let mut text_col = col()
                .gap(6.0)
                .min_w(0.0)
                .shrink()
                .child(kind)
                .child(text(pl.name.clone()).font(ty::LARGE_TITLE).color(p.text).max_lines(2));
            if !pl.description.is_empty() {
                text_col = text_col.child(text(pl.description.clone()).font(ty::BODY).color(p.text2).max_lines(3));
            }
            text_col = text_col
                .child(text(meta).font(ty::SMALL).color(p.text2))
                .child(row().gap(8.0).mt(10.0).items_center().child(play_buttons(p, head_ids.clone())).child(more));
            let mut out = col().w_full().child(
                row()
                    .w_full()
                    .gap(28.0)
                    .items_end()
                    .pt(12.0)
                    .pb(24.0)
                    .child(playlist_cover(s, p, id, 220.0).shadow(14.0, 40.0, rune::Color::BLACK.alpha(0.35)))
                    .child(text_col),
            );
            if head_ids.is_empty() {
                out = out.child(
                    col()
                        .w_full()
                        .py(40.0)
                        .gap(8.0)
                        .items_center()
                        .child(icon(&icons::LIST_MUSIC, 36.0).color(p.text3))
                        .child(text(t("pl.emptyTitle")).font(ty::HEADLINE).color(p.text))
                        .child(text(if smart { t("pl.emptySmart") } else { t("pl.emptyText") }).font(ty::BODY).color(p.text2)),
                );
            }
            out
        },
    )
}

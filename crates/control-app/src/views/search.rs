//! Résultats de recherche : morceaux, albums, artistes, vidéos et playlists.

use std::rc::Rc;

use rune::{Cursor, Cx, El, col, row, text};

use control_core::model::TrackId;

use crate::i18n::{t, tv};
use crate::state::{Route, Ui};
use crate::theme::Palette;
use crate::ui::common::{CoverKind, cover, page_head, section_title, ty};
use crate::ui::grid::{GAP, card_width, columns};
use crate::ui::tracklist::{Layout, TrackList, track_list};
use crate::views::albums::album_card;

pub fn view(ui: &Ui, p: &Palette, cx: &mut Cx<Ui>) -> El {
    let res = ui.results.clone().unwrap_or_default();
    let ids: Rc<Vec<TrackId>> = Rc::new(res.tracks.iter().map(|&i| ui.cat.entries[i].track.id).collect());
    let width = cx.layout_size("search").map(|[w, _]| w).unwrap_or(1000.0);
    let query = ui.search.text.trim().to_string();
    track_list(
        ui,
        p,
        TrackList { key: "search", layout: Layout::Search, ids, entries: None, playlist: None, sort: None, on_sort: None, track_numbers: false },
        move |s, p| {
            let res = s.results.clone().unwrap_or_default();
            let mut out = col().w_full().child(page_head(p, &tv("search.resultsFor", &[("q", &query)]), None, None));
            if res.is_empty() {
                return out.child(text(t("search.noneTitle")).font(ty::BODY).color(p.text2).py(30.0));
            }
            let cols = columns(width, 150.0);
            let w = card_width(width, cols);
            if !res.artists.is_empty() {
                let mut r = row().w_full().gap(GAP).pb(GAP);
                for &a in res.artists.iter().take(cols) {
                    let ar = &s.cat.artists[a];
                    let key = ar.key.clone();
                    r = r.child(
                        col()
                            .key(format!("sr-ar-{a}"))
                            .w(w)
                            .gap(6.0)
                            .items_center()
                            .cursor(Cursor::Pointer)
                            .child(cover(s, ar.cover.as_deref(), &ar.name, w, w / 2.0, CoverKind::Artist))
                            .child(text(ar.name.clone()).font(ty::BODY_MEDIUM).color(p.text).truncate())
                            .on_press(move |s: &mut Ui| s.navigate(Route::Artist(key.clone()))),
                    );
                }
                out = out.child(section_title(p, &t("nav.artists"))).child(r);
            }
            if !res.albums.is_empty() {
                let mut r = row().w_full().gap(GAP).pb(GAP);
                for &a in res.albums.iter().take(cols) {
                    r = r.child(album_card(s, p, a, w, true));
                }
                out = out.child(section_title(p, &t("nav.albums"))).child(r);
            }
            if !res.playlists.is_empty() {
                let mut r = col().w_full().gap(2.0).pb(GAP);
                for &i in &res.playlists {
                    let pl = &s.cat.playlists[i];
                    let id = pl.id;
                    r = r.child(
                        row()
                            .key(format!("sr-pl-{id}"))
                            .h(44.0)
                            .px(10.0)
                            .gap(12.0)
                            .items_center()
                            .radius(p.r(8.0))
                            .hover(p.fill)
                            .cursor(Cursor::Pointer)
                            .child(crate::views::playlist::playlist_cover(s, p, id, 32.0))
                            .child(text(pl.name.clone()).font(ty::BODY_MEDIUM).color(p.text))
                            .on_press(move |s: &mut Ui| s.navigate(Route::Playlist(id))),
                    );
                }
                out = out.child(section_title(p, &t("nav.playlists"))).child(r);
            }
            if !res.videos.is_empty() {
                let mut r = row().w_full().gap(GAP).pb(GAP);
                let vw = card_width(width, columns(width, 220.0));
                for &v in res.videos.iter().take(columns(width, 220.0)) {
                    let id = s.cat.entries[v].track.id;
                    r = r.child(
                        col()
                            .key(format!("sr-v-{id}"))
                            .w(vw)
                            .gap(6.0)
                            .cursor(Cursor::Pointer)
                            .child(crate::views::videos::thumb(s, p, id, vw, vw * 9.0 / 16.0))
                            .child(text(s.cat.entries[v].track.title.clone()).font(ty::BODY_MEDIUM).color(p.text).truncate())
                            .on_press(move |s: &mut Ui| {
                                let (cat, b) = (s.cat.clone(), s.backend.clone());
                                s.player.play_list(&[id], 0, Some(false), &cat, &b);
                                s.now_playing = true;
                            }),
                    );
                }
                out = out.child(section_title(p, &t("nav.videos"))).child(r);
            }
            if !res.tracks.is_empty() {
                out = out.child(section_title(p, &t("nav.songs")));
            }
            out
        },
    )
}

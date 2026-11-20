//! Morceaux : toute la musique, triable par colonne.

use std::rc::Rc;

use rune::{El, row};
use serde_json::json;

use control_core::catalog::sort_tracks;
use control_core::model::TrackId;

use crate::format;
use crate::i18n::{t, tn};
use crate::icons;
use crate::state::Ui;
use crate::theme::Palette;
use crate::ui::common::{button, page_head};
use crate::ui::tracklist::{Layout, TrackList, track_list};
use crate::views::empty_library;

/// « 18 morceaux · 7 min ».
pub fn stat_line(ui: &Ui, ids: &[TrackId]) -> String {
    let secs: f64 = ids.iter().filter_map(|id| ui.cat.track(*id)).map(|t| t.duration).sum();
    format!("{} · {}", tn("count.tracks", ids.len()), format::long(secs))
}

/// Boutons « Lecture » et « Aléatoire » d'une liste.
pub fn play_buttons(p: &Palette, ids: Rc<Vec<TrackId>>) -> El {
    let a = ids.clone();
    row()
        .gap(8.0)
        .child(button(p, "play-all", &t("common.play"), Some(&icons::PLAY), true).on_press(move |s: &mut Ui| {
            let (cat, b) = (s.cat.clone(), s.backend.clone());
            s.player.play_list(&a, 0, Some(false), &cat, &b);
        }))
        .child(button(p, "shuffle-all", &t("common.shuffle"), Some(&icons::SHUFFLE), false).on_press(move |s: &mut Ui| {
            let (cat, b) = (s.cat.clone(), s.backend.clone());
            let start = if ids.is_empty() { 0 } else { rand_index(ids.len()) };
            s.player.play_list(&ids, start, Some(true), &cat, &b);
            s.set(json!({ "shuffle": true }));
        }))
}

fn rand_index(n: usize) -> usize {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0) as usize;
    nanos % n
}

fn on_sort(s: &mut Ui, key: &'static str) {
    let cur = &s.settings.sort.songs;
    let dir = if cur.key == key && cur.dir == "asc" { "desc" } else { "asc" };
    s.set(json!({ "sort": { "songs": { "key": key, "dir": dir } } }));
}

pub fn view(ui: &Ui, p: &Palette) -> El {
    if ui.ready && ui.cat.audio.is_empty() {
        return empty_library(ui, p);
    }
    let sort = &ui.settings.sort.songs;
    let mut list = ui.cat.audio.clone();
    sort_tracks(&ui.cat, &mut list, &sort.key, sort.dir == "desc");
    let ids: Rc<Vec<TrackId>> = Rc::new(list.iter().map(|&i| ui.cat.entries[i].track.id).collect());
    let head_ids = ids.clone();
    track_list(
        ui,
        p,
        TrackList {
            key: "songs",
            layout: Layout::Songs,
            ids: ids.clone(),
            entries: None,
            playlist: None,
            sort: Some((sort.key.clone(), sort.dir == "desc")),
            on_sort: Some(on_sort),
            track_numbers: false,
        },
        move |s, p| page_head(p, &t("nav.songs"), Some(stat_line(s, &head_ids)), Some(play_buttons(p, head_ids.clone()))),
    )
}

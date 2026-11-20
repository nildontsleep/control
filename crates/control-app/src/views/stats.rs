//! Statistiques : temps d'écoute, activité, tops, genres, films et séries ; accès au récap
//! (`src/renderer/js/views/stats.js`).

mod blocks;
pub mod charts;
pub mod dates;

use std::collections::HashMap;
use std::sync::Arc;

use rune::{Cx, El, ScrollSpec, SurfaceFit, col, el};
use rune_kit::MenuItem;
use serde_json::json;
use time::Date;

use control_core::stats::Stats;

use crate::format::count;
use crate::i18n::{t, tv};
use crate::icons;
use crate::state::{Route, Ui};
use crate::theme::Palette;
use crate::ui::common::{CoverKind, button, cover, empty_state, page_head};
use crate::views::recap::{self, Kind, Lists, Period};
use crate::views::segmented;
use blocks::{Rank, block_title, card, genre_dot, kpi, note, poster, rank_list, tiles};
use charts::{Bar, Tip, bar_chart, heatmap};
use dates::{
    add_days, day_label, day_str, fmt_duration, parse_day, plays, short_day, tcount, today, weekday_name, weekday_short,
};

/// Styles de texte propres aux statistiques (tailles de `stats.css`).
pub mod style {
    use rune::TextStyle;

    pub const KPI: TextStyle = TextStyle::new(26.0, 720.0).features("'tnum' 1");
    pub const KPI_LABEL: TextStyle = TextStyle::new(12.5, 600.0);
    pub const SUB: TextStyle = TextStyle::new(11.5, 400.0);
    pub const CARD_TITLE: TextStyle = TextStyle::new(13.5, 650.0);
    pub const TINY: TextStyle = TextStyle::new(10.5, 400.0).features("'tnum' 1");
    pub const RANK_N: TextStyle = TextStyle::new(12.0, 650.0).features("'tnum' 1");
    pub const POSTER_TITLE: TextStyle = TextStyle::new(12.5, 600.0);
    pub const POSTER_RANK: TextStyle = TextStyle::new(11.0, 700.0);
}

/// Taille des tops demandés (comme l'original).
pub const TOP: usize = 10;
const SIDE: f32 = 32.0;

/// Une période de la page : premier et dernier jour (vides pour « tout »), nombre de jours pour un histogramme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Range {
    pub from: String,
    pub to: String,
    pub days: Option<u32>,
}

/// Les quatre choix du sélecteur (réglage `statsRange`).
pub fn range_for(kind: &str, today: Date) -> Range {
    match kind {
        "7d" => Range { from: day_str(add_days(today, -6)), to: day_str(today), days: Some(7) },
        "30d" => Range { from: day_str(add_days(today, -29)), to: day_str(today), days: Some(30) },
        "year" => Range { from: format!("{:04}-01-01", today.year()), to: day_str(today), days: None },
        _ => Range { from: String::new(), to: String::new(), days: None },
    }
}

/// État de la vue des statistiques.
#[derive(Default)]
pub struct StatsState {
    /// Dernières statistiques reçues, avec la période demandée.
    pub loaded: Option<(Range, Arc<Stats>)>,
    pub loading: bool,
    pub error: Option<String>,
    seq: u64,
    /// Menu « Récap » ouvert.
    pub menu_open: bool,
    pub tip: Option<Tip>,
}

/// Charge les statistiques d'une période sur un thread (`ui.spawn`), et rapporte le résultat.
pub fn fetch(
    ui: &Ui,
    from: String,
    to: String,
    apply: impl FnOnce(&mut Ui, Result<Stats, String>, &mut rune::Tx) + Send + 'static,
) {
    ui.spawn(
        move |b| {
            let db = b.db.lock();
            control_core::stats::get(&db, &from, &to, TOP, control_core::util::now_ms()).map_err(|e| e.to_string())
        },
        apply,
    );
}

/// La vue s'ouvre (ou la période change) : charger les statistiques.
pub fn on_open(ui: &mut Ui) {
    let range = range_for(&ui.settings.stats_range, today());
    ui.stats.seq += 1;
    ui.stats.loading = true;
    ui.stats.menu_open = false;
    ui.stats.tip = None;
    let seq = ui.stats.seq;
    let (from, to) = (range.from.clone(), range.to.clone());
    fetch(ui, from, to, move |s, res, _| {
        if s.stats.seq != seq {
            return;
        }
        s.stats.loading = false;
        match res {
            Ok(data) => {
                s.stats.error = None;
                s.stats.loaded = Some((range, Arc::new(data)));
            }
            Err(err) => {
                log::warn!("Statistiques : {err}");
                s.stats.error = Some(err);
            }
        }
    });
}

fn set_range(s: &mut Ui, v: String) {
    s.set(json!({ "statsRange": v }));
    on_open(s);
}

/// Ouvre le récap d'une période, avec les périodes connues de la page.
fn start_recap(s: &mut Ui, period: Period) {
    s.stats.menu_open = false;
    let lists = s.stats.loaded.as_ref().map(|(_, d)| Lists::of(d)).unwrap_or_default();
    recap::open(s, period, Some(lists));
}

/// Menu du bouton « Récap » : un récap par année, puis par mois, par semaine et par jour d'activité (les plus récents).
fn recap_menu(ui: &Ui, p: &Palette, trigger: El, layer: &mut Vec<El>) -> El {
    let k = crate::ui::common::kit(p);
    let lists = ui.stats.loaded.as_ref().map(|(_, d)| Lists::of(d)).unwrap_or_default();
    let mut menu = rune_kit::Menu::new("st-recap")
        .trigger(trigger)
        .open(ui.stats.menu_open)
        .placement(rune_kit::Placement::BottomEnd)
        .min_width(270.0)
        .on_close(|s: &mut Ui| s.stats.menu_open = false);
    for y in lists.years.iter().take(3) {
        if let Some(period) = Period::parse(Kind::Year, y) {
            menu = menu.item(
                MenuItem::new(tv("stats.wrapped", &[("year", y)]))
                    .icon(&icons::SPARKLES)
                    .on_select(move |s: &mut Ui| start_recap(s, period)),
            );
        }
    }
    let now = today();
    let sections: [(&str, Kind, usize, &'static rune::Icon); 3] = [
        ("stats.byMonth", Kind::Month, 6, &icons::CALENDAR),
        ("stats.byWeek", Kind::Week, 5, &icons::CALENDAR_DAYS),
        ("stats.byDay", Kind::Day, 6, &icons::CLOCK),
    ];
    for (header, kind, max, glyph) in sections {
        let list = lists.get(kind);
        if list.is_empty() {
            continue;
        }
        menu = menu.separator().group_label(t(header));
        for key in list.iter().take(max) {
            if let Some(period) = Period::parse(kind, key) {
                menu = menu.item(
                    MenuItem::new(period.menu_label(now))
                        .icon(glyph)
                        .on_select(move |s: &mut Ui| start_recap(s, period)),
                );
            }
        }
    }
    menu.build(&k, layer)
}

pub fn view(ui: &Ui, p: &Palette, cx: &mut Cx<Ui>) -> El {
    let width = cx.layout_size("stats").map(|[w, _]| w).unwrap_or(1000.0);
    let mut layer: Vec<El> = Vec::new();

    let options = vec![
        ("7d".to_string(), t("stats.p7")),
        ("30d".to_string(), t("stats.p30")),
        ("year".to_string(), t("stats.pYear")),
        ("all".to_string(), t("stats.pAll")),
    ];
    let kind = ui.settings.stats_range.clone();
    let mut actions = rune::row().gap(10.0).items_center().child(segmented(p, "st-range", options, &kind, set_range));
    let has_years = ui.stats.loaded.as_ref().is_some_and(|(_, d)| !d.years.is_empty());
    if has_years {
        let trigger = button(p, "st-recap-btn", &t("stats.wrappedShort"), Some(&icons::SPARKLES), true)
            .on_press(|s: &mut Ui| s.stats.menu_open = !s.stats.menu_open);
        actions = actions.child(recap_menu(ui, p, trigger, &mut layer));
    }

    let mut page = col().w_full().px(SIDE).pb(120.0).child(page_head(
        p,
        &t("nav.stats"),
        Some(t("stats.subtitle")),
        Some(actions),
    ));
    page =
        page.child(body(ui, p, width).opacity(if ui.stats.loading && ui.stats.loaded.is_some() { 0.55 } else { 1.0 }));

    let scroll = col()
        .key("stats")
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: false })
        .scroll_edge_fade(8.0)
        // Ouverte au démarrage (dernière vue), la page n'est pas passée par `on_open` : elle se charge ici.
        .on_layout(|s: &mut Ui, _| {
            if s.stats.loaded.is_none() && !s.stats.loading && s.stats.error.is_none() {
                on_open(s);
            }
        })
        .child(page);
    let mut root = el().w_full().h_full().child(scroll).children(layer);
    if let Some(tip) = &ui.stats.tip {
        root = root.child(charts::tip(p, tip));
    }
    root
}

/// Contenu sous l'en-tête.
fn body(ui: &Ui, p: &Palette, width: f32) -> El {
    if let Some(err) = &ui.stats.error {
        return empty_state(p, &icons::TRIANGLE_ALERT, &t("err.title"), err, vec![]);
    }
    let Some((range, data)) = &ui.stats.loaded else {
        return col().w_full().min_h(320.0);
    };
    let (music, video) = (&data.music, &data.video);
    // Sans aucun historique, un écran d'accueil plutôt que des zéros.
    if music.plays == 0 && video.seconds <= 0.0 && data.years.is_empty() {
        return empty_state(p, &icons::CHART_COLUMN, &t("stats.emptyTitle"), &t("stats.emptyText"), vec![]);
    }
    // Les colonnes suivent la largeur de la fenêtre, comme les `@media` de l'original.
    let window = width + crate::ui::sidebar::WIDTH;
    let wide = window > 1080.0;
    let mut sections: Vec<El> = Vec::new();

    // Musique — chiffres clés
    let completed = (music.completed > 0).then(|| tcount("stats.completed", music.completed));
    let streak_sub = tv("stats.bestStreak", &[("n", &tcount("stats.days", data.streak.longest as i64))]);
    sections.push(col().w_full().gap(12.0).child(block_title(p, t("stats.music"))).child(tiles(
        vec![
            kpi(p, t("stats.listened"), fmt_duration(music.seconds), None, &icons::HEADPHONES),
            kpi(p, t("stats.playsLabel"), count(music.plays), completed, &icons::PLAY),
            kpi(
                p,
                t("stats.tracksLabel"),
                count(music.tracks),
                Some(tcount("stats.albumsOf", music.albums)),
                &icons::MUSIC,
            ),
            kpi(p, t("stats.artistsLabel"), count(music.artists), None, &icons::USERS),
            kpi(
                p,
                t("stats.streak"),
                tcount("stats.days", data.streak.current as i64),
                Some(streak_sub),
                &icons::FLAME,
            ),
        ],
        if wide { 5 } else { 3 },
    )));

    // Activité : histogramme des jours sur 7 ou 30 jours, calendrier sinon.
    let by_day: HashMap<String, f64> = music.by_day.iter().map(|d| (d.day.clone(), d.seconds)).collect();
    let activity = match (range.days, parse_day(&range.from)) {
        (Some(days), Some(from)) => {
            let items: Vec<Bar> = (0..days)
                .map(|i| {
                    let d = add_days(from, i as i64);
                    let v = by_day.get(&day_str(d)).copied().unwrap_or(0.0);
                    Bar { label: short_day(d, days), value: v, title: day_label(d), sub: nothing_or(v) }
                })
                .collect();
            bar_chart(ui, p, "st-days", &items, if days > 14 { 5 } else { 1 }, 150.0, true)
        }
        _ => heatmap(p, &by_day, today(), 53),
    };
    sections.push(card(p, Some(t("stats.activity")), vec![activity]));

    // Heures et jours de la semaine
    let hours: Vec<Bar> = music
        .by_hour
        .iter()
        .enumerate()
        .map(|(i, &v)| Bar {
            label: if i % 6 == 0 { format!("{i} h") } else { String::new() },
            value: v,
            title: format!("{i:02}:00 – {i:02}:59"),
            sub: nothing_or(v),
        })
        .collect();
    let weekdays: Vec<Bar> = music
        .by_weekday
        .iter()
        .enumerate()
        .map(|(i, &v)| Bar {
            label: weekday_short(i).to_string(),
            value: v,
            title: weekday_name(i).to_string(),
            sub: nothing_or(v),
        })
        .collect();
    let mut hour_card = vec![bar_chart(ui, p, "st-hours", &hours, 1, 120.0, false)];
    let mut day_card = vec![bar_chart(ui, p, "st-wd", &weekdays, 1, 120.0, false)];
    if music.seconds > 0.0 {
        hour_card.push(note(p, tv("stats.peakHour", &[("h", &peak(&music.by_hour))])));
        day_card.push(note(p, tv("stats.peakDay", &[("d", &weekday_name(peak(&music.by_weekday)))])));
    }
    sections.push(blocks::grid(
        vec![card(p, Some(t("stats.byHour")), hour_card), card(p, Some(t("stats.byWeekday")), day_card)],
        if window > 820.0 { 2 } else { 1 },
        12.0,
    ));

    if music.plays > 0 {
        sections.push(blocks::grid(tops(ui, p, data), if wide { 3 } else { 1 }, 12.0));
        if !music.genres.is_empty() {
            let top = music.genres.iter().map(|g| g.plays).max().unwrap_or(1).max(1) as f32;
            let rows = music
                .genres
                .iter()
                .enumerate()
                .map(|(i, g)| Rank {
                    key: format!("st-genre-{i}"),
                    art: genre_dot(p, i, 0.1),
                    round: true,
                    title: g.name.clone(),
                    sub: fmt_duration(g.seconds),
                    value: plays(g.plays),
                    share: g.plays as f32 / top,
                    on_press: None,
                })
                .collect();
            sections.push(card(p, Some(t("stats.genres")), vec![rank_list(p, rows)]));
        }
    }

    if video.seconds > 0.0 || !video.top.is_empty() {
        sections.push(video_block(ui, p, data, width));
    }
    col().w_full().gap(18.0).children(sections)
}

fn nothing_or(v: f64) -> String {
    if v > 0.0 { fmt_duration(v) } else { t("stats.nothing") }
}

/// Indice de la plus grande valeur (la première en cas d'égalité).
pub fn peak(values: &[f64]) -> usize {
    let max = values.iter().copied().fold(f64::MIN, f64::max);
    values.iter().position(|v| *v == max).unwrap_or(0)
}

/// Les trois classements : morceaux, artistes, albums.
fn tops(ui: &Ui, p: &Palette, data: &Arc<Stats>) -> Vec<El> {
    let music = &data.music;
    let share = |v: i64, top: i64| v as f32 / top.max(1) as f32;

    let top_plays = music.top_tracks.iter().map(|x| x.plays).max().unwrap_or(1);
    let ids: Arc<Vec<i64>> = Arc::new(music.top_tracks.iter().map(|x| x.id).collect());
    let tracks = music
        .top_tracks
        .iter()
        .enumerate()
        .map(|(i, x)| {
            let ids = ids.clone();
            let target = x.id;
            Rank {
                key: format!("st-track-{i}"),
                art: cover(
                    ui,
                    x.cover.as_deref(),
                    &format!("{}{}", x.title, x.album),
                    40.0,
                    p.r(8.0),
                    CoverKind::Music,
                ),
                round: false,
                title: x.title.clone(),
                sub: if x.artist.is_empty() { "—".into() } else { x.artist.clone() },
                value: plays(x.plays),
                share: share(x.plays, top_plays),
                on_press: Some(Box::new(move |s: &mut Ui| {
                    let known: Vec<i64> = ids.iter().copied().filter(|id| s.cat.by_id.contains_key(id)).collect();
                    if let Some(start) = known.iter().position(|id| *id == target) {
                        let (cat, b) = (s.cat.clone(), s.backend.clone());
                        s.player.play_list(&known, start, Some(false), &cat, &b);
                    }
                })),
            }
        })
        .collect();

    let top_artist = music.top_artists.iter().map(|x| x.plays).max().unwrap_or(1);
    let artists = music
        .top_artists
        .iter()
        .enumerate()
        .map(|(i, x)| {
            let name = x.name.clone();
            Rank {
                key: format!("st-artist-{i}"),
                art: cover(ui, x.cover.as_deref(), &x.name, 40.0, 20.0, CoverKind::Artist),
                round: true,
                title: x.name.clone(),
                sub: fmt_duration(x.seconds),
                value: plays(x.plays),
                share: share(x.plays, top_artist),
                on_press: Some(Box::new(move |s: &mut Ui| {
                    if let Some(a) = s.cat.artists.iter().find(|a| a.name == name) {
                        let key = a.key.clone();
                        s.navigate(Route::Artist(key));
                    }
                })),
            }
        })
        .collect();

    let top_album = music.top_albums.iter().map(|x| x.plays).max().unwrap_or(1);
    let albums = music
        .top_albums
        .iter()
        .enumerate()
        .map(|(i, x)| {
            let track = x.track_id;
            Rank {
                key: format!("st-album-{i}"),
                art: cover(
                    ui,
                    x.cover.as_deref(),
                    &format!("{}{}", x.album, x.album),
                    40.0,
                    p.r(8.0),
                    CoverKind::Music,
                ),
                round: false,
                title: x.album.clone(),
                sub: x.artist.clone().filter(|a| !a.is_empty()).unwrap_or_else(|| "—".into()),
                value: plays(x.plays),
                share: share(x.plays, top_album),
                on_press: Some(Box::new(move |s: &mut Ui| {
                    if let Some(key) = s.cat.entry(track).and_then(|e| e.album_key.clone()) {
                        s.navigate(Route::Album(key));
                    }
                })),
            }
        })
        .collect();

    vec![
        card(p, Some(t("stats.topTracks")), vec![rank_list(p, tracks)]),
        card(p, Some(t("stats.topArtists")), vec![rank_list(p, artists)]),
        card(p, Some(t("stats.topAlbums")), vec![rank_list(p, albums)]),
    ]
}

/// Films, séries, animés (et vidéos locales).
fn video_block(ui: &Ui, p: &Palette, data: &Arc<Stats>, width: f32) -> El {
    let video = &data.video;
    let kpis = tiles(
        vec![
            kpi(p, t("stats.watched"), fmt_duration(video.seconds), None, &icons::POPCORN),
            kpi(p, t("stats.titlesLabel"), count(video.titles), None, &icons::CLAPPERBOARD),
            kpi(p, t("stats.finished"), count(video.finished), Some(t("stats.finishedSub")), &icons::CIRCLE_CHECK),
        ],
        3,
    );
    // Grille « auto-fill » de 116 points au moins, comme `.st-posters`.
    let inner = (width - 2.0 * SIDE - 40.0).max(116.0);
    let cols = (((inner + 14.0) / (116.0 + 14.0)).floor() as usize).max(1);
    let posters: Vec<El> = video
        .top
        .iter()
        .take(8)
        .enumerate()
        .map(|(i, x)| {
            let local_id = (x.kind == "local").then(|| x.id.parse::<i64>().ok()).flatten();
            let art = local_id
                .and_then(|id| ui.video_thumb(id))
                .map(|img| el().w_full().h_full().pixels_fit(img.pixels(), SurfaceFit::Fill));
            let kind = x.kind.clone();
            poster(
                p,
                format!("st-poster-{i}"),
                art,
                i + 1,
                x.title.clone().unwrap_or_else(|| "—".into()),
                fmt_duration(x.seconds),
                move |s: &mut Ui| {
                    if let Some(id) = local_id {
                        if s.cat.by_id.contains_key(&id) {
                            let (cat, b) = (s.cat.clone(), s.backend.clone());
                            s.player.play_list(&[id], 0, None, &cat, &b);
                        }
                    } else {
                        // Pas encore de fiche d'un titre : l'accueil de son catalogue.
                        let route = if kind == "anime" {
                            crate::views::watch::WatchRoute::Anime
                        } else {
                            crate::views::watch::WatchRoute::Movies
                        };
                        s.navigate(Route::Watch(route));
                    }
                },
            )
        })
        .collect();
    let mut block = col().w_full().gap(12.0).child(block_title(p, t("stats.video"))).child(kpis).child(card(
        p,
        Some(t("stats.mostWatched")),
        vec![blocks::grid_gap(posters, cols, 14.0, 16.0)],
    ));
    if !video.genres.is_empty() {
        let top = video.genres.iter().map(|g| g.seconds).fold(1.0, f64::max) as f32;
        let rows = video
            .genres
            .iter()
            .enumerate()
            .map(|(i, g)| Rank {
                key: format!("st-vgenre-{i}"),
                art: genre_dot(p, i, 0.12),
                round: true,
                title: g.name.clone(),
                sub: tcount("stats.titles", g.titles),
                value: fmt_duration(g.seconds),
                share: g.seconds as f32 / top,
                on_press: None,
            })
            .collect();
        block = block.child(card(p, Some(t("stats.videoGenres")), vec![rank_list(p, rows)]));
    }
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_of_the_picker() {
        let today = parse_day("2026-11-20").expect("date");
        assert_eq!(range_for("7d", today), Range { from: "2026-11-14".into(), to: "2026-11-20".into(), days: Some(7) });
        assert_eq!(range_for("30d", today).from, "2026-10-22");
        assert_eq!(range_for("year", today), Range { from: "2026-01-01".into(), to: "2026-11-20".into(), days: None });
        assert_eq!(range_for("all", today).from, "");
        assert_eq!(peak(&[1.0, 3.0, 3.0, 0.0]), 1);
    }
}

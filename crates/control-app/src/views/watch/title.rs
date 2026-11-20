//! Fiche d'un film, d'une série ou d'un animé (port de `views/title.js`) : lecture ou reprise, « Ma liste »,
//! bande-annonce, épisodes avec leur progression, épisode ouvert avec précédent et suivant, casting, titres liés.

use rune::{Color, Cursor, Cx, El, SurfaceFit, TextCase, TextStyle, col, el, icon, row, text};
use serde_json::json;

use control_core::media::{Details, NextAiring, Person, Progress, neighbour};

use super::cards::{self, Item, POSTER_W, episode_tag, error_state, glass_pill, poster_card, poster_shelf, shelf};
use super::home::content_width;
use super::play::{self, Meta};
use super::{Load, Opened, load_season, page};
use crate::format;
use crate::i18n::{self, t, tn, tv};
use crate::icons;
use crate::state::Ui;
use crate::theme::{Palette, rgba};
use crate::ui::common::{hsl, seed_hues, ty};
use crate::views::segmented;

/// Épisodes d'animé par onglet.
pub const EP_GROUP: usize = 50;

pub const BLOCK_TITLE: TextStyle = TextStyle::new(20.0, 700.0).tracking(-0.4);

/// Où reprendre : épisode en cours, épisode suivant, ou début (avec le texte du grand bouton).
#[derive(Debug, Clone, PartialEq)]
pub struct Goal {
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub label: String,
    pub sub: Option<String>,
}

fn is_movie(kind: &str, d: &Details) -> bool {
    kind == "movie" || matches!(d, Details::Anime(a) if a.is_movie)
}

/// Port de `target()` : d'après la progression (la plus récente d'abord).
pub fn goal(kind: &str, d: &Details, progress: &[Progress]) -> Goal {
    let last = progress.first();
    let left = |r: &Progress| tv("resume.left", &[("t", &format::long(r.duration - r.position))]);
    if is_movie(kind, d) {
        let resume = last.filter(|l| !l.finished && l.position >= 30.0);
        return Goal {
            season: None,
            episode: None,
            label: if resume.is_some() { t("title.resume") } else { t("common.play") },
            sub: resume.map(left),
        };
    }
    let first_season = match d {
        Details::Tmdb(t) if kind == "tv" => Some(t.seasons.first().map_or(1, |s| s.number)),
        _ => None,
    };
    let start =
        |label: String| Goal { season: first_season, episode: Some(1), label, sub: episode_tag(first_season, Some(1)) };
    match last {
        None => start(t("common.play")),
        Some(l) if !l.finished => {
            // Épisode entamé : on y retourne, avec reprise seulement s'il a vraiment avancé.
            let far = l.position >= 30.0;
            let tag = episode_tag(l.season, l.episode).unwrap_or_default();
            Goal {
                season: l.season,
                episode: l.episode,
                label: if far { t("title.resume") } else { t("common.play") },
                sub: Some(if far { format!("{tag} · {}", left(l)) } else { tag }),
            }
        }
        Some(l) => match neighbour(d, l.season, l.episode, 1) {
            Some(n) => Goal {
                season: n.season,
                episode: Some(n.episode),
                label: t("title.next"),
                sub: episode_tag(n.season, Some(n.episode)),
            },
            None => start(t("title.again")),
        },
    }
}

/// Les détails viennent d'arriver : saison (séries) ou groupe d'épisodes (animés) de l'épisode à reprendre.
pub fn pick_start(s: &mut Ui) {
    let page = &s.watch.title;
    let Some(d) = page.details.ready() else { return };
    let g = goal(&page.kind, d, &page.progress);
    match d {
        Details::Tmdb(td) if page.kind == "tv" => {
            let season = g.season.or_else(|| td.seasons.first().map(|x| x.number)).unwrap_or(1);
            s.watch.title.season = Some(season);
            load_season(s, season);
        }
        Details::Anime(a) if !a.is_movie => {
            s.watch.title.group = (g.episode.unwrap_or(1).max(1) as usize - 1) / EP_GROUP;
        }
        _ => {}
    }
}

/// Date courte d'une diffusion (« 5 avr. 2023 », « Apr 5, 2023 »).
pub fn short_date(iso: &str) -> Option<String> {
    let mut parts = iso.split('-').map(|x| x.parse::<u32>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    if !(1..=12).contains(&m) {
        return None;
    }
    Some(match i18n::current() {
        i18n::Lang::Fr => {
            const M: [&str; 12] =
                ["janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc."];
            format!("{d} {} {y}", M[m as usize - 1])
        }
        i18n::Lang::En => {
            const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
            format!("{} {d}, {y}", M[m as usize - 1])
        }
    })
}

/// Date du prochain épisode (« vendredi 12 décembre », « Friday, December 12 »).
pub fn airing_date(ms: i64) -> String {
    let dt = control_core::util::local_datetime(ms);
    let (wd, day, month) =
        (dt.weekday().number_days_from_monday() as usize, dt.day(), u8::from(dt.month()) as usize - 1);
    match i18n::current() {
        i18n::Lang::Fr => {
            const W: [&str; 7] = ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"];
            const M: [&str; 12] = [
                "janvier",
                "février",
                "mars",
                "avril",
                "mai",
                "juin",
                "juillet",
                "août",
                "septembre",
                "octobre",
                "novembre",
                "décembre",
            ];
            format!("{} {day} {}", W[wd], M[month])
        }
        i18n::Lang::En => {
            const W: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
            const M: [&str; 12] = [
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ];
            format!("{}, {} {day}", W[wd], M[month])
        }
    }
}

pub fn view(ui: &Ui, p: &Palette, cx: &mut Cx<Ui>, kind: &str, id: &str) -> El {
    let key = format!("watch-title-{kind}-{id}");
    let page_state = &ui.watch.title;
    if page_state.kind != kind || page_state.id != id {
        return page(key, vec![skeleton(p)]);
    }
    let d = match &page_state.details {
        Load::Loading => return page(key, vec![skeleton(p)]),
        Load::Failed(err) => {
            let (k, i) = (kind.to_string(), id.to_string());
            return page(key, vec![error_state(p, "title-retry", err, move |s: &mut Ui| super::load_title(s, &k, &i))]);
        }
        Load::Ready(d) => d,
    };
    let width = content_width(cx);
    let mut parts = vec![hero(ui, p, kind, id, d)];
    if let Some(o) = ui.watch.opened.as_ref().filter(|o| o.kind == kind && o.id == id) {
        parts.push(opened_bar(ui, p, d, o));
    }
    let series = kind == "tv" || matches!(d, Details::Anime(a) if !a.is_movie);
    if series {
        parts.push(super::episodes::episodes(ui, p, kind, id, d));
    }
    match d {
        Details::Anime(a) => {
            if !a.related.is_empty() {
                let cards = a
                    .related
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        let item = Item::from_anime(&r.item);
                        let sub = [Some(relation_label(&r.relation)), item.year.map(|y| y.to_string())]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" · ");
                        poster_card(ui, p, format!("rel-{i}"), &item, POSTER_W, sub, None)
                    })
                    .collect();
                parts.push(shelf(ui, p, "related", &t("title.related"), None, width, 8.0 + POSTER_W * 0.75, cards));
            }
            parts.extend(cast_row(ui, p, &a.cast, width));
            let recs: Vec<Item> = a.recommendations.iter().map(Item::from_anime).collect();
            if !recs.is_empty() {
                parts.push(poster_shelf(ui, p, "recs", &t("title.recommended"), None, width, &recs, false));
            }
        }
        Details::Tmdb(td) => {
            parts.extend(cast_row(ui, p, &td.cast, width));
            let recs: Vec<Item> = td.recommendations.iter().map(Item::from_tmdb).collect();
            if !recs.is_empty() {
                parts.push(poster_shelf(ui, p, "recs", &t("title.recommended"), None, width, &recs, false));
            }
            let similar: Vec<Item> = td.similar.iter().map(Item::from_tmdb).collect();
            if !similar.is_empty() {
                parts.push(poster_shelf(ui, p, "similar", &t("title.similar"), None, width, &similar, false));
            }
        }
    }
    page(key, parts)
}

fn skeleton(p: &Palette) -> El {
    col()
        .w_full()
        .gap(14.0)
        .child(el().w_full().h(420.0).radius(p.r(24.0)).fill(p.fill))
        .child(el().w_frac(0.6).h(18.0).radius(p.r(8.0)).fill(p.fill))
        .child(el().w_frac(0.35).h(18.0).radius(p.r(8.0)).fill(p.fill))
}

fn relation_label(rel: &str) -> String {
    t(match rel {
        "PREQUEL" => "rel.prequel",
        "SEQUEL" => "rel.sequel",
        "PARENT" => "rel.parent",
        "SUMMARY" => "rel.summary",
        "SIDE_STORY" => "rel.side",
        "SPIN_OFF" => "rel.spinoff",
        _ => "rel.alt",
    })
}

// ─── Bannière de la fiche ───────────────────────────────────────────────────────────────────────────────────────

/// Fond pleine taille : l'image distante, sinon un dégradé tiré du titre.
fn backdrop_fill(ui: &Ui, url: Option<&str>, seed: &str) -> El {
    let (h1, h2) = seed_hues(seed);
    let base = el()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .linear_gradient(150f32.to_radians(), [(0.0, hsl(h1, 0.42, 0.30)), (1.0, hsl(h2, 0.40, 0.14))]);
    match ui.watch.images.get(ui, url) {
        Some(img) => base.child(el().w_full().h_full().pixels_fit(img.pixels(), SurfaceFit::Fill)),
        None => base,
    }
}

fn meta_chips(kind: &str, d: &Details) -> El {
    let white = Color::WHITE.alpha(0.88);
    let style = TextStyle::new(13.0, 560.0);
    let mut r = row().flex_wrap().gap(14.0).items_center();
    let plain = |s: String| text(s).font(style).color(white);
    let (year, runtime, rating) = match d {
        Details::Tmdb(td) => (td.item.year, td.runtime, td.item.rating),
        Details::Anime(a) => (a.year, a.runtime, a.rating),
    };
    if let Some(y) = year {
        r = r.child(plain(y.to_string()));
    }
    if let Some(m) = runtime {
        r = r.child(plain(format::long(m as f64 * 60.0)));
    }
    if let Details::Tmdb(td) = d
        && let Some(c) = td.certification.as_deref().filter(|c| !c.is_empty())
    {
        r = r.child(
            text(c.to_string())
                .font(TextStyle::new(11.5, 700.0))
                .color(white)
                .px(7.0)
                .py(1.0)
                .radius(5.0)
                .border(1.0, Color::WHITE.alpha(0.5)),
        );
    }
    if let Some(v) = rating {
        r = r.child(
            row()
                .gap(4.0)
                .items_center()
                .child(icon(&icons::STAR, 13.0).color(rgba(255, 214, 10, 1.0)))
                .child(plain(format!("{v:.1}"))),
        );
    }
    match d {
        Details::Tmdb(td) if kind == "tv" => {
            if let Some(n) = td.number_of_seasons {
                r = r.child(plain(tn("title.seasons", n)));
            }
        }
        Details::Anime(a) => {
            if !a.is_movie && !a.episodes.is_empty() {
                r = r.child(plain(tn("title.episodes", a.episodes.len())));
            }
            if !a.format.is_empty() && a.format != "TV" {
                r = r.child(plain(a.format.replace('_', " ")));
            }
        }
        _ => {}
    }
    r
}

fn credits(kind: &str, d: &Details) -> El {
    let mut lines: Vec<(String, String)> = Vec::new();
    match d {
        Details::Tmdb(td) => {
            if !td.creators.is_empty() {
                lines.push((
                    if kind == "movie" { t("title.directedBy") } else { t("title.createdBy") },
                    td.creators.join(", "),
                ));
            }
        }
        Details::Anime(a) => {
            if !a.studios.is_empty() {
                lines.push((t("title.studio"), a.studios.join(", ")));
            }
            if let Some(NextAiring { episode, at }) = a.next_airing {
                lines.push((t("title.nextEpisode"), format!("{} {episode} · {}", t("ep.short"), airing_date(at))));
            }
        }
    }
    let mut r = row().flex_wrap().gap(28.0).mt(8.0);
    for (k, v) in lines {
        r = r.child(
            row()
                .gap(8.0)
                .child(text(k).font(TextStyle::new(12.5, 400.0)).color(Color::WHITE.alpha(0.62)))
                .child(text(v).font(TextStyle::new(12.5, 600.0)).color(Color::WHITE.alpha(0.92))),
        );
    }
    r
}

/// Grand bouton de lecture : libellé et, dessous, l'épisode visé.
fn play_button(p: &Palette, g: &Goal) -> El {
    let mut stack = col().child(text(g.label.clone()).font(TextStyle::new(14.5, 650.0)).color(p.on_accent));
    if let Some(sub) = &g.sub {
        stack = stack.child(text(sub.clone()).font(TextStyle::new(11.5, 520.0)).color(p.on_accent.alpha(0.85)));
    }
    row()
        .key("title-play")
        .h(46.0)
        .pl(18.0)
        .pr(22.0)
        .gap(10.0)
        .items_center()
        .radius(p.r(12.0))
        .fill(p.accent)
        .hover(Color::WHITE.alpha(0.12))
        .cursor(Cursor::Pointer)
        .press_scale(0.98)
        .role("button")
        .label(g.label.clone())
        .child(icon(&icons::PLAY, 18.0).color(p.on_accent))
        .child(stack)
}

fn hero(ui: &Ui, p: &Palette, kind: &str, id: &str, d: &Details) -> El {
    let meta = Meta::of(d);
    let (overview, tagline, original, trailer, genres) = match d {
        Details::Tmdb(td) => {
            (td.item.overview.clone(), td.tagline.clone(), String::new(), td.trailer.clone(), td.genres.clone())
        }
        Details::Anime(a) => (
            a.overview.clone(),
            a.tagline.clone(),
            if a.original != a.title { a.original.clone() } else { String::new() },
            a.trailer.clone(),
            a.genres.iter().map(|g| cards::anime_genre_label(g)).collect(),
        ),
    };
    let g = goal(kind, d, &ui.watch.title.progress);
    let (k, i) = (kind.to_string(), id.to_string());
    let (season, episode) = (g.season, g.episode);
    let mut actions = row()
        .flex_wrap()
        .gap(8.0)
        .items_center()
        .mt(10.0)
        .child(play_button(p, &g).on_press(move |s: &mut Ui| play::open(s, &k, &i, season, episode)));
    let listed = ui.watch.in_list(kind, id);
    let list_item = Item {
        kind: kind.to_string(),
        id: id.to_string(),
        title: meta.title.clone(),
        poster: meta.poster.clone(),
        backdrop: meta.backdrop.clone(),
        year: meta.year,
        rating: match d {
            Details::Tmdb(td) => td.item.rating,
            Details::Anime(a) => a.rating,
        },
        ..Default::default()
    };
    actions = actions.child(
        glass_pill(
            p,
            "title-list",
            &if listed { t("list.inList") } else { t("list.add") },
            Some(if listed { &icons::BOOKMARK_CHECK } else { &icons::BOOKMARK_PLUS }),
            false,
        )
        .h(46.0)
        .px(16.0)
        .on_press(move |s: &mut Ui| cards::toggle_list(s, list_item.clone())),
    );
    if let Some(yt) = trailer.filter(|y| !y.is_empty()) {
        actions = actions.child(
            glass_pill(p, "title-trailer", &t("title.trailer"), Some(&icons::CLAPPERBOARD), false)
                .h(46.0)
                .px(16.0)
                .on_press(move |_: &mut Ui| crate::os::open_url(&format!("https://www.youtube.com/watch?v={yt}"))),
        );
    }
    if kind == "anime" {
        let current = if ui.settings.media.dub { "dub" } else { "sub" };
        actions = actions.child(segmented(
            p,
            "title-dub",
            vec![("sub".into(), t("dub.sub")), ("dub".into(), t("dub.en"))],
            current,
            |s: &mut Ui, v: String| s.set(json!({ "media": { "dub": v == "dub" } })),
        ));
    }

    let mut info = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(8.0)
        .pb(2.0)
        .child(
            text(cards::kind_label(kind))
                .font(TextStyle::new(12.0, 700.0).tracking(1.2))
                .text_case(TextCase::Uppercase)
                .color(Color::WHITE.alpha(0.75)),
        )
        .child(text(meta.title.clone()).font(cards::HERO_TITLE).color(Color::WHITE).max_lines(2));
    if !original.is_empty() {
        info = info.child(text(original).font(ty::BODY).color(Color::WHITE.alpha(0.6)));
    }
    info = info.child(meta_chips(kind, d));
    if !genres.is_empty() {
        let mut gr = row().flex_wrap().gap(6.0);
        for g in genres {
            gr = gr.child(
                text(g)
                    .font(TextStyle::new(12.0, 560.0))
                    .color(Color::WHITE)
                    .px(11.0)
                    .py(3.0)
                    .radius(12.0)
                    .fill(Color::WHITE.alpha(0.16)),
            );
        }
        info = info.child(gr);
    }
    if !tagline.is_empty() {
        info = info.child(text(tagline).font(TextStyle::new(13.0, 400.0).italic()).color(Color::WHITE.alpha(0.78)));
    }
    if !overview.is_empty() {
        info = info.child(
            text(overview)
                .font(TextStyle::new(13.0, 400.0).lh(19.5))
                .color(Color::WHITE.alpha(0.88))
                .max_lines(5)
                .max_w(720.0)
                .mt(2.0),
        );
    }
    info = info.child(actions).child(credits(kind, d));

    let r = p.r(24.0);
    let poster_r = p.r(14.0);
    el().key("title-hero")
        .w_full()
        .min_h(440.0)
        .radius(r)
        .clip_rounded(r)
        .fill(rgba(17, 17, 17, 1.0))
        .shadow(24.0, 60.0, Color::BLACK.alpha(0.4))
        .child(backdrop_fill(ui, meta.backdrop.as_deref().or(meta.poster.as_deref()), &meta.title))
        .child(cards::shade())
        .child(
            row()
                .w_full()
                .min_h(440.0)
                .items_end()
                .gap(32.0)
                .padding(36.0)
                .child(
                    el().size(200.0, 300.0)
                        .radius(poster_r)
                        .shadow(24.0, 50.0, Color::BLACK.alpha(0.55))
                        .child(cards::art(ui, meta.poster.as_deref(), &meta.title, kind, 200.0, 300.0, poster_r, true)),
                )
                .child(info),
        )
}

// ─── Épisode ouvert ─────────────────────────────────────────────────────────────────────────────────────────────

fn episode_name(ui: &Ui, d: &Details, season: Option<i64>, episode: Option<i64>) -> String {
    let ep = episode.unwrap_or(0);
    match d {
        Details::Anime(a) => a.episodes.iter().find(|e| e.number == ep).map(|e| e.name.clone()).unwrap_or_default(),
        Details::Tmdb(_) => season
            .and_then(|s| ui.watch.title.seasons.get(&s))
            .and_then(Load::ready)
            .and_then(|data| data.episodes.iter().find(|e| e.number == ep).map(|e| e.name.clone()))
            .unwrap_or_default(),
    }
}

/// Ce qui remplace la page de lecture : l'épisode ouvert dans le navigateur, précédent, suivant, « vu », rouvrir.
fn opened_bar(ui: &Ui, p: &Palette, d: &Details, o: &Opened) -> El {
    let movie = is_movie(&o.kind, d);
    let title = if movie {
        Meta::of(d).title
    } else {
        [episode_tag(o.season, o.episode), Some(episode_name(ui, d, o.season, o.episode)).filter(|n| !n.is_empty())]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" — ")
    };
    let finished = ui.watch.title.progress.iter().any(|r| r.season == o.season && r.episode == o.episode && r.finished);
    let small = |key: &str, label: String, glyph: &'static rune::Icon, enabled: bool| {
        crate::ui::common::button(p, key, &label, Some(glyph), false).opacity(if enabled { 1.0 } else { 0.4 })
    };
    let mut actions = row().gap(8.0).items_center().flex_wrap();
    if !movie {
        for (dir, key, label, glyph) in [
            (-1, "opened-prev", t("watch.prev"), &icons::SKIP_BACK),
            (1, "opened-next", t("watch.next"), &icons::SKIP_FORWARD),
        ] {
            let target = neighbour(d, o.season, o.episode, dir);
            let mut b = small(key, label, glyph, target.is_some());
            if let Some(n) = target {
                let (k, i) = (o.kind.clone(), o.id.clone());
                b = b.on_press(move |s: &mut Ui| {
                    if k == "tv"
                        && let Some(season) = n.season
                    {
                        s.watch.title.season = Some(season);
                        load_season(s, season);
                    }
                    play::open(s, &k, &i, n.season, Some(n.episode));
                });
            }
            actions = actions.child(b);
        }
    }
    if !finished {
        let opened = o.clone();
        actions = actions.child(
            small("opened-seen", t("watch.markSeen"), &icons::CIRCLE_CHECK, true)
                .on_press(move |s: &mut Ui| play::mark_seen(s, opened.clone())),
        );
    }
    let again = o.clone();
    actions = actions.child(
        small("opened-again", t("watch.reopen"), &icons::EXTERNAL_LINK, true)
            .on_press(move |s: &mut Ui| play::open(s, &again.kind, &again.id, again.season, again.episode)),
    );
    col()
        .key("opened-bar")
        .w_full()
        .mt(18.0)
        .padding(16.0)
        .gap(12.0)
        .radius(p.r(16.0))
        .fill(p.fill)
        .border(0.5, p.separator)
        .child(
            row()
                .w_full()
                .gap(14.0)
                .items_center()
                .justify_between()
                .flex_wrap()
                .child(
                    row()
                        .gap(12.0)
                        .items_center()
                        .min_w(0.0)
                        .shrink()
                        .child(
                            row()
                                .size(36.0, 36.0)
                                .radius(18.0)
                                .center()
                                .fill(p.accent_soft)
                                .child(icon(&icons::MONITOR_PLAY, 18.0).color(p.accent)),
                        )
                        .child(
                            col()
                                .min_w(0.0)
                                .shrink()
                                .child(text(t("watch.openedIn")).font(ty::SMALL).color(p.text2))
                                .child(text(title).font(ty::HEADLINE).color(p.text).truncate()),
                        ),
                )
                .child(actions),
        )
        .child(text(t("watch.browserNote")).font(ty::SMALL).color(p.text3).wrap())
}

// ─── Casting ────────────────────────────────────────────────────────────────────────────────────────────────────

fn cast_row(ui: &Ui, p: &Palette, cast: &[Person], width: f32) -> Option<El> {
    let cards: Vec<El> = cast
        .iter()
        .filter(|c| !c.name.is_empty())
        .enumerate()
        .map(|(i, c)| {
            let photo = el().size(92.0, 92.0).radius(46.0).clip_rounded(46.0).fill(p.fill_strong).shadow(
                6.0,
                16.0,
                Color::BLACK.alpha(0.25),
            );
            let photo = match ui.watch.images.get(ui, c.profile.as_deref()) {
                Some(img) => photo.child(el().w_full().h_full().pixels_fit(img.pixels(), SurfaceFit::Fill)),
                None => photo.child(
                    row().w_full().h_full().center().child(
                        text(c.name.chars().next().map(String::from).unwrap_or_default())
                            .font(TextStyle::new(28.0, 700.0))
                            .color(p.text3),
                    ),
                ),
            };
            col()
                .key(format!("cast-{i}"))
                .w(104.0)
                .items_center()
                .child(photo.mb(8.0))
                .child(
                    text(c.name.clone())
                        .font(TextStyle::new(12.5, 620.0))
                        .color(p.text)
                        .truncate()
                        .align(rune::Align::Center)
                        .w_full(),
                )
                .child(
                    text(c.role.clone())
                        .font(TextStyle::new(11.5, 400.0))
                        .color(p.text3)
                        .truncate()
                        .align(rune::Align::Center)
                        .w_full(),
                )
        })
        .collect();
    (!cards.is_empty()).then(|| shelf(ui, p, "cast", &t("title.cast"), None, width, 8.0 + 46.0, cards))
}

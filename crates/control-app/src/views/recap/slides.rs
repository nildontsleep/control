//! Cartes du récap (musique, habitudes, films, résumé) et leur décor, comme `buildSlides` de `wrapped.js`.
//! Les mêmes fonctions dessinent la fenêtre du récap et l'image enregistrée (`export.rs`).

use rune::{Align, Color, El, Image, SurfaceFit, TextStyle, Transition, col, el, icon, row, text};
use time::Date;

use control_core::stats::Stats;

use crate::format::count;
use crate::i18n::{t, tv};
use crate::icons;
use crate::theme::hex;
use crate::ui::common::{hsl, seed_hues};
use crate::views::recap::period::{Kind, Period};
use crate::views::stats::dates::{day_month, fmt_duration, monday_of, parse_day, plays, weekday_day_month};
use crate::views::stats::peak;

/// Un décor différent par carte (dégradés qui se fondent d'une carte à l'autre).
const PALETTES: [[&str; 3]; 6] = [
    ["#ff375f", "#7a3cff", "#ff9f0a"],
    ["#0a84ff", "#bf5af2", "#30d158"],
    ["#ff9f0a", "#ff375f", "#5e5ce6"],
    ["#30d158", "#0a84ff", "#64d2ff"],
    ["#bf5af2", "#ff375f", "#ff9f0a"],
    ["#64d2ff", "#5e5ce6", "#ff375f"],
];

/// Fond de la scène (`#0b0b10`).
pub fn stage_bg() -> Color {
    crate::theme::rgba(11, 11, 16, 1.0)
}

/// Les trois couleurs du décor de la carte `index`.
pub fn palette(index: usize) -> [Color; 3] {
    PALETTES[index % PALETTES.len()].map(|h| hex(h).unwrap_or(Color::WHITE))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slide {
    Empty,
    Intro,
    Time,
    Artist,
    Tracks,
    Sound,
    Habits,
    Video,
    Summary,
}

fn is_empty(data: &Stats) -> bool {
    data.music.plays == 0 && data.video.seconds < 60.0
}

/// Les cartes d'une période, dans l'ordre.
pub fn slides(data: &Stats) -> Vec<Slide> {
    if is_empty(data) {
        return vec![Slide::Empty];
    }
    let music = &data.music;
    let mut out = vec![Slide::Intro];
    if music.plays > 0 {
        out.push(Slide::Time);
        if !music.top_artists.is_empty() {
            out.push(Slide::Artist);
        }
        if !music.top_tracks.is_empty() {
            out.push(Slide::Tracks);
        }
        if !music.genres.is_empty() || !music.top_albums.is_empty() {
            out.push(Slide::Sound);
        }
        out.push(Slide::Habits);
    }
    if data.video.seconds >= 60.0 {
        out.push(Slide::Video);
    }
    out.push(Slide::Summary);
    out
}

/// Ce qu'il faut pour dessiner une carte.
pub struct Ctx<'a> {
    pub period: Period,
    pub data: &'a Stats,
    /// Pochette par empreinte (petite ou grande), si elle est chargée.
    pub cover: &'a dyn Fn(Option<&str>, bool) -> Option<Image>,
    /// Hauteur de la scène, pour les tailles en `vh` de l'original.
    pub height: f32,
    pub today: Date,
    /// Bouton « Enregistrer l'image » de la dernière carte (absent de l'image elle-même).
    pub save: Option<El>,
}

const WHITE: Color = Color::WHITE;

fn clamp_vh(ctx: &Ctx, min: f32, vh: f32, max: f32) -> f32 {
    (ctx.height * vh / 100.0).clamp(min, max)
}

fn style(size: f32, weight: f32, em: f32) -> TextStyle {
    TextStyle::new(size, weight).tracking(size * em)
}

fn eyebrow(s: String) -> El {
    text(s.to_uppercase()).font(style(13.0, 700.0, 0.16)).color(WHITE.alpha(0.72)).align(Align::Center)
}

fn lead(s: String) -> El {
    text(s).font(TextStyle::new(18.0, 400.0).lh(25.0)).color(WHITE.alpha(0.85)).wrap().align(Align::Center).max_w(440.0)
}

fn big(ctx: &Ctx, s: String, small: bool) -> El {
    let size = if small { clamp_vh(ctx, 34.0, 6.0, 52.0) } else { clamp_vh(ctx, 38.0, 7.0, 62.0) };
    text(s).font(style(size, 780.0, -0.035).lh(size * 1.08)).color(WHITE).wrap().align(Align::Center).max_w(584.0)
}

fn giant(ctx: &Ctx, s: String) -> El {
    let size = clamp_vh(ctx, 80.0, 18.0, 170.0);
    text(s).font(style(size, 800.0, -0.05).features("'tnum' 1").lh(size)).color(WHITE)
}

fn title(ctx: &Ctx, s: String) -> El {
    text(s).font(style(clamp_vh(ctx, 24.0, 4.0, 34.0), 700.0, -0.02)).color(WHITE).align(Align::Center).wrap()
}

/// Le grand titre : l'année en géant, les titres plus longs un peu plus petits.
fn huge(ctx: &Ctx) -> El {
    let label = ctx.period.title();
    let (size, em) = if ctx.period.kind() == Kind::Year {
        (clamp_vh(ctx, 110.0, 24.0, 220.0), -0.06)
    } else if label.chars().count() > 9 {
        (clamp_vh(ctx, 54.0, 11.0, 104.0), -0.045)
    } else {
        (clamp_vh(ctx, 64.0, 15.0, 140.0), -0.045)
    };
    text(label).font(style(size, 820.0, em).lh(size * 0.95)).color(WHITE).align(Align::Center)
}

/// Visuel arrondi : la pochette si elle est chargée, sinon un dégradé déterministe.
fn art(ctx: &Ctx, cover: Option<&str>, seed: &str, size: f32, small: bool, round: bool, radius: f32) -> El {
    let r = if round { size / 2.0 } else { radius };
    let (h1, h2) = seed_hues(if seed.is_empty() { "x" } else { seed });
    let mut a = el()
        .size(size, size)
        .radius(r)
        .clip_rounded(r)
        .linear_gradient(150f32.to_radians(), [(0.0, hsl(h1, 0.64, 0.60)), (1.0, hsl(h2, 0.58, 0.32))])
        .shadow(18.0, 50.0, Color::BLACK.alpha(0.4))
        .border(0.5, WHITE.alpha(0.2));
    if let Some(img) = (ctx.cover)(cover, small) {
        a = a.child(el().w_full().h_full().pixels_fit(img.pixels(), SurfaceFit::Fill));
    }
    a
}

fn chip(s: String) -> El {
    row()
        .px(13.0)
        .py(6.0)
        .radius(999.0)
        .fill(WHITE.alpha(0.18))
        .child(text(s).font(TextStyle::new(14.0, 600.0)).color(WHITE))
}

fn chips(items: Vec<String>) -> El {
    row().flex_wrap().justify_center().gap(8.0).max_w(584.0).children(items.into_iter().map(chip))
}

fn pill(glyph: &'static rune::Icon, s: String) -> El {
    row()
        .items_center()
        .gap(7.0)
        .px(16.0)
        .py(8.0)
        .radius(999.0)
        .fill(WHITE.alpha(0.18))
        .child(icon(glyph, 16.0).color(WHITE))
        .child(text(s).font(TextStyle::new(14.0, 650.0)).color(WHITE))
}

/// Pochette et libellés côte à côte (album n°1, titre n°1).
fn album_line(art: Option<El>, eyebrow_text: String, strong: String, sub: String) -> El {
    let mut r = row().items_center().gap(16.0).mt(14.0);
    if let Some(a) = art {
        r = r.child(a);
    }
    r.child(
        col()
            .gap(3.0)
            .min_w(0.0)
            .shrink()
            .child(text(eyebrow_text.to_uppercase()).font(style(13.0, 700.0, 0.16)).color(WHITE.alpha(0.72)))
            .child(text(strong).font(style(22.0, 700.0, -0.02)).color(WHITE).truncate())
            .child(text(sub).font(TextStyle::new(14.0, 400.0)).color(WHITE.alpha(0.75)).truncate()),
    )
}

/// « Aujourd'hui », « Cette semaine » quand la période est en cours, sinon « Ce jour-là », « Cette semaine-là ».
fn listened_key(ctx: &Ctx) -> String {
    match ctx.period {
        Period::Day(d) if d == ctx.today => "wrapped.youListenedToday".into(),
        Period::Week(start) if start == monday_of(ctx.today) => "wrapped.youListenedThisWeek".into(),
        p => format!("wrapped.youListened{}", p.kind().suffix()),
    }
}

/// Le contenu d'une carte (sans le décor), ses éléments les uns sous les autres.
pub fn content(ctx: &Ctx, slide: Slide) -> Vec<El> {
    let (music, video) = (&ctx.data.music, &ctx.data.video);
    let sfx = ctx.period.kind().suffix();
    let mut out: Vec<El> = Vec::new();
    match slide {
        Slide::Empty => {
            out.push(eyebrow("control".into()));
            out.push(huge(ctx));
            out.push(lead(t(&format!("wrapped.empty{sfx}"))));
        }
        Slide::Intro => {
            out.push(eyebrow(ctx.period.eyebrow()));
            out.push(huge(ctx));
            let key =
                if music.plays > 0 { format!("wrapped.introMusic{sfx}") } else { format!("wrapped.introVideo{sfx}") };
            out.push(lead(t(&key)));
            if ctx.save.is_some() {
                out.push(
                    row()
                        .items_center()
                        .gap(4.0)
                        .mt(28.0)
                        .child(text(t("wrapped.tap")).font(TextStyle::new(13.0, 400.0)).color(WHITE.alpha(0.6)))
                        .child(icon(&icons::CHEVRON_RIGHT, 16.0).color(WHITE.alpha(0.6))),
                );
            }
        }
        Slide::Time => {
            out.push(eyebrow(t(&listened_key(ctx))));
            out.push(giant(ctx, count((music.seconds / 60.0).round() as i64)));
            out.push(title(ctx, t("wrapped.minutes")));
            let detail = if music.seconds < 3600.0 {
                tv("wrapped.timeDetailShort", &[("tracks", &count(music.tracks)), ("plays", &count(music.plays))])
            } else {
                let hours = (music.seconds / 360.0).round() / 10.0;
                let h = if hours.fract() == 0.0 { count(hours as i64) } else { decimal(hours) };
                tv("wrapped.timeDetail", &[("h", &h), ("tracks", &count(music.tracks)), ("plays", &count(music.plays))])
            };
            out.push(lead(detail));
        }
        Slide::Artist => {
            if let Some(a) = music.top_artists.first() {
                out.push(eyebrow(t("wrapped.topArtist")));
                out.push(art(ctx, a.cover.as_deref(), &a.name, art_l(ctx), false, true, 14.0));
                out.push(big(ctx, a.name.clone(), false));
                out.push(lead(tv(
                    "wrapped.artistDetail",
                    &[("plays", &count(a.plays)), ("time", &fmt_duration(a.seconds))],
                )));
                if music.top_artists.len() > 1 {
                    out.push(chips(
                        music
                            .top_artists
                            .iter()
                            .enumerate()
                            .skip(1)
                            .take(4)
                            .map(|(i, a)| format!("{}. {}", i + 1, a.name))
                            .collect(),
                    ));
                }
            }
        }
        Slide::Tracks => {
            out.push(eyebrow(t("wrapped.topTracks")));
            let mut list = col().w_full().gap(12.0).mt(8.0);
            for (i, x) in music.top_tracks.iter().take(5).enumerate() {
                let artist = if x.artist.is_empty() { "—".to_string() } else { x.artist.clone() };
                list = list.child(
                    row()
                        .key(format!("w-li-{i}"))
                        .w_full()
                        .items_center()
                        .gap(12.0)
                        .pt(8.0)
                        .pb(8.0)
                        .pl(8.0)
                        .pr(12.0)
                        .radius(16.0)
                        .fill(WHITE.alpha(0.14))
                        .transition(Transition::offset(0.0, 26.0).combined(Transition::opacity()))
                        .enter_delay(0.15 + i as f32 * 0.1)
                        .child(
                            text((i + 1).to_string())
                                .font(TextStyle::new(18.0, 800.0))
                                .color(WHITE.alpha(0.7))
                                .w(26.0)
                                .align(Align::Center),
                        )
                        .child(art(
                            ctx,
                            x.cover.as_deref(),
                            &format!("{}{}", x.title, x.album),
                            56.0,
                            true,
                            false,
                            10.0,
                        ))
                        .child(
                            col()
                                .grow(1.0)
                                .min_w(0.0)
                                .child(text(x.title.clone()).font(TextStyle::new(15.0, 700.0)).color(WHITE).truncate())
                                .child(
                                    text(artist).font(TextStyle::new(13.0, 400.0)).color(WHITE.alpha(0.7)).truncate(),
                                ),
                        )
                        .child(text(plays(x.plays)).font(TextStyle::new(13.0, 650.0)).color(WHITE.alpha(0.85))),
                );
            }
            out.push(list);
        }
        Slide::Sound => {
            if let Some(g) = music.genres.first() {
                out.push(eyebrow(t("wrapped.topGenre")));
                out.push(big(ctx, g.name.clone(), false));
                if music.genres.len() > 1 {
                    out.push(chips(music.genres.iter().skip(1).take(3).map(|g| g.name.clone()).collect()));
                }
            }
            if let Some(a) = music.top_albums.first() {
                let cover =
                    art(ctx, a.cover.as_deref(), &format!("{}{}", a.album, a.album), art_l(ctx), false, false, 14.0);
                let artist = a.artist.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "—".into());
                out.push(album_line(Some(cover), t("wrapped.topAlbum"), a.album.clone(), artist));
            }
        }
        Slide::Habits => {
            out.push(eyebrow(t("wrapped.habits")));
            // Pour une seule journée, « la journée la plus musicale » n'a pas de sens : seules les heures comptent.
            let busiest = if ctx.period.kind() == Kind::Day {
                None
            } else {
                music.by_day.iter().fold(None::<&control_core::stats::DayPlays>, |best, d| match best {
                    Some(b) if b.seconds >= d.seconds => Some(b),
                    _ => Some(d),
                })
            };
            if let Some(b) = busiest
                && let Some(date) = parse_day(&b.day)
            {
                let label = if ctx.period.kind() == Kind::Week { weekday_day_month(date) } else { day_month(date) };
                out.push(big(ctx, label, true));
                out.push(lead(tv("wrapped.busiestDay", &[("time", &fmt_duration(b.seconds))])));
            }
            out.push(hours_chart(&music.by_hour));
            out.push(lead(tv("wrapped.peakHour", &[("h", &peak(&music.by_hour))])));
            if ctx.data.streak.longest > 1 {
                out.push(pill(
                    &icons::FLAME,
                    crate::views::stats::dates::tcount("wrapped.streak", ctx.data.streak.longest as i64),
                ));
            }
        }
        Slide::Video => {
            out.push(eyebrow(t("wrapped.youWatched")));
            let hours = (video.seconds / 360.0).round() / 10.0;
            out.push(giant(ctx, if hours.fract() == 0.0 { count(hours as i64) } else { decimal(hours) }));
            out.push(title(ctx, t("wrapped.hours")));
            if let Some(top) = video.top.first() {
                out.push(album_line(
                    None,
                    t("wrapped.topTitle"),
                    top.title.clone().unwrap_or_else(|| "—".into()),
                    fmt_duration(top.seconds),
                ));
            }
        }
        Slide::Summary => {
            out.push(eyebrow(t("wrapped.summary")));
            out.push(summary(ctx));
            if let Some(save) = &ctx.save {
                out.push(save.clone());
            }
        }
    }
    out
}

/// « 12,5 » en français, « 12.5 » en anglais.
fn decimal(v: f64) -> String {
    let s = format!("{v:.1}");
    if crate::i18n::current() == crate::i18n::Lang::Fr { s.replace('.', ",") } else { s }
}

fn art_l(ctx: &Ctx) -> f32 {
    (ctx.height * 0.26).min(220.0)
}

/// Les 24 heures en barres, l'heure de pointe en blanc.
fn hours_chart(by_hour: &[f64]) -> El {
    let max = by_hour.iter().copied().fold(1.0, f64::max);
    let top = peak(by_hour);
    let mut r = row().w_full().max_w(420.0).h(96.0).my(8.0).gap(3.0).items_end();
    for (i, v) in by_hour.iter().enumerate() {
        let frac = ((v / max) as f32).max(0.04);
        r = r.child(el().grow(1.0).h(96.0 * frac).radius(3.0).fill(if i == top { WHITE } else { WHITE.alpha(0.38) }));
    }
    r
}

fn summary(ctx: &Ctx) -> El {
    let (music, video) = (&ctx.data.music, &ctx.data.video);
    let mut card = col()
        .w_full()
        .max_w(440.0)
        .gap(12.0)
        .px(24.0)
        .py(22.0)
        .radius(26.0)
        .fill(Color::BLACK.alpha(0.28))
        .border(0.5, WHITE.alpha(0.22))
        .child(
            row()
                .w_full()
                .items_end()
                .justify_between()
                .gap(12.0)
                .child(text(ctx.period.title()).font(style(40.0, 800.0, -0.04)).color(WHITE).truncate().shrink())
                .child(text(ctx.period.eyebrow()).font(TextStyle::new(13.0, 700.0)).color(WHITE.alpha(0.65)).pb(6.0)),
        );
    if music.plays > 0 {
        let stat = |value: String, label: String| {
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(text(value).font(style(30.0, 780.0, -0.03).features("'tnum' 1")).color(WHITE))
                .child(text(label).font(TextStyle::new(12.5, 400.0)).color(WHITE.alpha(0.65)))
        };
        card = card.child(
            row()
                .w_full()
                .gap(10.0)
                .child(stat(count((music.seconds / 60.0).round() as i64), t("wrapped.minutes")))
                .child(stat(count(music.plays), t("stats.playsLabel"))),
        );
    }
    let line = |label: String, value: String| {
        col()
            .w_full()
            .pt(10.0)
            .border_sides([0.5, 0.0, 0.0, 0.0], WHITE.alpha(0.18))
            .child(text(label.to_uppercase()).font(style(11.5, 650.0, 0.1)).color(WHITE.alpha(0.6)))
            .child(text(value).font(TextStyle::new(19.0, 650.0)).color(WHITE).truncate())
    };
    if let Some(a) = music.top_artists.first() {
        card = card.child(line(t("wrapped.topArtist"), a.name.clone()));
    }
    if let Some(x) = music.top_tracks.first() {
        card = card.child(line(t("wrapped.topTrack"), x.title.clone()));
    }
    if let Some(g) = music.genres.first() {
        card = card.child(line(t("wrapped.topGenre"), g.name.clone()));
    }
    if video.seconds >= 60.0 {
        card = card.child(line(t("wrapped.youWatched"), fmt_duration(video.seconds)));
    }
    card
}

/// La carte posée sur sa scène : ses éléments entrent l'un après l'autre.
pub fn stage_content(ctx: &Ctx, slide: Slide, index: usize, back: bool) -> El {
    let dy = if back { -18.0 } else { 26.0 };
    let mut c = col().key(format!("w-slide-{index}-{slide:?}")).w_full().items_center().gap(14.0);
    for (i, e) in content(ctx, slide).into_iter().enumerate() {
        c = c.child(
            col()
                .key(format!("w-in-{index}-{i}"))
                .w_full()
                .items_center()
                .transition(Transition::asymmetric(
                    Transition::offset(0.0, dy).combined(Transition::opacity()),
                    Transition::none(),
                ))
                .enter_delay(i as f32 * 0.08)
                .child(e),
        );
    }
    c
}

/// Le décor : trois halos colorés sur fond sombre, puis un léger vignettage.
pub fn backdrop(colors: [Color; 3], w: f32, h: f32) -> El {
    let (bw, bh) = (w * 1.4, h * 1.4);
    let glow = |center: [f32; 2], r: [f32; 2], c: Color, end: f32| {
        el().absolute(0.0, 0.0).size(bw, bh).radial_gradient(
            center,
            [r[0] * bw, r[1] * bh],
            [(0.0, c), (end, c.alpha(0.0)), (1.0, c.alpha(0.0))],
        )
    };
    let [g1, g2, g3] = colors;
    el().absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(stage_bg())
        .clip_soft(0.0)
        .child(
            el().key("w-bg")
                .absolute(-0.2 * w, -0.2 * h)
                .size(bw, bh)
                .opacity(0.85)
                .child(glow([0.18, 0.22], [0.6, 0.55], g1, 0.7))
                .child(glow([0.85, 0.80], [0.55, 0.6], g2, 0.7))
                .child(glow([0.78, 0.14], [0.4, 0.4], g3, 0.72)),
        )
        .child(el().absolute(0.0, 0.0).w_full().h_full().radial_gradient(
            [0.5, 0.4],
            [1.2 * w, 0.9 * h],
            [(0.0, Color::BLACK.alpha(0.0)), (0.4, Color::BLACK.alpha(0.0)), (1.0, Color::BLACK.alpha(0.45))],
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_follow_the_language() {
        let _guard = crate::i18n::TEST_LANG.lock().unwrap_or_else(|e| e.into_inner());
        crate::i18n::set_language("fr");
        assert_eq!(decimal(12.5), "12,5");
        crate::i18n::set_language("en");
        assert_eq!(decimal(12.5), "12.5");
        crate::i18n::set_language("fr");
    }
}

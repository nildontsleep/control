//! Briques des pages « Regarder » (port de `media/ui.js`) : affiches, cartes de reprise, rangées horizontales,
//! bannière, genres, états de chargement et d'erreur, carte de la clé TMDB.

use rune::{Color, Cursor, El, Icon, SurfaceFit, TextCase, TextStyle, col, el, icon, row, text};
use rune_kit::{Kit, TextInput};
use serde_json::json;

use control_core::media::{
    AnimeItem, CatalogItem, ErrorCode, ListEntry, MediaError, Progress, TmdbItem, WatchlistItem,
};

use super::{KeyStatus, WatchRoute};
use crate::format;
use crate::i18n::{self, t, tv};
use crate::icons;
use crate::state::{Route, SettingsTab, Sheet, Ui};
use crate::theme::{Palette, rgba};
use crate::ui::common::{button, empty_state, hsl, seed_hues, ty};

/// Largeur d'une affiche dans une rangée.
pub const POSTER_W: f32 = 168.0;
/// Largeur d'une carte de reprise (16:9).
pub const RESUME_W: f32 = 300.0;
/// Écart entre les cartes d'une rangée.
pub const ROW_GAP: f32 = 18.0;

pub const HERO_TITLE: TextStyle = TextStyle::new(40.0, 800.0).tracking(-1.2).lh(42.0);
pub const SHELF_TITLE: TextStyle = TextStyle::new(20.0, 700.0).tracking(-0.4);
const CAPTION_WIDE: TextStyle = TextStyle::new(12.0, 650.0).tracking(0.6);

/// Une affiche, quel que soit le catalogue d'où elle vient.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Item {
    /// `movie`, `tv` ou `anime`.
    pub kind: String,
    pub id: String,
    pub title: String,
    pub overview: String,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub year: Option<i64>,
    pub rating: Option<f64>,
    /// Format AniList (`TV`, `MOVIE`…), vide ailleurs.
    pub format: String,
}

impl Item {
    pub fn from_tmdb(i: &TmdbItem) -> Item {
        Item {
            kind: i.kind.clone(),
            id: i.id.clone(),
            title: i.title.clone(),
            overview: i.overview.clone(),
            poster: i.poster.clone(),
            backdrop: i.backdrop.clone(),
            year: i.year,
            rating: i.rating,
            format: String::new(),
        }
    }

    pub fn from_anime(a: &AnimeItem) -> Item {
        Item {
            kind: "anime".into(),
            id: a.id.clone(),
            title: a.title.clone(),
            overview: a.overview.clone(),
            poster: a.poster.clone(),
            backdrop: a.backdrop.clone(),
            year: a.year,
            rating: a.rating,
            format: a.format.clone(),
        }
    }

    pub fn from_catalog(c: &CatalogItem) -> Item {
        match c {
            CatalogItem::Tmdb(i) => Item::from_tmdb(i),
            CatalogItem::Anime(a) => Item::from_anime(a),
        }
    }

    pub fn from_saved(w: &WatchlistItem) -> Item {
        Item {
            kind: w.kind.clone(),
            id: w.id.clone(),
            title: w.title.clone(),
            poster: w.poster.clone(),
            backdrop: w.backdrop.clone(),
            year: w.year,
            rating: w.rating,
            ..Default::default()
        }
    }

    pub fn entry(&self) -> ListEntry {
        ListEntry {
            kind: self.kind.clone(),
            id: self.id.clone(),
            title: self.title.clone(),
            poster: self.poster.clone(),
            backdrop: self.backdrop.clone(),
            year: self.year,
            rating: self.rating,
        }
    }

    pub fn route(&self) -> Route {
        title_route(&self.kind, &self.id)
    }
}

pub fn title_route(kind: &str, id: &str) -> Route {
    Route::Watch(WatchRoute::Title { kind: kind.to_string(), id: id.to_string() })
}

pub fn kind_label(kind: &str) -> String {
    t(match kind {
        "tv" => "kind.tv",
        "anime" => "kind.anime",
        _ => "kind.movie",
    })
}

pub fn type_icon(kind: &str) -> &'static Icon {
    match kind {
        "tv" => &icons::TV,
        "anime" => &icons::TV_MINIMAL,
        "movie" => &icons::CLAPPERBOARD,
        _ => &icons::FILM,
    }
}

/// Libellé d'un genre AniList (traduit en français, tel quel en anglais).
pub fn anime_genre_label(name: &str) -> String {
    if i18n::current() == i18n::Lang::En {
        return name.to_string();
    }
    match name {
        "Adventure" => "Aventure",
        "Comedy" => "Comédie",
        "Drama" => "Drame",
        "Fantasy" => "Fantastique",
        "Horror" => "Horreur",
        "Mahou Shoujo" => "Magical girl",
        "Music" => "Musique",
        "Mystery" => "Mystère",
        "Psychological" => "Psychologique",
        "Sci-Fi" => "Science-fiction",
        "Slice of Life" => "Tranche de vie",
        "Sports" => "Sport",
        "Supernatural" => "Surnaturel",
        other => other,
    }
    .to_string()
}

/// « S1 · E3 » pour une série, « Ép. 3 » pour un animé.
pub fn episode_tag(season: Option<i64>, episode: Option<i64>) -> Option<String> {
    match (season, episode) {
        (Some(s), Some(e)) => Some(format!("S{s} · E{e}")),
        (None, Some(e)) => Some(format!("{} {e}", t("ep.short"))),
        _ => None,
    }
}

/// Texte de l'erreur d'un catalogue.
pub fn error_text(err: &MediaError) -> String {
    let key = match err.code {
        ErrorCode::NoKey => "err.nokey",
        ErrorCode::Auth => "err.auth",
        ErrorCode::Network | ErrorCode::Timeout => "err.network",
        ErrorCode::RateLimit => "err.ratelimit",
        ErrorCode::NotFound => "err.notfound",
        _ => {
            return if err.message.is_empty() { t("err.unknown") } else { err.message.clone() };
        }
    };
    t(key)
}

// ─── Visuels ────────────────────────────────────────────────────────────────────────────────────────────────────

/// Un visuel distant (affiche, fond, photo) : l'image une fois chargée, sinon un dégradé tiré du titre, avec
/// l'icône du type et, si `caption`, le titre en clair (comme `.pcard-blank`).
#[allow(clippy::too_many_arguments)]
pub fn art(ui: &Ui, url: Option<&str>, seed: &str, kind: &str, w: f32, h: f32, radius: f32, caption: bool) -> El {
    let (h1, h2) = seed_hues(seed);
    let base = el()
        .size(w, h)
        .radius(radius)
        .clip_rounded(radius)
        .linear_gradient(150f32.to_radians(), [(0.0, hsl(h1, 0.42, 0.34)), (1.0, hsl(h2, 0.40, 0.18))]);
    match ui.watch.images.get(ui, url) {
        Some(img) => base.child(el().w_full().h_full().pixels_fit(img.pixels(), SurfaceFit::Fill)),
        None => {
            let mut c = col()
                .w_full()
                .h_full()
                .center()
                .gap(8.0)
                .padding(14.0)
                .child(icon(type_icon(kind), (w.min(h) * 0.17).clamp(14.0, 34.0)).color(Color::WHITE.alpha(0.55)));
            if caption && !seed.is_empty() {
                c = c.child(
                    text(seed.to_string())
                        .font(ty::SMALL_MEDIUM)
                        .color(Color::WHITE.alpha(0.7))
                        .align(rune::Align::Center)
                        .max_lines(3)
                        .w_full(),
                );
            }
            base.child(c)
        }
    }
}

/// Pastille de note (étoile jaune).
pub fn rating_badge(rating: f64) -> El {
    row()
        .h(20.0)
        .px(7.0)
        .gap(3.0)
        .items_center()
        .radius(10.0)
        .fill(Color::BLACK.alpha(0.55))
        .child(icon(&icons::STAR, 11.0).color(rgba(255, 214, 10, 1.0)))
        .child(text(format!("{rating:.1}")).font(TextStyle::new(11.5, 650.0).features("'tnum' 1")).color(Color::WHITE))
}

/// Barre de progression au pied d'un visuel.
pub fn progress_bar(p: &Palette, frac: f64, w: f32, h: f32, thick: bool) -> El {
    let bar = if thick { 4.0 } else { 3.0 };
    let frac = frac.clamp(0.0, 1.0) as f32;
    col()
        .absolute(0.0, 0.0)
        .size(w, h)
        .justify_end()
        .child(row().w_full().h(bar).fill(Color::BLACK.alpha(0.5)).child(el().h_full().w(w * frac).fill(p.accent)))
}

/// Bouton rond translucide posé sur un visuel.
fn glass_button(key: String, glyph: &'static Icon, size: f32, label: String) -> El {
    row()
        .key(key)
        .size(size, size)
        .radius(size / 2.0)
        .center()
        .fill(Color::BLACK.alpha(0.5))
        .border(1.0, Color::WHITE.alpha(0.35))
        .cursor(Cursor::Pointer)
        .role("button")
        .label(label)
        .hover(Color::WHITE.alpha(0.12))
        .child(icon(glyph, size * 0.42).color(Color::WHITE))
}

/// Ajoute ou retire un titre de « Ma liste », avec un message.
pub fn toggle_list(s: &mut Ui, item: Item) {
    let entry = item.entry();
    s.spawn(
        move |b| b.media.watchlist_toggle(&entry),
        move |s, res, _| match res {
            Ok(added) => {
                let msg = if added { "toast.listAdded" } else { "toast.listRemoved" };
                s.toast(tv(msg, &[("title", &item.title)]), if added { "bookmark-check" } else { "bookmark" });
                super::load_local(s);
            }
            Err(err) => s.toast(error_text(&err), "triangle-alert"),
        },
    );
}

/// Sous-titre d'une affiche : type (si `show_kind`) et année.
pub fn poster_sub(item: &Item, show_kind: bool) -> String {
    [show_kind.then(|| kind_label(&item.kind)), item.year.map(|y| y.to_string())]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Affiche 2:3 avec note, progression facultative, titre et sous-titre. Un bouton « Ma liste » apparaît au survol
/// (l'original le proposait dans un menu contextuel).
pub fn poster_card(ui: &Ui, p: &Palette, key: String, item: &Item, w: f32, sub: String, progress: Option<f64>) -> El {
    let h = (w * 1.5).round();
    let r = p.r(12.0);
    let mut art_box = el().size(w, h).radius(r).shadow(10.0, 26.0, Color::BLACK.alpha(0.3)).child(art(
        ui,
        item.poster.as_deref(),
        &item.title,
        &item.kind,
        w,
        h,
        r,
        true,
    ));
    if let Some(rating) = item.rating {
        art_box = art_box.child(row().absolute(0.0, 8.0).w(w - 8.0).justify_end().child(rating_badge(rating)));
    }
    if let Some(frac) = progress {
        art_box = art_box.child(
            el().absolute(0.0, 0.0).size(w, h).radius(r).clip_rounded(r).child(progress_bar(p, frac, w, h, false)),
        );
    }
    let listed = ui.watch.in_list(&item.kind, &item.id);
    let label = if listed { t("list.remove") } else { t("list.add") };
    let toggle_item = item.clone();
    let bookmark = glass_button(
        format!("{key}-list"),
        if listed { &icons::BOOKMARK_CHECK } else { &icons::BOOKMARK_PLUS },
        30.0,
        label,
    )
    .on_press(move |s: &mut Ui| toggle_list(s, toggle_item.clone()));
    art_box = art_box.child(row().absolute(8.0, 8.0).group_opacity(0.0, 1.0).child(bookmark));
    let route = item.route();
    col()
        .key(key)
        .w(w)
        .cursor(Cursor::Pointer)
        .hover_group()
        .press_scale(0.98)
        .role("link")
        .label(item.title.clone())
        .child(art_box)
        .child(text(item.title.clone()).font(TextStyle::new(13.0, 620.0)).color(p.text).truncate().w_full().mt(10.0))
        .child(text(sub).font(ty::SMALL).color(p.text2).truncate().w_full().mt(1.0))
        .on_press(move |s: &mut Ui| s.navigate(route.clone()))
}

/// Carte 16:9 de « Continuer à regarder » : relance l'épisode, ou l'oublie (bouton au survol).
pub fn resume_card(ui: &Ui, p: &Palette, key: String, entry: &Progress) -> El {
    let (w, h) = (RESUME_W, (RESUME_W * 9.0 / 16.0).round());
    let r = p.r(12.0);
    let left = (entry.duration - entry.position).max(0.0);
    let sub = [
        episode_tag(entry.season, entry.episode),
        (left > 0.0).then(|| tv("resume.left", &[("t", &format::long(left))])),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let frac = if entry.duration > 0.0 { entry.position / entry.duration } else { 0.0 };
    let src = entry.backdrop.as_deref().or(entry.poster.as_deref());
    let (kind, id, season, episode) = (entry.kind.clone(), entry.id.clone(), entry.season, entry.episode);
    let (rk, rid) = (kind.clone(), id.clone());
    let remove = glass_button(format!("{key}-rm"), &icons::X, 28.0, t("resume.remove")).on_press(move |s: &mut Ui| {
        let (k, i) = (rk.clone(), rid.clone());
        s.spawn(move |b| b.media.watch_remove(&k, &i), |s, _, _| super::load_local(s));
    });
    let art_box = el()
        .size(w, h)
        .radius(r)
        .shadow(10.0, 26.0, Color::BLACK.alpha(0.3))
        .child(art(ui, src, &entry.title, &entry.kind, w, h, r, true))
        .child(el().absolute(0.0, 0.0).size(w, h).radius(r).clip_rounded(r).child(progress_bar(p, frac, w, h, true)))
        .child(row().absolute(0.0, 0.0).size(w, h).center().group_opacity(0.0, 1.0).child(glass_button(
            format!("{key}-play"),
            &icons::PLAY,
            50.0,
            t("common.play"),
        )))
        .child(row().absolute(w - 36.0, 8.0).group_opacity(0.0, 1.0).child(remove));
    let title = if entry.title.is_empty() { "—".to_string() } else { entry.title.clone() };
    col()
        .key(key)
        .w(w)
        .cursor(Cursor::Pointer)
        .hover_group()
        .press_scale(0.985)
        .child(art_box)
        .child(text(title).font(TextStyle::new(13.0, 620.0)).color(p.text).truncate().w_full().mt(10.0))
        .child(text(sub).font(ty::SMALL).color(p.text2).truncate().w_full())
        .on_press(move |s: &mut Ui| super::play::open(s, &kind, &id, season, episode))
}

/// Rangée horizontale : titre, « Tout voir » facultatif, flèches au survol. `width` est la largeur disponible et
/// `arrow_top` la hauteur où centrer les flèches.
#[allow(clippy::too_many_arguments)]
pub fn shelf(
    ui: &Ui,
    p: &Palette,
    key: &str,
    title: &str,
    more: Option<Route>,
    width: f32,
    arrow_top: f32,
    items: Vec<El>,
) -> El {
    let mut head = row()
        .w_full()
        .items_center()
        .justify_between()
        .mb(8.0)
        .child(text(title.to_string()).font(SHELF_TITLE).color(p.text));
    if let Some(route) = more {
        head = head.child(
            row()
                .key(format!("{key}-more"))
                .gap(2.0)
                .items_center()
                .cursor(Cursor::Pointer)
                .role("link")
                .child(text(t("shelf.seeAll")).font(TextStyle::new(12.5, 600.0)).color(p.accent))
                .child(icon(&icons::CHEVRON_RIGHT, 14.0).color(p.accent))
                .on_press(move |s: &mut Ui| s.navigate(route.clone())),
        );
    }
    let scroll_key = format!("{key}-row");
    let offsets = ui.watch.offsets.clone();
    let sk = scroll_key.clone();
    let viewport = el()
        .key(scroll_key.clone())
        .w_full()
        .scroll_x()
        .scroll_edge_fade(6.0)
        .on_scroll_offset(move |pos| {
            offsets.borrow_mut().insert(sk.clone(), [pos.offset[0], pos.viewport[0], pos.max[0]]);
        })
        .child(row().gap(ROW_GAP).pt(8.0).pb(16.0).items_start().children(items));
    let arrow = |dir: f32, glyph: &'static Icon, label: String| {
        let k = scroll_key.clone();
        row()
            .key(format!("{k}-{}", if dir < 0.0 { "prev" } else { "next" }))
            .size(42.0, 42.0)
            .radius(21.0)
            .center()
            .fill(rgba(20, 20, 24, 0.72))
            .border(0.5, Color::WHITE.alpha(0.2))
            .shadow(6.0, 18.0, Color::BLACK.alpha(0.35))
            .cursor(Cursor::Pointer)
            .role("button")
            .label(label)
            .group_opacity(0.0, 1.0)
            .child(icon(glyph, 22.0).color(Color::WHITE))
            .on_press_tx(move |s: &mut Ui, tx| {
                let [off, view, max] = s.watch.offsets.borrow().get(&k).copied().unwrap_or([0.0, 600.0, f32::MAX]);
                let x = (off + dir * view * 0.85).clamp(0.0, max.max(0.0));
                tx.scroll_to(k.clone(), x, 0.0);
            })
    };
    let body = el()
        .w_full()
        .hover_group()
        .child(viewport)
        .child(el().absolute(-14.0, arrow_top - 21.0).child(arrow(-1.0, &icons::CHEVRON_LEFT, t("shelf.prev"))))
        .child(el().absolute(width - 28.0, arrow_top - 21.0).child(arrow(1.0, &icons::CHEVRON_RIGHT, t("shelf.next"))));
    col().key(key.to_string()).w_full().mt(18.0).child(head).child(body)
}

/// Rangée d'affiches.
#[allow(clippy::too_many_arguments)]
pub fn poster_shelf(
    ui: &Ui,
    p: &Palette,
    key: &str,
    title: &str,
    more: Option<Route>,
    width: f32,
    items: &[Item],
    show_kind: bool,
) -> El {
    let cards = items
        .iter()
        .enumerate()
        .map(|(i, it)| poster_card(ui, p, format!("{key}-{i}"), it, POSTER_W, poster_sub(it, show_kind), None))
        .collect();
    shelf(ui, p, key, title, more, width, 8.0 + POSTER_W * 0.75, cards)
}

/// Une pastille : libellé, choisie ou non, action.
pub type Chip = (String, bool, Box<dyn Fn(&mut Ui)>);

/// Pastilles de genres (ou de saisons) ; la pastille choisie est mise en avant.
pub fn chips(p: &Palette, key: &str, items: Vec<Chip>) -> El {
    let mut r = row().key(key.to_string()).w_full().flex_wrap().gap(8.0);
    for (i, (label, on, press)) in items.into_iter().enumerate() {
        let (bg, fg) = if on { (p.accent, p.on_accent) } else { (p.fill, p.text) };
        r = r.child(
            row()
                .key(format!("{key}-{i}"))
                .h(28.0)
                .px(12.0)
                .radius(14.0)
                .items_center()
                .fill(bg)
                .hover(if on { Color::WHITE.alpha(0.1) } else { p.fill_hover })
                .cursor(Cursor::Pointer)
                .press_scale(0.96)
                .role("button")
                .checked(on)
                .child(text(label).font(TextStyle::new(12.5, 500.0)).color(fg))
                .on_press(move |s: &mut Ui| press(s)),
        );
    }
    r
}

/// Rangée grise pendant le chargement.
pub fn skeleton_shelf(p: &Palette, key: &str, count: usize) -> El {
    let mut r = row().gap(ROW_GAP).pt(8.0);
    for _ in 0..count {
        r = r.child(
            col()
                .w(POSTER_W)
                .gap(10.0)
                .child(el().size(POSTER_W, POSTER_W * 1.5).radius(p.r(12.0)).fill(p.fill))
                .child(el().w(POSTER_W * 0.7).h(12.0).radius(p.r(6.0)).fill(p.fill)),
        );
    }
    col()
        .key(key.to_string())
        .w_full()
        .mt(18.0)
        .clip_rounded(0.0)
        .child(el().w(220.0).h(22.0).radius(p.r(8.0)).fill(p.fill).mb(8.0))
        .child(r)
}

/// Erreur d'un catalogue, avec « Réessayer ».
pub fn error_state(p: &Palette, key: &str, err: &MediaError, retry: impl Fn(&mut Ui) + 'static) -> El {
    let again = button(p, key, &t("action.retry"), Some(&icons::REFRESH_CW), true).on_press(retry);
    col().w_full().py(30.0).child(empty_state(
        p,
        &icons::TRIANGLE_ALERT,
        &t("err.title"),
        &error_text(err),
        vec![again],
    ))
}

/// Bouton translucide posé sur une image (bannière, fiche).
pub fn glass_pill(p: &Palette, key: &str, label: &str, glyph: Option<&'static Icon>, primary: bool) -> El {
    let (bg, fg) = if primary { (p.accent, p.on_accent) } else { (Color::WHITE.alpha(0.2), Color::WHITE) };
    let mut b = row()
        .key(key.to_string())
        .h(30.0)
        .px(12.0)
        .gap(6.0)
        .radius(p.r(8.0))
        .items_center()
        .fill(bg)
        .border(if primary { 0.0 } else { 0.5 }, Color::WHITE.alpha(0.25))
        .cursor(Cursor::Pointer)
        .role("button")
        .label(label.to_string())
        .hover(Color::WHITE.alpha(0.12));
    if let Some(g) = glyph {
        b = b.child(icon(g, 14.0).color(fg));
    }
    b.child(text(label.to_string()).font(ty::BODY_MEDIUM).color(fg))
}

/// Les deux voiles sombres d'une bannière (vers la gauche et vers le bas), pour lire le texte sur l'image.
pub fn shade() -> El {
    el().absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .linear_gradient(
            std::f32::consts::FRAC_PI_2,
            [(0.0, Color::BLACK.alpha(0.82)), (0.38, Color::BLACK.alpha(0.5)), (0.75, Color::BLACK.alpha(0.0))],
        )
        .child(
            el().w_full()
                .h_full()
                .linear_gradient(0.0, [(0.0, Color::BLACK.alpha(0.6)), (0.55, Color::BLACK.alpha(0.0))]),
        )
}

/// Bannière d'accueil : jusqu'à cinq titres en rotation (toutes les 8 s), avec lecture, fiche et « Ma liste ».
pub fn hero(ui: &Ui, p: &Palette, slides: &[Item], width: f32, height: f32) -> El {
    let n = slides.len().min(5);
    let index = ui.watch.hero % n.max(1);
    let item = &slides[index];
    let r = p.r(24.0);
    let meta =
        [Some(kind_label(&item.kind)), item.year.map(|y| y.to_string()), item.rating.map(|r| format!("★ {r:.1}"))]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("  ·  ");
    let listed = ui.watch.in_list(&item.kind, &item.id);
    let (play_item, info_route, list_item) = (item.clone(), item.route(), item.clone());
    let actions = row()
        .gap(8.0)
        .mt(6.0)
        .child(
            glass_pill(p, "hero-play", &t("common.play"), Some(&icons::PLAY), true)
                .on_press(move |s: &mut Ui| super::play::quick_play(s, play_item.clone())),
        )
        .child(
            glass_pill(p, "hero-info", &t("hero.moreInfo"), Some(&icons::INFO), false)
                .on_press(move |s: &mut Ui| s.navigate(info_route.clone())),
        )
        .child(
            glass_pill(
                p,
                "hero-list",
                &if listed { t("list.inList") } else { t("list.add") },
                Some(if listed { &icons::BOOKMARK_CHECK } else { &icons::BOOKMARK_PLUS }),
                false,
            )
            .on_press(move |s: &mut Ui| toggle_list(s, list_item.clone())),
        );
    let mut info = col()
        .key(format!("hero-info-{index}"))
        .w((width * 0.62).min(560.0))
        .gap(8.0)
        .child(text(meta).font(CAPTION_WIDE).text_case(TextCase::Uppercase).color(Color::WHITE.alpha(0.8)))
        .child(text(item.title.clone()).font(HERO_TITLE).color(Color::WHITE).max_lines(2));
    if !item.overview.is_empty() {
        info = info.child(
            text(item.overview.clone())
                .font(TextStyle::new(13.0, 400.0).lh(19.0))
                .color(Color::WHITE.alpha(0.82))
                .max_lines(3)
                .max_w(520.0),
        );
    }
    info = info.child(actions);
    let mut dots = row().gap(7.0).items_center();
    if n > 1 {
        for i in 0..n {
            let on = i == index;
            dots = dots.child(
                el().key(format!("hero-dot-{i}"))
                    .size(if on { 22.0 } else { 7.0 }, 7.0)
                    .radius(3.5)
                    .fill(if on { Color::WHITE } else { Color::WHITE.alpha(0.4) })
                    .cursor(Cursor::Pointer)
                    .role("button")
                    .label((i + 1).to_string())
                    .on_press(move |s: &mut Ui| s.watch.hero = i),
            );
        }
    }
    el().key("hero")
        .w_full()
        .h(height)
        .radius(r)
        .clip_rounded(r)
        .fill(rgba(17, 17, 17, 1.0))
        .shadow(24.0, 60.0, Color::BLACK.alpha(0.4))
        .mb(12.0)
        .child(el().key(format!("hero-bg-{index}")).absolute(0.0, 0.0).child(art(
            ui,
            item.backdrop.as_deref(),
            &item.title,
            &item.kind,
            width,
            height,
            0.0,
            false,
        )))
        .child(shade())
        .child(col().absolute(0.0, 0.0).w_full().h_full().justify_end().pl(38.0).pb(34.0).child(info))
        .child(row().absolute(0.0, 0.0).w_full().h_full().items_end().justify_end().padding(24.0).child(dots))
}

/// Carte de saisie de la clé TMDB, affichée tant qu'aucune clé ne fonctionne.
pub fn key_card(ui: &Ui, p: &Palette, k: &Kit) -> El {
    let input = TextInput::new(&ui.watch.key_edit)
        .placeholder(t("key.placeholder"))
        .spellcheck(false)
        .on_edit(|s: &mut Ui| &mut s.watch.key_edit)
        .on_submit(|s: &mut Ui, _| save_key(s))
        .build(k)
        .key("key-input")
        .grow(1.0)
        .min_w(0.0);
    let save = button(p, "key-save", &t("key.test"), None, true).on_press(save_key);
    let (status, color) = match ui.watch.key_status {
        KeyStatus::Idle => (String::new(), p.text2),
        KeyStatus::Testing => (t("key.testing"), p.text2),
        KeyStatus::Invalid => (t("key.invalid"), rgba(255, 107, 97, 1.0)),
        KeyStatus::Network => (t("err.network"), rgba(255, 107, 97, 1.0)),
    };
    let link = |key: &str, label: String| {
        text(label)
            .key(key.to_string())
            .font(TextStyle::new(12.5, 560.0))
            .color(p.accent)
            .cursor(Cursor::Pointer)
            .role("link")
    };
    let links = row()
        .gap(18.0)
        .mt(6.0)
        .child(
            link("key-get", t("key.get"))
                .on_press(|_: &mut Ui| crate::os::open_url("https://www.themoviedb.org/settings/api")),
        )
        .child(link("key-anime", t("key.anime")).on_press(|s: &mut Ui| s.navigate(Route::Watch(WatchRoute::Anime))))
        .child(link("key-settings", t("key.settings")).on_press(open_streaming_settings));
    row()
        .key("key-card")
        .w_full()
        .max_w(760.0)
        .self_center()
        .mt(24.0)
        .gap(20.0)
        .items_start()
        .padding(24.0)
        .radius(p.r(22.0))
        .linear_gradient(135f32.to_radians(), [(0.0, p.accent.alpha(0.2)), (1.0, p.accent.alpha(0.05))])
        .border(0.5, p.accent.alpha(0.35))
        .child(
            row()
                .size(54.0, 54.0)
                .radius(p.r(16.0))
                .center()
                .fill(p.accent)
                .shadow(10.0, 26.0, p.accent.alpha(0.4))
                .child(icon(&icons::KEY_ROUND, 26.0).color(p.on_accent)),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(
                    text(t("key.title")).font(TextStyle::new(19.0, 720.0).tracking(-0.4)).color(p.text).mt(2.0).mb(4.0),
                )
                .child(text(t("key.text")).font(TextStyle::new(13.0, 400.0).lh(19.0)).color(p.text2).wrap().mb(14.0))
                .child(row().w_full().gap(8.0).items_center().child(input).child(save))
                .child(text(status).font(TextStyle::new(12.5, 400.0)).color(color).h(18.0).mt(8.0))
                .child(links),
        )
}

/// Ouvre les réglages sur l'onglet Streaming (clé TMDB).
pub fn open_streaming_settings(s: &mut Ui) {
    s.field_edit = rune::TextEdit::new(s.settings.media.tmdb_key.clone());
    s.sheet = Some(Sheet::Settings(SettingsTab::Streaming));
}

/// Teste la clé saisie (sans l'enregistrer), puis l'enregistre si TMDB l'accepte.
fn save_key(s: &mut Ui) {
    let key = s.watch.key_edit.text.trim().to_string();
    if key.is_empty() || s.watch.key_status == KeyStatus::Testing {
        return;
    }
    s.watch.key_status = KeyStatus::Testing;
    let tested = key.clone();
    s.spawn(
        move |b| b.media.test_key(&tested),
        move |s, res, _| match res {
            Ok(_) => {
                s.watch.key_status = KeyStatus::Idle;
                s.watch.key_edit = rune::TextEdit::new("");
                s.set(json!({ "media": { "tmdbKey": key } }));
                s.toast(t("key.saved"), "circle-check");
                s.watch.homes.retain(|k, _| *k == "anime");
                if let Route::Watch(w) = s.route.clone() {
                    super::on_open(s, &w);
                }
            }
            Err(err) => {
                s.watch.key_status = if err.code == ErrorCode::Auth { KeyStatus::Invalid } else { KeyStatus::Network };
            }
        },
    );
}

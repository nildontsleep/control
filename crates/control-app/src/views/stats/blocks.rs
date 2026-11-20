//! Briques de la page des statistiques : chiffres clés, cartes, classements, affiches.

use rune::{Color, Cursor, El, Icon, col, el, icon, row, text};

use crate::state::Ui;
use crate::theme::Palette;
use crate::ui::common::ty;
use crate::views::stats::style;

/// Chiffre clé : icône, valeur, libellé, précision.
pub fn kpi(p: &Palette, label: String, value: String, sub: Option<String>, glyph: &'static Icon) -> El {
    let mut c = col()
        .min_w(0.0)
        .gap(2.0)
        .pt(16.0)
        .px(18.0)
        .pb(15.0)
        .radius(p.r(16.0))
        .fill(p.group)
        .border(0.5, p.panel_border.alpha(p.panel_border.a * 0.7))
        .child(
            el().size(30.0, 30.0)
                .mb(8.0)
                .radius(p.r(9.0))
                .fill(p.accent.alpha(0.14))
                .center()
                .child(icon(glyph, 17.0).color(p.accent)),
        )
        .child(text(value).font(style::KPI).color(p.text).truncate())
        .child(text(label).font(style::KPI_LABEL).color(p.text2).truncate());
    if let Some(s) = sub {
        c = c.child(text(s).font(style::SUB).color(p.text3).truncate());
    }
    c
}

/// Rangée de chiffres clés en `cols` colonnes égales (plusieurs lignes si besoin).
pub fn tiles(tiles: Vec<El>, cols: usize) -> El {
    grid(tiles, cols, 12.0)
}

/// Grille de `cols` colonnes égales, remplie ligne par ligne.
pub fn grid(items: Vec<El>, cols: usize, gap: f32) -> El {
    grid_gap(items, cols, gap, gap)
}

/// [`grid`] avec des écarts différents entre colonnes et entre lignes.
pub fn grid_gap(items: Vec<El>, cols: usize, col_gap: f32, row_gap: f32) -> El {
    let cols = cols.max(1);
    let mut out = col().w_full().gap(row_gap);
    let mut items = items.into_iter().peekable();
    while items.peek().is_some() {
        let mut r = row().w_full().gap(col_gap).items_stretch();
        for _ in 0..cols {
            match items.next() {
                Some(e) => r = r.child(col().grow(1.0).min_w(0.0).child(e.w_full().h_full())),
                None => r = r.child(el().grow(1.0).min_w(0.0)),
            }
        }
        out = out.child(r);
    }
    out
}

/// Carte avec un petit titre.
pub fn card(p: &Palette, title: Option<String>, body: Vec<El>) -> El {
    let mut c = col()
        .w_full()
        .min_w(0.0)
        .pt(16.0)
        .px(20.0)
        .pb(18.0)
        .radius(p.r(18.0))
        .fill(p.group)
        .border(0.5, p.panel_border.alpha(p.panel_border.a * 0.7));
    if let Some(t) = title {
        c = c.child(text(t).font(style::CARD_TITLE).color(p.text2).mb(14.0));
    }
    c.children(body)
}

/// Titre d'un bloc (« Musique », « Films, séries et animés »).
pub fn block_title(p: &Palette, label: String) -> El {
    text(label).font(ty::TITLE).color(p.text).mt(8.0)
}

/// Petite note sous un graphique.
pub fn note(p: &Palette, s: String) -> El {
    text(s).font(ty::SMALL).color(p.text3).mt(10.0)
}

/// Action d'une ligne de classement.
pub type OnPress = Box<dyn Fn(&mut Ui)>;

/// Une ligne de classement.
pub struct Rank {
    pub key: String,
    pub art: El,
    pub round: bool,
    pub title: String,
    pub sub: String,
    pub value: String,
    /// Part du premier (0 à 1), pour la barre de proportion.
    pub share: f32,
    pub on_press: Option<OnPress>,
}

/// Liste classée, chaque ligne avec sa barre de proportion.
pub fn rank_list(p: &Palette, rows: Vec<Rank>) -> El {
    let mut list = col().w_full().gap(2.0).role("list");
    for (i, r) in rows.into_iter().enumerate() {
        let art = el()
            .size(40.0, 40.0)
            .radius(if r.round { 20.0 } else { p.r(8.0) })
            .clip_rounded(if r.round { 20.0 } else { p.r(8.0) })
            .center()
            .child(r.art);
        let mut text_col =
            col().grow(1.0).min_w(0.0).child(text(r.title).font(ty::BODY_MEDIUM).color(p.text).truncate());
        if !r.sub.is_empty() {
            text_col = text_col.child(text(r.sub).font(ty::SMALL).color(p.text3).truncate());
        }
        let bar = row()
            .pin(Some(8.0), None, Some(8.0), Some(2.0))
            .h(3.0)
            .inert()
            .child(el().w_frac(r.share.clamp(0.04, 1.0)).h_full().radius(p.r(3.0)).fill(p.accent.alpha(0.55)));
        let mut line = row()
            .key(r.key)
            .w_full()
            .items_center()
            .gap(10.0)
            .pt(6.0)
            .px(8.0)
            .pb(9.0)
            .radius(p.r(10.0))
            .child(text((i + 1).to_string()).font(style::RANK_N).color(p.text3).w(20.0).align(rune::Align::Center))
            .child(art)
            .child(text_col)
            .child(text(r.value).font(ty::NUMBERS).color(p.text2))
            .child(bar);
        if let Some(f) = r.on_press {
            line = line.cursor(Cursor::Pointer).hover(p.fill).role("link").on_press(move |s: &mut Ui| f(s));
        }
        list = list.child(line);
    }
    list
}

/// Pastille d'un genre (plus pâle à chaque rang).
pub fn genre_dot(p: &Palette, rank: usize, step: f32) -> El {
    el().size(14.0, 14.0).radius(7.0).fill(p.accent).opacity((1.0 - rank as f32 * step).max(0.1))
}

/// Affiche d'un film ou d'une série : visuel 2:3, rang, titre, durée.
pub fn poster(
    p: &Palette,
    key: String,
    art: Option<El>,
    rank: usize,
    title: String,
    sub: String,
    on_press: impl Fn(&mut Ui) + 'static,
) -> El {
    let r = p.r(10.0);
    let mut visual = el()
        .w_full()
        .aspect_ratio(2.0 / 3.0)
        .radius(r)
        .clip_rounded(r)
        .fill(p.fill)
        .shadow(8.0, 20.0, Color::BLACK.alpha(0.25))
        .border(0.5, p.panel_border);
    visual = match art {
        Some(a) => visual.child(a),
        None => visual.center().child(icon(&crate::icons::FILM, 26.0).color(p.text3)),
    };
    visual = visual.child(
        row()
            .absolute(6.0, 6.0)
            .min_w(22.0)
            .h(19.0)
            .px(6.0)
            .radius(10.0)
            .center()
            .fill(Color::BLACK.alpha(0.55))
            .child(text(rank.to_string()).font(style::POSTER_RANK).color(Color::WHITE)),
    );
    col()
        .key(key)
        .min_w(0.0)
        .gap(3.0)
        .cursor(Cursor::Pointer)
        .role("button")
        .label(title.clone())
        .press_scale(0.98)
        .on_press(on_press)
        .child(visual)
        .child(text(title).font(style::POSTER_TITLE).color(p.text).truncate().mt(5.0))
        .child(text(sub).font(style::SUB).color(p.text3).truncate())
}

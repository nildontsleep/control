//! Grilles de cartes (albums, artistes, vidéos) virtualisées par rangées.

use std::borrow::Cow;
use std::rc::Rc;

use rune::{Cursor, Cx, El, GrowthSpec, ScrollSpec, col, el, icon, row, text};

use crate::icons;
use crate::state::Ui;
use crate::theme::Palette;
use crate::ui::common::ty;

pub const GAP: f32 = 22.0;
pub const SIDE: f32 = 32.0;

/// Nombre de colonnes pour des cartes d'au moins `min_card` points dans `width`.
pub fn columns(width: f32, min_card: f32) -> usize {
    let inner = (width - 2.0 * SIDE).max(min_card);
    (((inner + GAP) / (min_card + GAP)).floor() as usize).max(1)
}

/// Largeur d'une carte quand `cols` colonnes remplissent `width`.
pub fn card_width(width: f32, cols: usize) -> f32 {
    let inner = (width - 2.0 * SIDE).max(100.0);
    ((inner - GAP * (cols as f32 - 1.0)) / cols as f32).floor()
}

/// Une grille défilante : `head` en ligne 0, puis des rangées de `cols` cartes construites par `card(ui, index, w)`.
#[allow(clippy::too_many_arguments)]
pub fn grid(
    cx: &mut Cx<Ui>,
    key: &'static str,
    count: usize,
    min_card: f32,
    row_extra: f32,
    head: impl Fn(&Ui, &Palette) -> El + 'static,
    card: impl Fn(&Ui, &Palette, usize, f32) -> El + 'static,
    p: &Palette,
) -> El {
    let width = cx.layout_size(key).map(|[w, _]| w).unwrap_or(1000.0);
    let cols = columns(width, min_card);
    let w = card_width(width, cols);
    let rows = count.div_ceil(cols);
    let palette = *p;
    let card = Rc::new(card);
    let body = col()
        .key(format!("{key}-rows"))
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows(
            rows + 1,
            w + row_extra,
            move |_s: &Ui, i| if i == 0 { Cow::Borrowed("head") } else { Cow::Owned(format!("r{i}-{cols}")) },
            move |s: &Ui, i| {
                if i == 0 {
                    return head(s, &palette);
                }
                let start = (i - 1) * cols;
                let end = (start + cols).min(count);
                let mut r = row().w_full().gap(GAP).pb(GAP).items_start();
                for idx in start..end {
                    r = r.child(card(s, &palette, idx, w));
                }
                r
            },
        );
    col().key(key).w_full().h_full().min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).scroll_edge_fade(8.0).child(body.px(SIDE).pb(120.0))
}

#[allow(clippy::too_many_arguments)]
/// Carte : visuel, titre, sous-titre ; un bouton lecture apparaît au survol.
pub fn card(
    p: &Palette,
    key: String,
    art: El,
    w: f32,
    title: String,
    sub: String,
    round: bool,
    center: bool,
    on_play: impl Fn(&mut Ui) + 'static,
) -> El {
    let play = row()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .items_end()
        .justify_end()
        .padding(10.0)
        .group_opacity(0.0, 1.0)
        .child(
            row()
                .key(format!("{key}-play"))
                .size(40.0, 40.0)
                .radius(20.0)
                .center()
                .fill(p.accent)
                .cursor(Cursor::Pointer)
                .shadow(4.0, 14.0, rune::Color::BLACK.alpha(0.35))
                .role("button")
                .label(crate::i18n::t("menu.play"))
                .child(icon(&icons::PLAY, 18.0).color(p.on_accent))
                .on_press(on_play),
        );
    let mut title_el = text(title).font(ty::BODY_MEDIUM).color(p.text).truncate();
    let mut sub_el = text(sub).font(ty::SMALL).color(p.text2).truncate();
    if center {
        title_el = title_el.align(rune::Align::Center);
        sub_el = sub_el.align(rune::Align::Center);
    }
    let art_box = el().w(w).child(art).child(play);
    let _ = round;
    col()
        .key(key)
        .w(w)
        .gap(4.0)
        .cursor(Cursor::Pointer)
        .hover_group()
        .press_scale(0.98)
        .child(art_box.mb(6.0))
        .child(title_el.w_full())
        .child(sub_el.w_full())
}

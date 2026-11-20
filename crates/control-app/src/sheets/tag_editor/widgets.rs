//! Briques des feuilles d'édition (tags, playlists intelligentes, personnalisation) : feuille avec un pied fixe,
//! liste déroulante, petit bouton.

use rune::{Color, Cursor, El, Tx, col, icon, row, text};
use rune_kit::{Button, ControlSize, Kit, Select, SelectAppearance};

use crate::i18n::t;
use crate::icons;
use crate::state::Ui;
use crate::theme::Palette;
use crate::ui::common::ty;

/// Une liste déroulante : options, choix courant, largeur et texte quand rien n'est choisi.
pub(crate) struct Popup<'a> {
    pub key: &'a str,
    pub options: Vec<String>,
    pub current: Option<usize>,
    pub width: f32,
    pub placeholder: String,
}

/// Liste déroulante dont l'état ouvert vit dans `slot` ; `choose` reçoit l'indice de l'option.
pub(crate) fn popup_select(
    k: &Kit,
    layer: &mut Vec<El>,
    open: bool,
    spec: Popup<'_>,
    slot: fn(&mut Ui) -> &mut Option<String>,
    choose: impl Fn(&mut Ui, usize) + Clone + 'static,
) -> El {
    let key = spec.key.to_string();
    let opts = spec.options.clone();
    Select::new(key.clone(), &spec.options)
        .current(spec.current.and_then(|i| spec.options.get(i)).map(String::as_str))
        .placeholder(spec.placeholder)
        .label(key.clone())
        .appearance(SelectAppearance::Field)
        .width(spec.width)
        .open(open)
        .on_toggle(move |s: &mut Ui| {
            let slot = slot(s);
            *slot = if slot.as_deref() == Some(key.as_str()) { None } else { Some(key.clone()) };
        })
        .on_close(move |s: &mut Ui| *slot(s) = None)
        .on_choose(move |s: &mut Ui, v: &str, _: &mut Tx| {
            *slot(s) = None;
            if let Some(i) = opts.iter().position(|o| o == v) {
                choose(s, i);
            }
        })
        .build(k, layer)
}

/// Feuille modale avec un pied fixe (le corps défile). `close` sert au bouton, à Échap et au clic à côté.
pub(crate) fn card(
    p: &Palette,
    key: &str,
    title: String,
    (width, max_h): (f32, f32),
    body: El,
    footer: El,
    close: fn(&mut Ui),
) -> El {
    let close_btn = row()
        .key(format!("{key}-close"))
        .size(28.0, 28.0)
        .radius(14.0)
        .center()
        .cursor(Cursor::Pointer)
        .hover(p.fill_hover)
        .role("button")
        .label(t("common.close"))
        .child(icon(&icons::X, 16.0).color(p.text2))
        .on_press(close);
    let card = col()
        .key(key.to_string())
        .w(width)
        .max_h(max_h)
        .shrink()
        .min_h(0.0)
        .radius(p.r(16.0))
        .fill(p.panel_solid)
        .border(1.0, p.panel_border)
        .shadow(30.0, 80.0, Color::BLACK.alpha(if p.dark { 0.5 } else { 0.22 }))
        .clip_rounded(p.r(16.0))
        .role("dialog")
        .label(title.clone())
        .modal(true, close)
        .child(
            row()
                .w_full()
                .h(56.0)
                .px(22.0)
                .items_center()
                .justify_between()
                .child(text(title).font(ty::TITLE).color(p.text))
                .child(close_btn),
        )
        .child(
            col()
                .key(format!("{key}-body"))
                .w_full()
                .shrink()
                .min_h(0.0)
                .scroll(rune::ScrollSpec { stick_bottom: false })
                .scroll_edge_fade(10.0)
                .child(col().w_full().px(22.0).pb(8.0).child(body)),
        )
        .child(row().w_full().px(22.0).py(16.0).gap(8.0).items_center().child(footer));
    // Une colonne : la feuille rétrécit (et son corps défile) quand la fenêtre est basse.
    col()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .padding(40.0)
        .justify_center()
        .items_center()
        .fill(Color::BLACK.alpha(0.34))
        .on_background_press(close)
        .child(card)
}

pub(crate) fn small_button(label: String, glyph: Option<&'static rune::Icon>) -> Button {
    let b = Button::new(label).control_size(ControlSize::Small);
    match glyph {
        Some(g) => b.icon(g),
        None => b,
    }
}

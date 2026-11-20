//! Éléments partagés : pochettes, boutons, en-têtes de page, kit de composants.

use rune::{Color, Cursor, El, Icon, SurfaceFit, col, el, icon, row, text};
use rune_kit::{Kit, PaletteSeed};

use crate::state::Ui;
use crate::theme::{Palette, rgba};

/// Styles de texte (Inter, tailles de l'original).
pub mod ty {
    use rune::TextStyle;

    pub const LARGE_TITLE: TextStyle = TextStyle::new(30.0, 700.0);
    pub const TITLE: TextStyle = TextStyle::new(20.0, 650.0);
    pub const HEADLINE: TextStyle = TextStyle::new(15.0, 600.0);
    pub const BODY: TextStyle = TextStyle::new(13.0, 400.0);
    pub const BODY_MEDIUM: TextStyle = TextStyle::new(13.0, 550.0);
    pub const SMALL: TextStyle = TextStyle::new(12.0, 400.0);
    pub const SMALL_MEDIUM: TextStyle = TextStyle::new(12.0, 550.0);
    pub const CAPTION: TextStyle = TextStyle::new(11.0, 600.0);
    pub const NUMBERS: TextStyle = TextStyle::new(12.0, 400.0).features("'tnum' 1");
}

/// Composants rune-kit aux couleurs de l'application.
pub fn kit(p: &Palette) -> Kit {
    let bg = if p.dark { rgba(30, 30, 34, 1.0) } else { rgba(246, 246, 249, 1.0) };
    let fg = if p.dark { rgba(255, 255, 255, 0.92) } else { rgba(0, 0, 0, 0.86) };
    Kit::from_palette(PaletteSeed { bg, fg, accent: p.accent })
}

/// Couleur HSL (teinte en degrés, saturation et luminosité de 0 à 1).
pub fn hsl(h: f32, s: f32, l: f32) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to = |v: f32| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    rgba(to(r), to(g), to(b), 1.0)
}

fn fnv1a(s: &str) -> u32 {
    let mut hash: u32 = 2_166_136_261;
    for unit in s.encode_utf16() {
        hash ^= unit as u32;
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
}

/// Les deux teintes du dégradé de repli d'une pochette absente (même calcul que l'original).
pub fn seed_hues(seed: &str) -> (f32, f32) {
    let seed = if seed.is_empty() { "?" } else { seed };
    let hue = fnv1a(seed) % 360;
    let hue2 = (hue + 38 + fnv1a(&format!("{seed}x")) % 40) % 360;
    (hue as f32, hue2 as f32)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CoverKind {
    Music,
    Video,
    Artist,
    Playlist,
}

/// Une pochette carrée : l'image si elle existe, sinon un dégradé déterministe et une icône.
pub fn cover(ui: &Ui, hash: Option<&str>, seed: &str, size: f32, radius: f32, kind: CoverKind) -> El {
    let small = size <= 120.0;
    let (h1, h2) = seed_hues(seed);
    let glyph: &'static Icon = match kind {
        CoverKind::Music => &crate::icons::MUSIC_2,
        CoverKind::Video => &crate::icons::FILM,
        CoverKind::Artist => &crate::icons::USER,
        CoverKind::Playlist => &crate::icons::LIST_MUSIC,
    };
    let base = el()
        .size(size, size)
        .radius(radius)
        .clip_rounded(radius)
        .linear_gradient(150f32.to_radians(), [(0.0, hsl(h1, 0.64, 0.60)), (1.0, hsl(h2, 0.58, 0.32))]);
    match ui.cover(hash, small) {
        Some(img) => base.child(el().w_full().h_full().pixels_fit(img.pixels(), SurfaceFit::Fill)),
        None => base.center().child(icon(glyph, (size * 0.32).clamp(12.0, 64.0)).color(Color::WHITE.alpha(0.85))),
    }
}

/// Bouton icône rond et discret (barre de lecture, en-têtes).
pub fn icon_button(p: &Palette, key: &str, glyph: &'static Icon, size: f32, active: bool) -> El {
    let color = if active { p.accent } else { p.text };
    row()
        .key(key.to_string())
        .size(size + 12.0, size + 12.0)
        .radius((size + 12.0) / 2.0)
        .center()
        .cursor(Cursor::Pointer)
        .hover(Color::WHITE.alpha(0.08))
        .child(icon(glyph, size).color(color))
}

/// Bouton plein (accent) ou discret, avec une icône facultative.
pub fn button(p: &Palette, key: &str, label: &str, glyph: Option<&'static Icon>, primary: bool) -> El {
    let (bg, fg) = if primary { (p.accent, p.on_accent) } else { (p.fill_strong, p.text) };
    let mut b = row()
        .key(key.to_string())
        .h(30.0)
        .px(12.0)
        .gap(6.0)
        .radius(p.r(8.0))
        .items_center()
        .fill(bg)
        .cursor(Cursor::Pointer)
        .role("button")
        .label(label.to_string())
        .hover(Color::WHITE.alpha(0.12));
    if let Some(g) = glyph {
        b = b.child(icon(g, 14.0).color(fg));
    }
    b.child(text(label.to_string()).font(ty::BODY_MEDIUM).color(fg))
}

/// En-tête d'une page : grand titre, sous-titre et actions à droite.
pub fn page_head(p: &Palette, title: &str, subtitle: Option<String>, actions: Option<El>) -> El {
    let mut left = col().gap(4.0).shrink().min_w(0.0).child(
        text(title.to_string()).font(ty::LARGE_TITLE).color(p.text).truncate(),
    );
    if let Some(sub) = subtitle {
        left = left.child(text(sub).font(ty::BODY).color(p.text2));
    }
    let mut head = row().w_full().items_end().justify_between().gap(16.0).pb(18.0).child(left);
    if let Some(a) = actions {
        head = head.child(a);
    }
    head
}

/// Petit titre de section (« Albums », « Titres populaires »…).
pub fn section_title(p: &Palette, label: &str) -> El {
    text(label.to_string()).font(ty::TITLE).color(p.text).mt(10.0).mb(12.0)
}

/// Message d'une vue vide, centré.
pub fn empty_state(p: &Palette, glyph: &'static Icon, title: &str, body: &str, actions: Vec<El>) -> El {
    col()
        .w_full()
        .grow(1.0)
        .min_h(320.0)
        .center()
        .gap(10.0)
        .child(icon(glyph, 40.0).color(p.text3))
        .child(text(title.to_string()).font(ty::TITLE).color(p.text))
        .child(text(body.to_string()).font(ty::BODY).color(p.text2).max_w(420.0).wrap().align(rune::Align::Center))
        .child(row().gap(8.0).mt(8.0).children(actions))
}


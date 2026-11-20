//! La fenêtre : fond, barre latérale, vue courante, barre de lecture, et ce qui s'ouvre par-dessus.

pub mod common;
pub mod grid;
pub mod menu;
pub mod playerbar;
pub mod sidebar;
pub mod tracklist;

use rune::{Cursor, Cx, El, Key, KeyEvent, Tx, col, el, icon, row, text};

use crate::icons;
use crate::state::{Route, Ui};
use crate::theme::Palette;
use crate::ui::common::{kit, ty};
use crate::views;

/// Barre du haut de la vue : précédent et suivant.
fn toolbar(ui: &Ui, p: &Palette) -> El {
    let nav_btn = |key: &str, glyph: &'static rune::Icon, enabled: bool| {
        row()
            .key(key.to_string())
            .size(28.0, 28.0)
            .radius(p.r(7.0))
            .center()
            .cursor(if enabled { Cursor::Pointer } else { Cursor::Default })
            .hover(if enabled { p.fill } else { rune::Color::TRANSPARENT })
            .opacity(if enabled { 1.0 } else { 0.35 })
            .child(icon(glyph, 18.0).color(p.text2))
    };
    row()
        .w_full()
        .h(46.0)
        .px(24.0)
        .gap(4.0)
        .items_center()
        .child(nav_btn("tb-back", &icons::CHEVRON_LEFT, !ui.back.is_empty()).on_press(|s: &mut Ui| s.go_back()))
        .child(nav_btn("tb-forward", &icons::CHEVRON_RIGHT, !ui.forward.is_empty()).on_press(|s: &mut Ui| s.go_forward()))
}

fn current_view(ui: &Ui, p: &Palette, cx: &mut Cx<Ui>) -> El {
    match &ui.route {
        Route::Songs => views::songs::view(ui, p),
        Route::Albums => views::albums::view(ui, p, cx),
        Route::Album(k) => views::albums::album_view(ui, p, k),
        Route::Artists => views::artists::view(ui, p, cx),
        Route::Artist(k) => views::artists::artist_view(ui, p, k, cx),
        Route::Videos => views::videos::view(ui, p, cx),
        Route::Playlist(id) => views::playlist::view(ui, p, *id),
        Route::Search => views::search::view(ui, p, cx),
        Route::Downloads => views::downloads::view(ui, p, cx),
        Route::Stats => views::stats::view(ui, p, cx),
        Route::Watch(w) => views::watch::view(ui, p, cx, w),
    }
}

/// Fond : couleur du thème, teintée par la pochette en cours (réglage « Fond coloré »).
fn backdrop(ui: &Ui, p: &Palette) -> El {
    let mut bg = el().absolute(0.0, 0.0).w_full().h_full().fill(p.bg);
    if ui.settings.ambient
        && let Some(tr) = &ui.player.track
        && let Some(img) = ui.cover(tr.cover.as_deref(), true)
    {
        bg = bg.child(
            el().absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .scale(1.15)
                .pixels_fit(img.pixels(), rune::SurfaceFit::Fill)
                .blur(60.0)
                .opacity(if p.dark { 0.5 } else { 0.42 }),
        );
        bg = bg.child(el().absolute(0.0, 0.0).w_full().h_full().fill(p.veil));
    }
    bg
}

fn toasts(ui: &Ui, p: &Palette) -> El {
    let mut c = col().gap(8.0).items_center();
    for t in &ui.toasts {
        let glyph = icons::by_name(t.icon).unwrap_or(&icons::CHECK);
        c = c.child(
            row()
                .key(format!("toast-{}", t.id))
                .h(38.0)
                .px(14.0)
                .gap(8.0)
                .items_center()
                .radius(19.0)
                .fill(p.menu)
                .backdrop_blur(20.0)
                .border(1.0, p.panel_border)
                .shadow(8.0, 24.0, rune::Color::BLACK.alpha(0.3))
                .child(icon(glyph, 15.0).color(p.accent))
                .child(text(t.text.clone()).font(ty::BODY_MEDIUM).color(p.text)),
        );
    }
    row().absolute(0.0, 0.0).w_full().pt(18.0).justify_center().child(c).inert()
}

pub fn view(ui: &Ui, cx: &mut Cx<Ui>) -> El {
    let p = Palette::from_settings(&ui.settings, cx.system_dark());
    let k = kit(&p);
    if let Some(r) = cx.window_layout("pl-more") {
        ui.more_rect.set(r);
    }
    let mut layer: Vec<El> = Vec::new();

    let main = col()
        .key("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .child(toolbar(ui, &p))
        .child(col().key("view").w_full().grow(1.0).min_h(0.0).child(current_view(ui, &p, cx)));

    let body = row().w_full().h_full().child(sidebar::sidebar(ui, &p, &k)).child(main);

    let base = el()
        .key("window")
        .w_full()
        .h_full()
        .child(backdrop(ui, &p))
        .child(body.absolute(0.0, 0.0))
        .child(
            col()
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .justify_end()
                .pl(sidebar::WIDTH)
                .child(playerbar::player_bar(ui, &p, cx))
        )
        .child(toasts(ui, &p));

    crate::sheets::sheet(ui, &p, &k, &mut layer);
    if let Some(m) = &ui.menu {
        layer.push(menu::context_menu(ui, &p, &k, m));
    }
    rune_kit::root(base, layer)
}

/// Raccourcis clavier de la fenêtre (quand aucun champ ne les prend).
pub fn on_key(s: &mut Ui, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if tx.focused().is_some() && !matches!(ev.key, Key::Escape) {
        return false;
    }
    let (cat, b) = (s.cat.clone(), s.backend.clone());
    match &ev.key {
        Key::Text(t) if t == " " => s.player.toggle(),
        Key::Escape => {
            if s.menu.is_some() {
                s.menu = None;
            } else if s.sheet.is_some() {
                s.sheet = None;
            } else if s.now_playing {
                s.now_playing = false;
            } else {
                return false;
            }
        }
        Key::Right if ev.mods.cmd => s.player.next(&cat, &b),
        Key::Left if ev.mods.cmd => s.player.prev(&cat, &b),
        Key::Right => {
            let pos = s.player.position();
            s.player.seek(pos + 10.0);
        }
        Key::Left => {
            let pos = s.player.position();
            s.player.seek(pos - 10.0);
        }
        Key::Text(t) if t == "a" || t == "A" => {
            let on = !s.settings.automix;
            s.set(serde_json::json!({ "automix": on }));
        }
        _ => return false,
    }
    true
}

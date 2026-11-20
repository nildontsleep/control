//! Mode immersif : plein écran sans commandes, rien que la pochette et les paroles qui défilent (ligne courante en
//! grand, mots qui s'allument au rythme du chant, les autres en retrait), sur un fond aux couleurs de la pochette.

use rune::{Color, Cursor, Cx, El, SurfaceFit, TextStyle, col, el, icon, row, text};
use serde_json::json;

use crate::i18n::t;
use crate::icons;
use crate::state::Ui;
use crate::theme::Palette;
use crate::ui::common::{CoverKind, cover, ty};

const BIG: TextStyle = TextStyle::new(38.0, 750.0);
const NEAR: TextStyle = TextStyle::new(28.0, 700.0);

/// Une ligne de paroles ; la ligne courante allume ses mots un à un.
fn lyric_line(ui: &Ui, i: usize, active: Option<usize>, position: f64) -> El {
    let Some(lrc) = ui.lyrics.lrc() else { return el() };
    let line = &lrc.lines[i];
    let distance = active.map(|a| (i as i64 - a as i64).unsigned_abs()).unwrap_or(2);
    let key = format!("im-{i}");
    if line.is_gap() {
        let on = active == Some(i);
        return row()
            .key(key)
            .gap(10.0)
            .py(6.0)
            .children((0..3).map(|_| el().size(12.0, 12.0).radius(6.0).fill(Color::WHITE.alpha(if on { 0.9 } else { 0.35 }))));
    }
    if active == Some(i) {
        let lit = lrc.lit_words(i, position);
        let (parts, spaced) = line.parts();
        let mut r = row().key(key).flex_wrap().gap(if spaced { 10.0 } else { 0.0 });
        for (n, w) in parts.iter().enumerate() {
            r = r.child(text(w.to_string()).font(BIG).color(Color::WHITE.alpha(if n < lit { 1.0 } else { 0.45 })));
        }
        return r.py(4.0);
    }
    let alpha = match distance {
        1 => 0.5,
        2 => 0.32,
        _ => 0.2,
    };
    let mut el_ = text(line.text.clone()).key(key).font(NEAR).color(Color::WHITE.alpha(alpha)).wrap().py(4.0);
    if distance > 1 {
        el_ = el_.blur(1.5 * (distance.min(4) as f32 - 1.0));
    }
    if let Some(t0) = line.t {
        el_ = el_.cursor(Cursor::Pointer).on_press(move |s: &mut Ui| s.player.seek(t0));
    }
    el_
}

pub fn view(ui: &Ui, p: &Palette, cx: &mut Cx<Ui>) -> El {
    let window = cx.layout_size("window").unwrap_or([1280.0, 820.0]);
    let track = ui.player.track.as_ref();
    let show_lyrics = ui.settings.immersive_lyrics && ui.lyrics.lrc().is_some();
    let art = (window[1] * 0.55).clamp(220.0, 560.0).min(window[0] * if show_lyrics { 0.38 } else { 0.6 });

    let mut bg = el().absolute(0.0, 0.0).w_full().h_full().fill(if p.oled { Color::BLACK } else { p.bg });
    if let Some(img) = track.and_then(|tr| ui.cover(tr.cover.as_deref(), true)) {
        bg = bg
            .child(el().absolute(0.0, 0.0).w_full().h_full().scale(1.3).pixels_fit(img.pixels(), SurfaceFit::Fill).blur(90.0).opacity(0.85))
            .child(el().absolute(0.0, 0.0).w_full().h_full().fill(Color::BLACK.alpha(0.35)));
    }

    let cover_el = match track {
        Some(tr) => {
            let seed = if tr.album.is_empty() { tr.title.clone() } else { tr.album.clone() };
            cover(ui, tr.cover.as_deref(), &seed, art, p.r(20.0), CoverKind::Music)
                .shadow(30.0, 80.0, Color::BLACK.alpha(0.5))
                .cursor(Cursor::Pointer)
                .on_press(|s: &mut Ui| s.player.toggle())
        }
        None => el().size(art, art),
    };
    let mut left = col().gap(18.0).items_start().child(cover_el);
    if let Some(tr) = track {
        left = left
            .child(text(tr.title.clone()).font(TextStyle::new(26.0, 700.0)).color(Color::WHITE).max_w(art).truncate())
            .child(text(tr.artist.clone()).font(ty::HEADLINE).color(Color::WHITE.alpha(0.7)).max_w(art).truncate());
    }

    let mut stage = row().w_full().grow(1.0).min_h(0.0).px(70.0).gap(70.0).items_center().justify_center().child(left);
    if show_lyrics && let Some(lrc) = ui.lyrics.lrc() {
        let position = ui.player.position();
        let active = ui.lyric_line;
        let mut lines = col().key("im-lines").w_full().gap(18.0);
        // Autour de la ligne courante : quelques lignes avant, davantage après.
        let center = active.unwrap_or(0);
        let from = center.saturating_sub(3);
        let to = (center + 7).min(lrc.lines.len());
        for i in from..to {
            lines = lines.child(lyric_line(ui, i, active, position));
        }
        stage = stage.child(col().grow(1.0).w(0.0).max_w(720.0).h_full().justify_center().clip_fade(0.0, 60.0, 0.0, 60.0).child(lines));
    }

    let bar = row()
        .absolute(0.0, 0.0)
        .w_full()
        .h(56.0)
        .px(20.0)
        .gap(8.0)
        .items_center()
        .justify_end()
        .group_opacity(0.25, 1.0)
        .hover_group()
        .child(
            row()
                .key("im-lyrics")
                .size(36.0, 36.0)
                .radius(18.0)
                .center()
                .cursor(Cursor::Pointer)
                .fill(Color::WHITE.alpha(0.12))
                .role("button")
                .label(t("lyr.title"))
                .child(icon(&icons::MIC_VOCAL, 17.0).color(Color::WHITE))
                .on_press(|s: &mut Ui| {
                    let on = !s.settings.immersive_lyrics;
                    s.set(json!({ "immersiveLyrics": on }));
                }),
        )
        .child(
            row()
                .key("im-exit")
                .size(36.0, 36.0)
                .radius(18.0)
                .center()
                .cursor(Cursor::Pointer)
                .fill(Color::WHITE.alpha(0.12))
                .role("button")
                .label(t("common.close"))
                .child(icon(&icons::X, 17.0).color(Color::WHITE))
                .on_press(|s: &mut Ui| s.immersive = false),
        );

    el().key("immersive")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .child(bg)
        .child(col().absolute(0.0, 0.0).w_full().h_full().pt(56.0).pb(40.0).child(stage))
        .child(bar)
        .modal(false, |s: &mut Ui| s.immersive = false)
}

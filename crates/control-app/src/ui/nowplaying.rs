//! « À l'écoute » : pochette (ou vidéo) en grand, commandes, et un panneau File d'attente / Paroles / Égaliseur,
//! sur un fond de pochette floutée.

use rune::{Color, Cursor, Cx, El, ScrollSpec, SurfaceFit, col, el, icon, row, text};
use serde_json::json;

use control_core::queue::Repeat;

use crate::format;
use crate::i18n::t;
use crate::icons;
use crate::state::{MenuTarget, OpenMenu, Route, Ui};
use crate::theme::Palette;
use crate::ui::common::{CoverKind, cover, icon_button, ty};
use crate::ui::playerbar::{seek_bar, slider};

/// Hauteur d'une ligne de la file, espace compris (pour le glisser-déposer).
const QUEUE_ROW: f32 = 58.0;

const RATES: &[f64] = &[0.5, 0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 2.0];

fn tab_button(p: &Palette, key: &'static str, glyph: &'static rune::Icon, label: String, on: bool) -> El {
    row()
        .key(format!("np-tab-{key}"))
        .h(28.0)
        .px(12.0)
        .gap(6.0)
        .radius(p.r(7.0))
        .items_center()
        .cursor(Cursor::Pointer)
        .role("tab")
        .checked(on)
        .fill(if on { p.seg_thumb } else { Color::TRANSPARENT })
        .child(icon(glyph, 14.0).color(if on { p.text } else { p.text2 }))
        .child(text(label).font(ty::SMALL_MEDIUM).color(if on { p.text } else { p.text2 }))
        .on_press(move |s: &mut Ui| {
            s.set(json!({ "npTab": key }));
            if key == "lyrics" {
                crate::ui::lyrics::ensure(s, false);
            }
        })
}

fn queue_panel(ui: &Ui, p: &Palette) -> El {
    let items = ui.player.queue.items();
    let current = ui.player.queue.index();
    let mut list = col().w_full().gap(2.0);
    if items.is_empty() {
        return col().w_full().py(40.0).items_center().child(text(t("queue.empty")).font(ty::BODY).color(p.text2));
    }
    for (i, it) in items.iter().enumerate() {
        let Some(e) = ui.cat.entry(it.track) else { continue };
        let tr = &e.track;
        let playing = current == Some(i);
        let past = current.is_some_and(|c| i < c);
        let seed = if tr.album.is_empty() { tr.title.clone() } else { tr.album.clone() };
        let mut r = row()
            .key(format!("q-{}", it.uid))
            .w_full()
            .h(56.0)
            .px(8.0)
            .gap(12.0)
            .items_center()
            .radius(p.r(10.0))
            .hover(p.fill)
            .opacity(if past { 0.55 } else { 1.0 })
            .child(
                row().w(22.0).justify_center().child(if playing {
                    icon(&icons::AUDIO_LINES, 15.0).color(p.accent)
                } else {
                    el()
                }),
            )
            .child(cover(ui, tr.cover.as_deref(), &seed, 40.0, p.r(6.0), if tr.is_video() { CoverKind::Video } else { CoverKind::Music }))
            .child(
                col()
                    .gap(2.0)
                    .grow(1.0)
                    .w(0.0)
                    .min_w(0.0)
                    .child(text(tr.title.clone()).font(ty::BODY_MEDIUM).color(if playing { p.accent } else { p.text }).truncate())
                    .child(text(e.artist_label.clone()).font(ty::SMALL).color(p.text2).truncate()),
            )
            .child(text(format::time(tr.duration)).font(ty::NUMBERS).color(p.text2));
        // Glisser une entrée la déplace dans la file (sauf le morceau en cours).
        if let Some((from, dy)) = ui.queue_drag
            && from == i
        {
            r = r.offset(0.0, dy).raise().fill(p.fill_strong).instant_changes();
        }
        if !playing {
            r = r.on_drag(move |s: &mut Ui, d: &mut rune::Drag, _tx| match d.phase {
                rune::Phase::Down => s.queue_drag = Some((i, 0.0)),
                rune::Phase::Move => s.queue_drag = Some((i, d.dy)),
                rune::Phase::Up => {
                    s.queue_drag = None;
                    let steps = (d.dy / QUEUE_ROW).round() as i64;
                    if !d.cancelled() && steps != 0 {
                        let len = s.player.queue.len() as i64;
                        let to = (i as i64 + steps).clamp(0, len - 1) as usize;
                        let cat = s.cat.clone();
                        s.player.move_item(i, to, &cat);
                    }
                }
            });
        }
        r = r
            .on_double_press(move |s: &mut Ui| {
                let (cat, b) = (s.cat.clone(), s.backend.clone());
                s.player.jump_to(i, &cat, &b);
            })
            .on_secondary_press(move |s: &mut Ui, at, _| s.menu = Some(OpenMenu { at, target: MenuTarget::Queue(i), submenu: false }));
        list = list.child(r);
    }
    let clear = text(t("queue.clear"))
        .key("q-clear")
        .font(ty::SMALL_MEDIUM)
        .color(p.accent)
        .cursor(Cursor::Pointer)
        .on_press(|s: &mut Ui| {
            let cat = s.cat.clone();
            s.player.clear_upcoming(&cat);
        });
    col().w_full().gap(6.0).child(row().w_full().justify_end().pr(8.0).child(clear)).child(list)
}

fn eq_panel(ui: &Ui, p: &Palette, k: &rune_kit::Kit) -> El {
    crate::settings_ui::equalizer(ui, p, k)
}

/// La pochette, ou la vidéo en cours.
fn stage(ui: &Ui, p: &Palette, size: f32) -> El {
    let Some(tr) = &ui.player.track else {
        return el().size(size, size).radius(p.r(18.0)).fill(p.fill);
    };
    if tr.is_video()
        && let Some(px) = ui.video.pixels()
    {
        let (w, h) = ui.video.size().unwrap_or((16, 9));
        let ratio = w.max(1) as f32 / h.max(1) as f32;
        return el()
            .key("np-video")
            .w_full()
            .max_w(size * 1.6)
            .aspect_ratio(ratio)
            .radius(p.r(14.0))
            .clip_rounded(p.r(14.0))
            .fill(Color::BLACK)
            .pixels_fit(px, SurfaceFit::Fit)
            .on_press(|s: &mut Ui| s.player.toggle());
    }
    let seed = if tr.album.is_empty() { tr.title.clone() } else { tr.album.clone() };
    cover(ui, tr.cover.as_deref(), &seed, size, p.r(18.0), if tr.is_video() { CoverKind::Video } else { CoverKind::Music })
        .key("np-cover")
        .shadow(24.0, 60.0, Color::BLACK.alpha(0.45))
        .cursor(Cursor::Pointer)
        .on_press(|s: &mut Ui| s.player.toggle())
}

pub fn view(ui: &Ui, p: &Palette, k: &rune_kit::Kit, cx: &mut Cx<Ui>) -> El {
    if let Some([w, _]) = cx.layout_size("np-seek") {
        ui.live.np_bar_w.set(w);
    }
    let window = cx.layout_size("window").unwrap_or([1280.0, 820.0]);
    let size = (window[1] - 380.0).clamp(220.0, 480.0).min(window[0] * 0.36);
    let track = ui.player.track.as_ref();
    let playing = ui.player.is_playing();

    // En-tête : fermer, titre, mode immersif.
    let top = row()
        .w_full()
        .h(52.0)
        .px(20.0)
        .items_center()
        .justify_between()
        .child(
            icon_button(p, "np-close", &icons::CHEVRON_DOWN, 20.0, false).label(t("np.close")).on_press(|s: &mut Ui| s.now_playing = false),
        )
        .child(text(t("np.title")).font(ty::SMALL_MEDIUM).color(p.text2))
        .child(
            icon_button(p, "np-immersive", &icons::SPARKLES, 17.0, false)
                .label(t("np.immersive"))
                .on_press(|s: &mut Ui| {
                    s.immersive = true;
                    crate::ui::lyrics::ensure(s, false);
                }),
        );

    let mut info = col().w(size).gap(4.0).mt(18.0);
    if let Some(tr) = track {
        let entry = ui.cat.entry(tr.id);
        let artist = entry.map(|e| e.artist_label.clone()).unwrap_or_else(|| tr.artist.clone());
        let mut artist_el = text(if artist.is_empty() { t("album.unknown") } else { artist }).font(ty::TITLE).color(p.accent).truncate();
        if let Some(name) = entry.and_then(|e| e.artist_names.first().cloned()) {
            let key = control_core::artists::artist_key(&name);
            artist_el = artist_el.cursor(Cursor::Pointer).on_press(move |s: &mut Ui| {
                s.navigate(Route::Artist(key.clone()));
            });
        }
        let meta = [tr.album.clone(), tr.year.map(|y| y.to_string()).unwrap_or_default()]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        info = info
            .child(text(tr.title.clone()).font(rune::TextStyle::new(24.0, 700.0)).color(p.text).truncate())
            .child(artist_el)
            .child(text(meta).font(ty::SMALL).color(p.text2).truncate());
        if let Some(err) = &ui.player.error {
            info = info.child(text(err.clone()).font(ty::SMALL).color(p.danger).truncate());
        }
    }

    let progress = col()
        .w(size)
        .gap(4.0)
        .mt(14.0)
        .child(seek_bar(ui, p, "np-seek", &ui.live.np_bar, true))
        .child(
            row()
                .w_full()
                .justify_between()
                .child(text("0:00").live_text(&ui.live.elapsed).font(ty::NUMBERS).color(p.text3))
                .child(text("-0:00").live_text(&ui.live.remaining).font(ty::NUMBERS).color(p.text3)),
        );

    let repeat_icon = if ui.player.repeat == Repeat::One { &icons::REPEAT_1 } else { &icons::REPEAT };
    let controls = row()
        .w(size)
        .justify_between()
        .items_center()
        .mt(10.0)
        .child(icon_button(p, "np-shuffle", &icons::SHUFFLE, 20.0, ui.player.shuffle).on_press(|s: &mut Ui| {
            let on = !s.settings.shuffle;
            s.set(json!({ "shuffle": on }));
        }))
        .child(icon_button(p, "np-prev", &icons::SKIP_BACK, 24.0, false).on_press(|s: &mut Ui| {
            let (cat, b) = (s.cat.clone(), s.backend.clone());
            s.player.prev(&cat, &b);
        }))
        .child(
            row()
                .key("np-play")
                .size(68.0, 68.0)
                .radius(34.0)
                .center()
                .fill(p.text)
                .cursor(Cursor::Pointer)
                .press_scale(0.94)
                .role("button")
                .label(if playing { t("player.pause") } else { t("player.play") })
                .child(icon(if playing { &icons::PAUSE } else { &icons::PLAY }, 30.0).color(p.bg))
                .on_press(|s: &mut Ui| s.player.toggle()),
        )
        .child(icon_button(p, "np-next", &icons::SKIP_FORWARD, 24.0, false).on_press(|s: &mut Ui| {
            let (cat, b) = (s.cat.clone(), s.backend.clone());
            s.player.next(&cat, &b);
        }))
        .child(icon_button(p, "np-repeat", repeat_icon, 20.0, ui.player.repeat != Repeat::Off).on_press(|s: &mut Ui| {
            let next = s.player.repeat.cycle();
            s.set(json!({ "repeat": next.as_str() }));
        }));

    let rate = ui.settings.rate;
    let rate_label = if (rate - 1.0).abs() < 1e-6 { "1×".to_string() } else { format!("{rate}×") };
    let bottom = row()
        .w(size)
        .gap(10.0)
        .items_center()
        .mt(12.0)
        .child(icon(&icons::VOLUME_1, 16.0).color(p.text2))
        .child(slider(p, "np-volume", if ui.settings.muted { 0.0 } else { ui.settings.volume as f32 }, 150.0, t("player.volume"), |s, v| {
            s.set(json!({ "volume": (v as f64 * 100.0).round() / 100.0, "muted": false }))
        }))
        .child(el().grow(1.0))
        .child(
            row()
                .key("np-rate")
                .h(26.0)
                .px(10.0)
                .radius(p.r(7.0))
                .center()
                .fill(p.fill_strong)
                .cursor(Cursor::Pointer)
                .role("button")
                .label(t("player.rate"))
                .child(text(rate_label).font(ty::SMALL_MEDIUM).color(p.text))
                .on_press(move |s: &mut Ui| {
                    let i = RATES.iter().position(|r| (r - rate).abs() < 1e-6).unwrap_or(3);
                    let next = RATES[(i + 1) % RATES.len()];
                    s.set(json!({ "rate": next }));
                }),
        );

    let left = col()
        .grow(1.0)
        .w(0.0)
        .h_full()
        .center()
        .child(stage(ui, p, size))
        .child(info)
        .child(progress)
        .child(controls)
        .child(bottom);

    let tab = ui.settings.np_tab.as_str();
    let tabs = row()
        .p_tabs(p)
        .child(tab_button(p, "queue", &icons::LIST_MUSIC, t("queue.title"), tab == "queue"))
        .child(tab_button(p, "lyrics", &icons::MIC_VOCAL, t("lyr.title"), tab == "lyrics"))
        .child(tab_button(p, "eq", &icons::SLIDERS_HORIZONTAL, t("eq.title"), tab == "eq"));
    let body = match tab {
        "lyrics" => crate::ui::lyrics::panel(ui, p),
        "eq" => eq_panel(ui, p, k),
        _ => queue_panel(ui, p),
    };
    let panel = col()
        .key("np-panel")
        .w(460.0)
        .h_full()
        .radius(p.r(22.0))
        .fill(p.panel)
        .backdrop_blur(30.0)
        .border(1.0, p.panel_border)
        .clip_rounded(p.r(22.0))
        .child(row().w_full().justify_center().py(14.0).child(tabs))
        .child(col().key("np-panel-body").w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).scroll_edge_fade(10.0).child(col().w_full().px(14.0).pb(14.0).child(body)));

    // Fond : la pochette floutée, sous un voile.
    let mut bg = el().absolute(0.0, 0.0).w_full().h_full().fill(p.bg);
    if let Some(img) = track.and_then(|tr| ui.cover(tr.cover.as_deref(), true)) {
        bg = bg
            .child(el().absolute(0.0, 0.0).w_full().h_full().scale(1.2).pixels_fit(img.pixels(), SurfaceFit::Fill).blur(70.0).opacity(0.65))
            .child(el().absolute(0.0, 0.0).w_full().h_full().fill(p.veil));
    }

    el().key("nowplaying")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .child(bg)
        .child(
            col()
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .child(top)
                .child(row().w_full().grow(1.0).min_h(0.0).px(40.0).pb(36.0).gap(40.0).child(left).child(panel)),
        )
        .modal(false, |s: &mut Ui| s.now_playing = false)
}

/// Fond du panneau d'onglets (contrôle segmenté).
trait TabsBox {
    fn p_tabs(self, p: &Palette) -> Self;
}

impl TabsBox for El {
    fn p_tabs(self, p: &Palette) -> El {
        self.h(34.0).padding(3.0).gap(2.0).radius(p.r(9.0)).fill(p.fill)
    }
}

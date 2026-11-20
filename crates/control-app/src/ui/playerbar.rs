//! Barre de lecture flottante : morceau en cours, commandes, progression, volume.
//!
//! La position avance par des valeurs « live » (texte et rectangle) mises à jour à chaque image : la vue n'est pas
//! reconstruite pendant la lecture.

use rune::{Color, Cursor, Cx, Drag, El, Live, LiveText, Phase, col, el, icon, row, text};

use control_core::queue::Repeat;
use serde_json::json;

use crate::format;
use crate::i18n::t;
use crate::icons;
use crate::state::{Route, Ui};
use crate::theme::Palette;
use crate::ui::common::{CoverKind, cover, icon_button, ty};

/// Valeurs qui suivent la lecture sans reconstruire la vue.
pub struct LiveBits {
    pub elapsed: LiveText,
    pub remaining: LiveText,
    pub total: LiveText,
    /// Rectangle rempli de la barre de progression (x, y, largeur, hauteur).
    pub bar: Live<[f32; 4]>,
    pub bar_w: std::cell::Cell<f32>,
    /// Barre de progression de « À l'écoute ».
    pub np_bar: Live<[f32; 4]>,
    pub np_bar_w: std::cell::Cell<f32>,
    /// Position glissée par l'utilisateur (la lecture n'y saute qu'au relâchement).
    pub scrub: std::cell::Cell<Option<f64>>,
}

impl LiveBits {
    pub fn new() -> LiveBits {
        LiveBits {
            elapsed: LiveText::new("0:00"),
            remaining: LiveText::new("-0:00"),
            total: LiveText::new("0:00"),
            bar: Live::new([0.0, 0.0, 0.0, 4.0]),
            bar_w: std::cell::Cell::new(0.0),
            np_bar: Live::new([0.0, 0.0, 0.0, 4.0]),
            np_bar_w: std::cell::Cell::new(0.0),
            scrub: std::cell::Cell::new(None),
        }
    }

    /// Met les valeurs à la position courante (appelé à chaque image pendant la lecture).
    pub fn update(&self, position: f64, duration: f64) {
        let pos = self.scrub.get().unwrap_or(position);
        let frac = if duration > 0.0 { (pos / duration).clamp(0.0, 1.0) as f32 } else { 0.0 };
        self.elapsed.set(format::time(pos));
        self.remaining.set(format!("-{}", format::time((duration - pos).max(0.0))));
        self.total.set(format::time(duration));
        self.bar.set([0.0, 0.0, self.bar_w.get() * frac, 4.0]);
        self.np_bar.set([0.0, 0.0, self.np_bar_w.get() * frac, 4.0]);
    }
}

impl Default for LiveBits {
    fn default() -> Self {
        Self::new()
    }
}

/// Une barre de progression qui se glisse. `width` vient de la mise en page (0 avant la première).
pub fn seek_bar(ui: &Ui, p: &Palette, key: &str, fill: &Live<[f32; 4]>, np: bool) -> El {
    let k = key.to_string();
    el()
        .key(k)
        .w_full()
        .h(16.0)
        .center_y()
        .cursor(Cursor::Pointer)
        .role("slider")
        .label(t("player.seek"))
        .child(
            el().w_full().h(4.0).radius(2.0).fill(p.control).clip_rounded(2.0).child(
                el().absolute(0.0, 0.0).h(4.0).radius(2.0).fill(if np { p.text } else { p.text.alpha(0.85) }).live_rect(fill),
            ),
        )
        .on_drag(move |s: &mut Ui, d: &mut Drag, _tx| {
            let dur = s.player.duration();
            if dur <= 0.0 || d.w <= 0.0 {
                return;
            }
            let frac = (d.x / d.w).clamp(0.0, 1.0) as f64;
            match d.phase {
                Phase::Up => {
                    s.live.scrub.set(None);
                    if !d.cancelled() {
                        s.player.seek(frac * dur);
                    }
                }
                _ => s.live.scrub.set(Some(frac * dur)),
            }
            s.live.update(s.player.position(), dur);
        })
        .opacity(if ui.player.track.is_some() { 1.0 } else { 0.5 })
}

/// Petit curseur horizontal (volume, égaliseur…) : `value` de 0 à 1, `set` reçoit la nouvelle valeur.
pub fn slider(p: &Palette, key: &str, value: f32, width: f32, label: String, set: fn(&mut Ui, f32)) -> El {
    let v = value.clamp(0.0, 1.0);
    el()
        .key(key.to_string())
        .w(width)
        .h(16.0)
        .center_y()
        .cursor(Cursor::Pointer)
        .role("slider")
        .label(label)
        .range_value(v as f64, 0.0, 1.0)
        .child(
            el().w_full().h(4.0).radius(2.0).fill(p.control).child(el().w(width * v).h(4.0).radius(2.0).fill(p.text.alpha(0.85))),
        )
        .on_drag(move |s: &mut Ui, d: &mut Drag, _tx| {
            if d.w > 0.0 {
                set(s, (d.x / d.w).clamp(0.0, 1.0));
            }
        })
        .on_step(move |s: &mut Ui, delta, _tx| {
            let cur = s.settings.volume as f32;
            set(s, (cur + delta as f32 * 0.05).clamp(0.0, 1.0));
        })
}

fn set_volume(s: &mut Ui, v: f32) {
    s.set(json!({ "volume": (v as f64 * 100.0).round() / 100.0, "muted": false }));
}

pub fn player_bar(ui: &Ui, p: &Palette, cx: &mut Cx<Ui>) -> El {
    if let Some([w, _]) = cx.layout_size("pb-seek") {
        ui.live.bar_w.set(w);
    }
    ui.live.update(ui.player.position(), ui.player.duration());
    let track = ui.player.track.as_ref();
    let playing = ui.player.is_playing();

    let info: El = match track {
        Some(tr) => {
            let artist = ui.cat.entry(tr.id).map(|e| e.artist_label.clone()).unwrap_or_else(|| tr.artist.clone());
            let sub = [artist, tr.album.clone()].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" — ");
            let seed = if tr.album.is_empty() { tr.title.clone() } else { tr.album.clone() };
            row()
                .key("pb-info")
                .gap(12.0)
                .items_center()
                .min_w(0.0)
                .cursor(Cursor::Pointer)
                .on_press(|s: &mut Ui| s.now_playing = true)
                .child(cover(ui, tr.cover.as_deref(), &seed, 52.0, p.r(8.0), if tr.is_video() { CoverKind::Video } else { CoverKind::Music }))
                .child(
                    col()
                        .gap(2.0)
                        .min_w(0.0)
                        .shrink()
                        .child(text(tr.title.clone()).font(ty::BODY_MEDIUM).color(p.text).truncate())
                        .child(text(sub).font(ty::SMALL).color(p.text2).truncate()),
                )
        }
        None => col()
            .gap(2.0)
            .child(text(t("player.nothing")).font(ty::BODY_MEDIUM).color(p.text2))
            .child(text(t("player.nothingSub")).font(ty::SMALL).color(p.text3)),
    };

    let repeat_icon = if ui.player.repeat == Repeat::One { &icons::REPEAT_1 } else { &icons::REPEAT };
    let play = row()
        .key("pb-play")
        .size(42.0, 42.0)
        .radius(21.0)
        .center()
        .fill(p.text)
        .cursor(Cursor::Pointer)
        .role("button")
        .label(if playing { t("player.pause") } else { t("player.play") })
        .press_scale(0.94)
        .child(icon(if playing { &icons::PAUSE } else { &icons::PLAY }, 20.0).color(p.bg))
        .on_press(|s: &mut Ui| s.player.toggle());

    let controls = row()
        .gap(14.0)
        .items_center()
        .child(
            icon_button(p, "pb-shuffle", &icons::SHUFFLE, 17.0, ui.player.shuffle)
                .label(t("player.shuffle"))
                .on_press(|s: &mut Ui| {
                    let on = !s.settings.shuffle;
                    s.set(json!({ "shuffle": on }));
                }),
        )
        .child(icon_button(p, "pb-prev", &icons::SKIP_BACK, 18.0, false).label(t("player.prev")).on_press(|s: &mut Ui| {
            let (cat, b) = (s.cat.clone(), s.backend.clone());
            s.player.prev(&cat, &b);
        }))
        .child(play)
        .child(icon_button(p, "pb-next", &icons::SKIP_FORWARD, 18.0, false).label(t("player.next")).on_press(|s: &mut Ui| {
            let (cat, b) = (s.cat.clone(), s.backend.clone());
            s.player.next(&cat, &b);
        }))
        .child(
            icon_button(p, "pb-repeat", repeat_icon, 17.0, ui.player.repeat != Repeat::Off)
                .label(t("player.repeat"))
                .on_press(|s: &mut Ui| {
                    let next = s.player.repeat.cycle();
                    s.set(json!({ "repeat": next.as_str() }));
                }),
        )
        .child(
            icon_button(p, "pb-automix", &icons::BLEND, 17.0, ui.settings.automix)
                .label(t("player.automix"))
                .help(t("player.automixTip"))
                .on_press(|s: &mut Ui| {
                    let on = !s.settings.automix;
                    s.set(json!({ "automix": on }));
                }),
        );

    let progress = row()
        .w_full()
        .gap(10.0)
        .items_center()
        .child(text("0:00").live_text(&ui.live.elapsed).font(ty::NUMBERS).color(p.text3).w(40.0).align(rune::Align::End))
        .child(seek_bar(ui, p, "pb-seek", &ui.live.bar, false).grow(1.0))
        .child(text("0:00").live_text(&ui.live.total).font(ty::NUMBERS).color(p.text3).w(40.0));

    let center = col().w(420.0).gap(6.0).items_center().child(controls).child(progress);

    let vol_icon = if ui.settings.muted || ui.settings.volume <= 0.0 {
        &icons::VOLUME
    } else if ui.settings.volume < 0.5 {
        &icons::VOLUME_1
    } else {
        &icons::VOLUME_2
    };
    let right = row()
        .gap(6.0)
        .items_center()
        .justify_end()
        .child(icon_button(p, "pb-queue", &icons::LIST_MUSIC, 17.0, ui.now_playing).label(t("player.queue")).on_press(|s: &mut Ui| {
            s.now_playing = !s.now_playing;
        }))
        .child(icon_button(p, "pb-mute", vol_icon, 17.0, false).label(t("player.mute")).on_press(|s: &mut Ui| {
            let m = !s.settings.muted;
            s.set(json!({ "muted": m }));
        }))
        .child(slider(
            p,
            "pb-volume",
            if ui.settings.muted { 0.0 } else { ui.settings.volume as f32 },
            92.0,
            t("player.volume"),
            set_volume,
        ))
        .child(icon_button(p, "pb-expand", &icons::MAXIMIZE_2, 16.0, false).label(t("np.open")).on_press(|s: &mut Ui| {
            s.now_playing = true;
        }));

    let bar = row()
        .key("playerbar")
        .w_full()
        .max_w(900.0)
        .h(80.0)
        .px(14.0)
        .gap(16.0)
        .items_center()
        .radius(p.r(22.0))
        .fill(p.bar)
        .backdrop_blur(30.0)
        .border(1.0, p.panel_border)
        .shadow(18.0, 50.0, Color::BLACK.alpha(if p.dark { 0.45 } else { 0.18 }))
        .child(info.w(0.0).grow(1.0))
        .child(center)
        .child(right.w(0.0).grow(1.0));

    // Une vidéo ouvre « À l'écoute », où elle s'affiche en grand.
    let _ = Route::Songs;
    row().w_full().px(24.0).pb(16.0).justify_center().child(bar)
}

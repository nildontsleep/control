//! Récap façon « Wrapped » : une suite de cartes plein écran pour une année, un mois, une semaine ou un jour,
//! une image à enregistrer, et la proposition du récap du mois ou de la semaine écoulés
//! (`src/renderer/js/ui/wrapped.js`, `src/renderer/js/stats/recap-prompt.js`).

mod export;
pub mod period;
mod slides;

use std::sync::Arc;

use rune::{Align, Color, Cursor, El, Key, KeyEvent, Tx, col, el, icon, row, text};
use rune_kit::Kit;
use serde_json::json;

use control_core::stats::Stats;

use crate::i18n::{t, tv};
use crate::icons;
use crate::state::{Sheet, Ui};
use crate::theme::Palette;
use crate::ui::common::ty;
use crate::views::stats::dates::{add_days, cap, monday_of, month_name, today, weekday_index};
pub use period::{Kind, Lists, Period};
use slides::{Ctx, Slide};

/// Un récap ouvert.
pub struct Open {
    pub period: Period,
    pub data: Arc<Stats>,
    pub lists: Lists,
    pub slides: Vec<Slide>,
    pub index: usize,
    /// La dernière navigation allait en arrière (les éléments entrent par le haut).
    pub back: bool,
    /// Une autre période se charge.
    pub busy: bool,
}

/// Proposition du récap d'une période écoulée (notification avec « Voir »).
pub struct Offer {
    id: u64,
    pub text: String,
    period: Period,
    data: Arc<Stats>,
}

/// État du récap.
#[derive(Default)]
pub struct RecapState {
    pub open: Option<Open>,
    pub offer: Option<Offer>,
    /// Taille de la fenêtre, mesurée par le récap lui-même (tailles en `vh` de l'original).
    pub size: [f32; 2],
    seq: u64,
    pub saving: bool,
    /// Message affiché dans le récap jusqu'à l'instant donné (les toasts de la fenêtre sont dessous).
    pub notice: Option<(String, &'static str, f64)>,
}

/// Ouvre le récap d'une période : ses statistiques se chargent d'abord. `lists` : périodes qui ont de l'activité,
/// si on les connaît déjà (sinon celles que renvoie le chargement).
pub fn open(ui: &mut Ui, period: Period, lists: Option<Lists>) {
    let (from, to) = period.range();
    crate::views::stats::fetch(ui, from, to, move |s, res, _| match res {
        Ok(data) => open_with(s, period, Arc::new(data), lists.clone()),
        Err(err) => s.toast(err, "triangle-alert"),
    });
}

/// Ouvre le récap avec des statistiques déjà chargées.
pub fn open_with(ui: &mut Ui, period: Period, data: Arc<Stats>, lists: Option<Lists>) {
    let lists = lists.filter(|l| !l.years.is_empty()).unwrap_or_else(|| Lists::of(&data));
    let slides = slides::slides(&data);
    ui.recap.open = Some(Open { period, data, lists, slides, index: 0, back: false, busy: false });
    ui.recap.offer = None;
    ui.menu = None;
    ui.sheet = Some(Sheet::Recap);
}

pub fn close(ui: &mut Ui) {
    ui.recap.open = None;
    ui.recap.offer = None;
    if ui.sheet == Some(Sheet::Recap) {
        ui.sheet = None;
    }
}

/// Change de période sans fermer le récap.
fn set_period(ui: &mut Ui, next: Period) {
    let Some(o) = ui.recap.open.as_mut() else { return };
    if o.busy || o.period == next {
        return;
    }
    o.busy = true;
    let (from, to) = next.range();
    crate::views::stats::fetch(ui, from, to, move |s, res, _| {
        let Some(o) = s.recap.open.as_mut() else { return };
        o.busy = false;
        // En cas d'échec, on reste sur la période affichée.
        if let Ok(data) = res {
            o.slides = slides::slides(&data);
            o.data = Arc::new(data);
            o.period = next;
            o.index = 0;
            o.back = false;
        }
    });
}

/// Carte suivante (`+1`) ou précédente (`-1`).
fn go(ui: &mut Ui, d: i32) {
    let Some(o) = ui.recap.open.as_mut() else { return };
    let target = o.index as i32 + d;
    if target < 0 || target >= o.slides.len() as i32 {
        return;
    }
    o.index = target as usize;
    o.back = d < 0;
}

fn on_key(s: &mut Ui, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    match &ev.key {
        Key::Escape => close(s),
        Key::Right | Key::Enter => go(s, 1),
        Key::Text(c) if c == " " => go(s, 1),
        Key::Left => go(s, -1),
        _ => return false,
    }
    true
}

// ─── Fenêtre ─────────────────────────────────────────────────────────────────

/// Feuille du récap (`Sheet::Recap`) : le récap ouvert, sinon la proposition.
pub fn sheet(ui: &Ui, p: &Palette, _k: &Kit, layer: &mut Vec<El>) {
    if let Some(o) = &ui.recap.open {
        layer.push(window(ui, o));
    } else if let Some(offer) = &ui.recap.offer {
        layer.push(offer_card(p, offer));
    }
}

const WHITE: Color = Color::WHITE;
fn ink() -> Color {
    crate::theme::rgba(17, 17, 17, 1.0)
}

/// Pastille du sélecteur de période.
fn pill(key: String, label: String, on: bool, in_group: bool, press: impl Fn(&mut Ui) + 'static) -> El {
    let fill = if on {
        WHITE
    } else if in_group {
        Color::TRANSPARENT
    } else {
        WHITE.alpha(0.12)
    };
    row()
        .key(key)
        .px(10.0)
        .py(4.0)
        .radius(999.0)
        .fill(fill)
        .cursor(Cursor::Pointer)
        .role("radio")
        .checked(on)
        .hover(if on { Color::TRANSPARENT } else { WHITE.alpha(0.1) })
        .child(text(label).font(rune::TextStyle::new(12.0, 650.0)).color(if on { ink() } else { WHITE.alpha(0.75) }))
        .on_press(press)
}

fn round_button(key: &str, glyph: &'static rune::Icon, size: f32, icon_size: f32, alpha: f32, label: String) -> El {
    row()
        .key(key.to_string())
        .size(size, size)
        .radius(size / 2.0)
        .center()
        .fill(WHITE.alpha(alpha))
        .hover(WHITE.alpha(0.12))
        .cursor(Cursor::Pointer)
        .role("button")
        .label(label)
        .child(icon(glyph, icon_size).color(WHITE))
}

/// En haut à gauche : la durée (année, mois…), puis les années ou les flèches entre périodes.
fn head(o: &Open) -> El {
    let mut parts = row().pin(Some(18.0), Some(14.0), Some(118.0), None).flex_wrap().items_center().gap(10.0);
    let available: Vec<Kind> = Kind::ALL.into_iter().filter(|k| !o.lists.get(*k).is_empty()).collect();
    if available.len() > 1 {
        let mut modes = row().padding(3.0).gap(4.0).radius(999.0).fill(WHITE.alpha(0.12)).role("radiogroup");
        for kind in available {
            let on = o.period.kind() == kind;
            modes = modes.child(pill(format!("w-mode-{kind:?}"), kind.label(), on, true, move |s: &mut Ui| {
                let next = s.recap.open.as_ref().and_then(|o| o.period.switch_to(kind, &o.lists));
                if let Some(next) = next {
                    set_period(s, next);
                }
            }));
        }
        parts = parts.child(modes);
    }
    if let Period::Year(current) = o.period {
        if o.lists.years.len() > 1 {
            let mut years = row().gap(4.0);
            for y in o.lists.years.iter().take(6) {
                if let Ok(year) = y.parse::<i32>() {
                    years = years.child(pill(
                        format!("w-year-{year}"),
                        y.clone(),
                        year == current,
                        false,
                        move |s: &mut Ui| set_period(s, Period::Year(year)),
                    ));
                }
            }
            parts = parts.child(years);
        }
    } else {
        let (older, newer) = o.period.neighbours(&o.lists);
        let (before, after) = o.period.kind().steps();
        let arrow = |key: &str, glyph: &'static rune::Icon, target: Option<Period>, label: &str| {
            let b = row()
                .key(key.to_string())
                .size(26.0, 26.0)
                .radius(13.0)
                .center()
                .role("button")
                .label(t(label))
                .opacity(if target.is_some() { 1.0 } else { 0.35 })
                .child(icon(glyph, 15.0).color(WHITE));
            match target {
                Some(p) => {
                    b.cursor(Cursor::Pointer).hover(WHITE.alpha(0.22)).on_press(move |s: &mut Ui| set_period(s, p))
                }
                None => b,
            }
        };
        parts = parts.child(
            row()
                .padding(3.0)
                .gap(4.0)
                .items_center()
                .radius(999.0)
                .fill(WHITE.alpha(0.12))
                .child(arrow("w-older", &icons::CHEVRON_LEFT, older, before))
                .child(
                    text(o.period.label())
                        .font(rune::TextStyle::new(12.5, 650.0))
                        .color(WHITE)
                        .min_w(118.0)
                        .px(4.0)
                        .align(Align::Center),
                )
                .child(arrow("w-newer", &icons::CHEVRON_RIGHT, newer, after)),
        );
    }
    parts
}

/// Le bouton « Enregistrer l'image » de la dernière carte.
fn save_button(saving: bool) -> El {
    row()
        .key("w-save")
        .h(32.0)
        .px(14.0)
        .gap(6.0)
        .mt(6.0)
        .radius(8.0)
        .items_center()
        .fill(WHITE)
        .cursor(Cursor::Pointer)
        .role("button")
        .label(t("wrapped.save"))
        .opacity(if saving { 0.6 } else { 1.0 })
        .child(icon(&icons::DOWNLOAD, 15.0).color(ink()))
        .child(text(t("wrapped.save")).font(ty::BODY_MEDIUM).color(ink()))
        .on_press_tx(export::save_dialog)
}

fn window(ui: &Ui, o: &Open) -> El {
    let [mut w, mut h] = ui.recap.size;
    if w < 1.0 || h < 1.0 {
        (w, h) = (1280.0, 820.0);
    }
    let slide = o.slides.get(o.index).copied().unwrap_or(Slide::Empty);
    let cover = |hash: Option<&str>, small: bool| ui.cover(hash, small);
    let ctx = Ctx {
        period: o.period,
        data: &o.data,
        cover: &cover,
        height: h,
        today: today(),
        save: Some(save_button(ui.recap.saving)),
    };

    // Toucher la gauche de la scène revient en arrière, le reste avance.
    let taps = row()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .child(el().key("w-tap-back").grow(3.0).h_full().cursor(Cursor::Pointer).on_press(|s: &mut Ui| go(s, -1)))
        .child(el().key("w-tap-next").grow(7.0).h_full().cursor(Cursor::Pointer).on_press(|s: &mut Ui| go(s, 1)));
    let stage = row().absolute(0.0, 0.0).w_full().h_full().justify_center().child(
        col()
            .key("w-stage")
            .w(w.min(640.0))
            .h_full()
            .pt(80.0)
            .pb(60.0)
            .px(28.0)
            .justify_center()
            .items_center()
            .opacity(if o.busy { 0.35 } else { 1.0 })
            .child(taps)
            .child(slides::stage_content(&ctx, slide, o.index, o.back)),
    );

    let mut bars = row().w((w - 140.0).clamp(80.0, 560.0)).gap(5.0);
    for k in 0..o.slides.len() {
        bars = bars.child(
            el().key(format!("w-bar-{k}"))
                .grow(1.0)
                .h(3.5)
                .radius(3.0)
                .clip_rounded(3.0)
                .fill(WHITE.alpha(0.28))
                .child(el().h_full().w_frac(if k <= o.index { 1.0 } else { 0.0 }).fill(WHITE)),
        );
    }
    let bars = row().absolute(0.0, 58.0).w_full().justify_center().inert().child(bars);

    let close_btn = round_button("w-close", &icons::X, 34.0, 18.0, 0.18, t("common.close"))
        .pin(None, Some(12.0), Some(16.0), None)
        .on_press(close);
    let save_btn = round_button("w-save-top", &icons::DOWNLOAD, 34.0, 17.0, 0.18, t("wrapped.save"))
        .pin(None, Some(12.0), Some(58.0), None)
        .help(t("wrapped.save"))
        .on_press_tx(export::save_dialog);

    let side = (w / 2.0 - 400.0).max(16.0);
    let mut win = el()
        .key("recap")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .role("dialog")
        .label(o.period.label())
        .modal(false, close)
        .focusable(true)
        .autofocus()
        .no_focus_ring()
        .on_key(on_key)
        .on_layout(|s: &mut Ui, r| {
            let [w0, h0] = s.recap.size;
            if (w0 - r[2]).abs() > 0.5 || (h0 - r[3]).abs() > 0.5 {
                s.recap.size = [r[2], r[3]];
            }
        })
        .child(slides::backdrop(slides::palette(o.index), w, h))
        .child(stage)
        .child(bars)
        .child(head(o))
        .child(save_btn)
        .child(close_btn);
    if let Some((msg, glyph, until)) = &ui.recap.notice
        && ui.clock < *until
    {
        let glyph = icons::by_name(glyph).unwrap_or(&icons::CHECK);
        win = win.child(
            row().absolute(0.0, h - 64.0).w_full().justify_center().inert().child(
                row()
                    .key("w-notice")
                    .h(38.0)
                    .px(14.0)
                    .gap(8.0)
                    .items_center()
                    .radius(19.0)
                    .fill(WHITE.alpha(0.18))
                    .child(icon(glyph, 15.0).color(WHITE))
                    .child(text(msg.clone()).font(ty::BODY_MEDIUM).color(WHITE)),
            ),
        );
    }
    if o.index > 0 {
        win = win.child(
            round_button("w-prev", &icons::CHEVRON_LEFT, 44.0, 22.0, 0.16, t("tag.prev"))
                .absolute(side, h / 2.0 - 22.0)
                .on_press(|s: &mut Ui| go(s, -1)),
        );
    }
    if o.index + 1 < o.slides.len() {
        win = win.child(
            round_button("w-next", &icons::CHEVRON_RIGHT, 44.0, 22.0, 0.16, t("tag.next"))
                .pin(None, Some(h / 2.0 - 22.0), Some(side), None)
                .on_press(|s: &mut Ui| go(s, 1)),
        );
    }
    win
}

// ─── Proposition au démarrage ────────────────────────────────────────────────

/// Assez d'écoute (30 min) ou de visionnage (1 h) pour qu'un récap ait du sens.
fn enough(data: &Stats) -> bool {
    data.music.seconds >= 1800.0 || data.video.seconds >= 3600.0
}

/// Au démarrage : proposer le récap du mois écoulé en début de mois (jusqu'au 12), sinon celui de la semaine
/// écoulée du lundi au mercredi. Une seule proposition par période, même ignorée (`recapSeen`, `recapSeenWeek`).
pub fn maybe_prompt(ui: &mut Ui) {
    let now = today();
    if now.day() <= 12 {
        let before = add_days(now, -(now.day() as i64));
        let period = Period::Month(before.year(), u8::from(before.month()));
        let key = period.key();
        if ui.settings.recap_seen != key {
            let (from, to) = period.range();
            crate::views::stats::fetch(ui, from, to, move |s, res, _| {
                let Ok(data) = res else { return };
                s.set(json!({ "recapSeen": key }));
                if enough(&data) {
                    let month = month_name(u8::from(before.month()));
                    show_offer(s, tv("wrapped.monthReady", &[("month", &month)]), period, Arc::new(data));
                } else {
                    offer_week(s);
                }
            });
            return;
        }
    }
    offer_week(ui);
}

/// Du lundi au mercredi, la semaine précédente (du lundi au dimanche).
fn offer_week(ui: &mut Ui) {
    let now = today();
    if weekday_index(now) > 2 {
        return;
    }
    let period = Period::Week(add_days(monday_of(now), -7));
    let key = period.key();
    if ui.settings.recap_seen_week == key {
        return;
    }
    let (from, to) = period.range();
    crate::views::stats::fetch(ui, from, to, move |s, res, _| {
        let Ok(data) = res else { return };
        s.set(json!({ "recapSeenWeek": key }));
        if enough(&data) {
            show_offer(s, t("wrapped.weekReady"), period, Arc::new(data));
        }
    });
}

/// Affiche la proposition 14 secondes (sans recouvrir une autre feuille ouverte).
pub fn show_offer(ui: &mut Ui, text: String, period: Period, data: Arc<Stats>) {
    if ui.sheet.is_some() {
        return;
    }
    ui.recap.seq += 1;
    let id = ui.recap.seq;
    ui.recap.offer = Some(Offer { id, text, period, data });
    ui.sheet = Some(Sheet::Recap);
    ui.spawn(
        |_| std::thread::sleep(std::time::Duration::from_secs(14)),
        move |s, (), _| {
            if s.recap.open.is_none() && s.recap.offer.as_ref().is_some_and(|o| o.id == id) {
                close(s);
            }
        },
    );
}

fn see_offer(s: &mut Ui) {
    if let Some(offer) = s.recap.offer.take() {
        open_with(s, offer.period, offer.data, None);
    }
}

/// La proposition, en haut de la fenêtre comme une notification, avec « Voir ».
fn offer_card(p: &Palette, offer: &Offer) -> El {
    let see = row()
        .key("offer-see")
        .h(28.0)
        .px(12.0)
        .radius(14.0)
        .center()
        .fill(p.accent)
        .cursor(Cursor::Pointer)
        .role("button")
        .hover(WHITE.alpha(0.12))
        .child(text(t("wrapped.see")).font(ty::SMALL_MEDIUM).color(p.on_accent))
        .on_press(see_offer);
    let dismiss = row()
        .key("offer-close")
        .size(24.0, 24.0)
        .radius(12.0)
        .center()
        .cursor(Cursor::Pointer)
        .hover(p.fill_hover)
        .role("button")
        .label(t("common.close"))
        .child(icon(&icons::X, 14.0).color(p.text2))
        .on_press(close);
    let card = row()
        .key(format!("offer-{}", offer.id))
        .h(44.0)
        .pl(14.0)
        .pr(8.0)
        .gap(10.0)
        .items_center()
        .radius(22.0)
        .fill(p.menu)
        .backdrop_blur(20.0)
        .border(1.0, p.panel_border)
        .shadow(8.0, 24.0, Color::BLACK.alpha(0.3))
        .role("status")
        .child(icon(&icons::SPARKLES, 16.0).color(p.accent))
        .child(text(cap(&offer.text)).font(ty::BODY_MEDIUM).color(p.text))
        .child(see)
        .child(dismiss);
    row().absolute(0.0, 0.0).w_full().pt(18.0).justify_center().child(card)
}

/// Rendus sans fenêtre (`--scene recap`, `recap-offer`) : le récap de l'année (ou du mois) en cours, chargé tout de suite.
pub fn scene(ui: &mut Ui, offer: bool, slide: usize) {
    let now = today();
    let period = if offer { Period::Month(now.year(), u8::from(now.month())) } else { Period::Year(now.year()) };
    let (from, to) = period.range();
    let res = {
        let db = ui.backend.db.lock();
        control_core::stats::get(&db, &from, &to, crate::views::stats::TOP, control_core::util::now_ms())
    };
    let Ok(data) = res else { return };
    let data = Arc::new(data);
    if offer {
        show_offer(ui, tv("wrapped.monthReady", &[("month", &month_name(u8::from(now.month())))]), period, data);
    } else {
        open_with(ui, period, data, None);
        if let Some(o) = ui.recap.open.as_mut() {
            o.index = slide.min(o.slides.len().saturating_sub(1));
        }
    }
}

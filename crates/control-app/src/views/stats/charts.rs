//! Graphiques des statistiques : histogrammes à barres fines, calendrier façon « contributions », info-bulle.

use std::collections::HashMap;

use rune::{Align, Color, El, Side, col, el, row, text};
use time::Date;

use crate::i18n::t;
use crate::state::Ui;
use crate::theme::Palette;
use crate::ui::common::ty;
use crate::views::stats::dates::{add_days, day_label, day_str, fmt_duration, month_short, weekday_index};
use crate::views::stats::style;

/// Ce que montre l'info-bulle (immédiate, au-dessus de l'élément survolé).
#[derive(Debug, Clone, PartialEq)]
pub struct Tip {
    pub key: String,
    pub title: String,
    pub value: String,
}

/// Une barre : étiquette sous la barre, valeur en secondes, titre et texte de l'info-bulle.
pub struct Bar {
    pub label: String,
    pub value: f64,
    pub title: String,
    pub sub: String,
}

/// Paliers « jolis » pour l'axe : 10 min, 15 min, 30 min, 1 h, 2 h…
pub fn nice_max(seconds: f64) -> f64 {
    const STEPS: [f64; 11] =
        [600.0, 900.0, 1800.0, 3600.0, 7200.0, 10800.0, 14400.0, 21600.0, 28800.0, 43200.0, 86400.0];
    STEPS.iter().copied().find(|s| *s >= seconds).unwrap_or_else(|| (seconds / 3600.0).ceil() * 3600.0)
}

/// Survol d'un élément de graphique : montre ou cache son info-bulle.
fn with_tip(e: El, key: String, title: String, value: String) -> El {
    let k = key.clone();
    e.key(key).on_hover_change(move |s: &mut Ui, inside, _| {
        if inside {
            s.stats.tip = Some(Tip { key: k.clone(), title: title.clone(), value: value.clone() });
        } else if s.stats.tip.as_ref().is_some_and(|t| t.key == k) {
            s.stats.tip = None;
        }
    })
}

/// L'info-bulle, à poser hors de la zone qui défile (elle se place au-dessus de son élément).
pub fn tip(p: &Palette, tip: &Tip) -> El {
    let fill = if p.dark { crate::theme::rgba(50, 50, 56, 0.96) } else { crate::theme::rgba(255, 255, 255, 0.97) };
    col()
        .key("st-tip")
        .anchored(tip.key.clone(), Side::Top, Align::Center, 8.0)
        .px(10.0)
        .py(6.0)
        .gap(1.0)
        .radius(p.r(9.0))
        .fill(fill)
        .border(0.5, p.panel_border)
        .shadow(8.0, 24.0, Color::BLACK.alpha(0.3))
        .inert()
        .child(text(crate::views::stats::dates::cap(&tip.title)).font(ty::SMALL_MEDIUM).color(p.text))
        .child(text(tip.value.clone()).font(ty::SMALL).color(p.text2))
}

/// Histogramme : une barre par élément, axe gradué en durées si `axis`.
pub fn bar_chart(ui: &Ui, p: &Palette, key: &str, items: &[Bar], label_every: usize, height: f32, axis: bool) -> El {
    let max = nice_max(items.iter().map(|i| i.value).fold(1.0, f64::max));
    let hovered = ui.stats.tip.as_ref().map(|t| t.key.as_str()).filter(|k| k.starts_with(&format!("{key}-")));
    let accent = p.accent;
    let mut bars = row().w_full().h(height).gap(3.0).items_end();
    let mut labels = row().w_full().gap(3.0).mt(7.0);
    for (i, item) in items.iter().enumerate() {
        let pct = if item.value > 0.0 { ((item.value / max) as f32).max(0.025) } else { 0.0 };
        let bar_key = format!("{key}-{i}");
        let dim = hovered.is_some_and(|k| k != bar_key);
        let fill = el()
            .w_full()
            .max_w(20.0)
            .h(pct * height)
            .corners([p.r(4.0), p.r(4.0), 0.0, 0.0])
            .linear_gradient(std::f32::consts::PI, [(0.0, accent), (1.0, accent.alpha(0.72))])
            .opacity(if dim { 0.5 } else { 1.0 });
        let column = col().grow(1.0).min_w(0.0).h_full().items_center().justify_end().child(fill);
        bars = bars.child(with_tip(column, bar_key, item.title.clone(), item.sub.clone()));
        let label = if i % label_every.max(1) == 0 { item.label.clone() } else { String::new() };
        labels = labels
            .child(row().grow(1.0).min_w(0.0).justify_center().child(text(label).font(style::TINY).color(p.text3)));
    }
    let mut chart = el().w_full();
    if axis {
        let mut grid = el().absolute(0.0, 0.0).w_full().h(height).inert();
        for f in [1.0f32, 0.5, 0.0] {
            let y = height * (1.0 - f);
            let line = el().pin(Some(0.0), Some(y), Some(0.0), None).h(1.0).fill(if f == 0.0 {
                p.separator
            } else {
                p.separator.alpha(p.separator.a * 0.7)
            });
            grid = grid.child(line);
            if f > 0.0 {
                grid = grid
                    .child(text(fmt_duration(max * f as f64)).font(style::TINY).color(p.text3).absolute(0.0, y - 8.0));
            }
        }
        chart = chart.child(grid).child(col().w_full().pl(46.0).child(bars).child(labels));
    } else {
        chart = chart.child(col().w_full().child(bars).child(labels));
    }
    chart
}

/// Couleur d'un niveau du calendrier (0 : rien, 4 : le plus).
fn level_color(p: &Palette, level: usize) -> Color {
    match level {
        0 => p.fill,
        1 => p.accent.alpha(0.28),
        2 => p.accent.alpha(0.5),
        3 => p.accent.alpha(0.75),
        _ => p.accent,
    }
}

/// Seuils des niveaux : les quartiles des jours écoutés.
pub fn level_cuts(values: &[f64]) -> [f64; 3] {
    let mut v: Vec<f64> = values.iter().copied().filter(|v| *v > 0.0).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q = |f: f64| if v.is_empty() { 0.0 } else { v[((v.len() as f64 * f).floor() as usize).min(v.len() - 1)] };
    [q(0.25), q(0.5), q(0.75)]
}

pub fn level(v: f64, cuts: &[f64; 3]) -> usize {
    if v <= 0.0 {
        0
    } else if v <= cuts[0] {
        1
    } else if v <= cuts[1] {
        2
    } else if v <= cuts[2] {
        3
    } else {
        4
    }
}

/// Calendrier : une colonne par semaine (du lundi au dimanche), les `weeks` dernières semaines.
pub fn heatmap(p: &Palette, by_day: &HashMap<String, f64>, today: Date, weeks: usize) -> El {
    let start = add_days(today, -((weeks as i64 - 1) * 7 + weekday_index(today) as i64));
    let values: Vec<f64> = by_day.values().copied().collect();
    let cuts = level_cuts(&values);
    let radius = p.r(3.5);
    let mut grid = row().w_full().gap(3.0);
    let mut months = row().w_full().gap(3.0);
    let mut last_month = 0u8;
    for w in 0..weeks {
        let mut column = col().grow(1.0).min_w(0.0).gap(3.0).items_center();
        for d in 0..7 {
            let date = add_days(start, (w * 7 + d) as i64);
            let cell = el().w_full().max_w(18.0).aspect_ratio(1.0).radius(radius);
            if date > today {
                column = column.child(cell);
                continue;
            }
            let key = day_str(date);
            let v = by_day.get(&key).copied().unwrap_or(0.0);
            let value = if v > 0.0 { fmt_duration(v) } else { t("stats.nothing") };
            let cell = cell.fill(level_color(p, level(v, &cuts)));
            column = column.child(with_tip(cell, format!("st-heat-{key}"), day_label(date), value));
        }
        grid = grid.child(column);
        let first = add_days(start, (w * 7) as i64);
        let m = u8::from(first.month());
        let label = if m != last_month && first.day() <= 7 { month_short(m).to_string() } else { String::new() };
        if first.day() <= 7 {
            last_month = m;
        }
        months = months.child(el().grow(1.0).min_w(0.0).child(text(label).font(style::TINY).color(p.text3)));
    }
    let mut legend = row()
        .w_full()
        .justify_end()
        .items_center()
        .gap(4.0)
        .mt(4.0)
        .child(text(t("stats.less")).font(style::TINY).color(p.text3));
    for l in 0..5 {
        legend = legend.child(el().size(12.0, 12.0).radius(radius).fill(level_color(p, l)));
    }
    legend = legend.child(text(t("stats.more")).font(style::TINY).color(p.text3));
    col().w_full().gap(6.0).child(months).child(grid).child(legend)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_steps_and_levels() {
        assert_eq!(nice_max(1.0), 600.0);
        assert_eq!(nice_max(3000.0), 3600.0);
        assert_eq!(nice_max(100_000.0), 100_800.0);
        let cuts = level_cuts(&[0.0, 10.0, 20.0, 30.0, 40.0]);
        assert_eq!(cuts, [20.0, 30.0, 40.0]);
        assert_eq!(level(0.0, &cuts), 0);
        assert_eq!(level(10.0, &cuts), 1);
        assert_eq!(level(41.0, &cuts), 4);
    }
}

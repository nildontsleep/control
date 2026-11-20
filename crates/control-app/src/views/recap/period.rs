//! Périodes d'un récap (année, mois, semaine, jour) : bornes, clés, titres et libellés, comme `wrapped.js`.

use time::Date;

use control_core::stats::Stats;

use crate::i18n::{t, tn};
use crate::views::stats::dates::{
    add_days, cap, day_label, day_month, day_month_short, day_str, monday_of, month_name, parse_day, weekday_day_month,
    weekday_index, weekday_name,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Year,
    Month,
    Week,
    Day,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Year, Kind::Month, Kind::Week, Kind::Day];

    /// Suffixe des textes propres à la durée de la période (année : aucun).
    pub fn suffix(self) -> &'static str {
        match self {
            Kind::Year => "",
            Kind::Month => "Month",
            Kind::Week => "Week",
            Kind::Day => "Day",
        }
    }

    /// Libellé du sélecteur (« Année », « Mois »…).
    pub fn label(self) -> String {
        t(match self {
            Kind::Year => "wrapped.modeYear",
            Kind::Month => "wrapped.modeMonth",
            Kind::Week => "wrapped.modeWeek",
            Kind::Day => "wrapped.modeDay",
        })
    }

    /// Libellés des flèches « précédent » et « suivant ».
    pub fn steps(self) -> (&'static str, &'static str) {
        match self {
            Kind::Year => ("", ""),
            Kind::Month => ("wrapped.monthBefore", "wrapped.monthAfter"),
            Kind::Week => ("wrapped.weekBefore", "wrapped.weekAfter"),
            Kind::Day => ("wrapped.dayBefore", "wrapped.dayAfter"),
        }
    }
}

/// Une période : l'année, un mois (1 à 12), une semaine (son lundi) ou un jour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Year(i32),
    Month(i32, u8),
    Week(Date),
    Day(Date),
}

/// Périodes qui ont de l'activité, la plus récente d'abord (clés de `Period::key`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lists {
    pub years: Vec<String>,
    pub months: Vec<String>,
    pub weeks: Vec<String>,
    pub days: Vec<String>,
}

impl Lists {
    pub fn of(data: &Stats) -> Lists {
        Lists {
            years: data.years.iter().map(|y| y.to_string()).collect(),
            months: data.months.clone(),
            weeks: weeks_of(&data.days),
            days: data.days.clone(),
        }
    }

    pub fn get(&self, kind: Kind) -> &[String] {
        match kind {
            Kind::Year => &self.years,
            Kind::Month => &self.months,
            Kind::Week => &self.weeks,
            Kind::Day => &self.days,
        }
    }
}

/// Lundis des semaines qui ont de l'activité, la plus récente d'abord.
pub fn weeks_of(days: &[String]) -> Vec<String> {
    let mut weeks: Vec<String> = days.iter().filter_map(|d| parse_day(d)).map(|d| day_str(monday_of(d))).collect();
    weeks.sort();
    weeks.dedup();
    weeks.reverse();
    weeks
}

fn last_day_of_month(year: i32, month: u8) -> u8 {
    time::Month::try_from(month).map(|m| m.length(year)).unwrap_or(31)
}

/// « 20 – 26 octobre » ou « 28 sept. – 4 oct. » (avec l'année si demandé).
fn week_span(start: Date, with_year: bool) -> String {
    let end = add_days(start, 6);
    let text = if start.month() == end.month() {
        if crate::i18n::current() == crate::i18n::Lang::Fr {
            format!("{} – {}", start.day(), day_month(end))
        } else {
            format!("{} – {}", day_month(start), end.day())
        }
    } else {
        format!("{} – {}", day_month_short(start), day_month_short(end))
    };
    if with_year { format!("{text} {}", end.year()) } else { text }
}

impl Period {
    pub fn kind(&self) -> Kind {
        match self {
            Period::Year(_) => Kind::Year,
            Period::Month(..) => Kind::Month,
            Period::Week(_) => Kind::Week,
            Period::Day(_) => Kind::Day,
        }
    }

    /// Premier et dernier jour (inclus), au format « AAAA-MM-JJ ».
    pub fn range(&self) -> (String, String) {
        match *self {
            Period::Year(y) => (format!("{y:04}-01-01"), format!("{y:04}-12-31")),
            Period::Month(y, m) => {
                (format!("{y:04}-{m:02}-01"), format!("{y:04}-{m:02}-{:02}", last_day_of_month(y, m)))
            }
            Period::Week(start) => (day_str(start), day_str(add_days(start, 6))),
            Period::Day(d) => (day_str(d), day_str(d)),
        }
    }

    /// Clé (nom de fichier, mémoire des récaps proposés, listes de périodes).
    pub fn key(&self) -> String {
        match *self {
            Period::Year(y) => y.to_string(),
            Period::Month(y, m) => format!("{y:04}-{m:02}"),
            Period::Week(start) => day_str(start),
            Period::Day(d) => day_str(d),
        }
    }

    pub fn parse(kind: Kind, key: &str) -> Option<Period> {
        match kind {
            Kind::Year => key.parse().ok().map(Period::Year),
            Kind::Month => {
                let y = key.get(0..4)?.parse().ok()?;
                let m: u8 = key.get(5..7)?.parse().ok()?;
                (1..=12).contains(&m).then_some(Period::Month(y, m))
            }
            Kind::Week => parse_day(key).map(|d| Period::Week(monday_of(d))),
            Kind::Day => parse_day(key).map(Period::Day),
        }
    }

    /// Nom du mois en capitale (« Novembre »).
    fn month_title(m: u8) -> String {
        cap(month_name(m))
    }

    /// Titre court, en grand sur la première carte.
    pub fn title(&self) -> String {
        match *self {
            Period::Year(y) => y.to_string(),
            Period::Month(_, m) => Self::month_title(m),
            Period::Week(start) => tn("wrapped.weekN", start.iso_week()),
            Period::Day(d) => cap(&format!("{} {}", weekday_name(weekday_index(d)), d.day())),
        }
    }

    /// Précision sous « control » : l'année, la plage de jours, le mois.
    pub fn sub(&self) -> String {
        match *self {
            Period::Year(_) => String::new(),
            Period::Month(y, _) => y.to_string(),
            Period::Week(start) => week_span(start, true),
            Period::Day(d) => cap(&format!("{} {}", month_name(u8::from(d.month())), d.year())),
        }
    }

    /// « control · 2026 » (ou « control » seul).
    pub fn eyebrow(&self) -> String {
        let sub = self.sub();
        if sub.is_empty() { "control".into() } else { format!("control · {sub}") }
    }

    /// Libellé complet (« Novembre 2026 », « Semaine 45 · 2 – 8 novembre »).
    pub fn label(&self) -> String {
        match *self {
            Period::Year(y) => y.to_string(),
            Period::Month(y, m) => format!("{} {y}", Self::month_title(m)),
            Period::Week(start) => format!("{} · {}", tn("wrapped.weekN", start.iso_week()), week_span(start, false)),
            Period::Day(d) => day_label(d),
        }
    }

    /// Libellé de menu : « Aujourd'hui », « Hier », « Cette semaine »… sinon le libellé complet.
    pub fn menu_label(&self, today: Date) -> String {
        match *self {
            Period::Day(d) => {
                if d == today {
                    t("wrapped.today")
                } else if d == add_days(today, -1) {
                    t("wrapped.yesterday")
                } else {
                    cap(&weekday_day_month(d))
                }
            }
            Period::Week(start) if start == monday_of(today) => t("wrapped.thisWeek"),
            Period::Week(start) if start == monday_of(add_days(today, -7)) => t("wrapped.lastWeek"),
            _ => self.label(),
        }
    }

    /// Nom du fichier image proposé (« control-2026 », « control-month-2026-11 »).
    pub fn file_name(&self) -> String {
        match self.kind() {
            Kind::Year => format!("control-{}", self.key()),
            Kind::Month => format!("control-month-{}", self.key()),
            Kind::Week => format!("control-week-{}", self.key()),
            Kind::Day => format!("control-day-{}", self.key()),
        }
    }

    /// Passer à une autre durée : la période de `kind` qui recoupe celle-ci, sinon la plus proche avant,
    /// sinon la plus ancienne.
    pub fn switch_to(&self, kind: Kind, lists: &Lists) -> Option<Period> {
        let list = lists.get(kind);
        if kind == self.kind() || list.is_empty() {
            return None;
        }
        let (from, to) = self.range();
        let periods: Vec<Period> = list.iter().filter_map(|k| Period::parse(kind, k)).collect();
        periods
            .iter()
            .find(|p| {
                let (a, b) = p.range();
                a <= to && b >= from
            })
            .or_else(|| periods.iter().find(|p| p.range().1 < from))
            .or_else(|| periods.last())
            .copied()
    }

    /// Périodes voisines qui ont de l'activité : (plus ancienne, plus récente).
    pub fn neighbours(&self, lists: &Lists) -> (Option<Period>, Option<Period>) {
        let kind = self.kind();
        let key = self.key();
        let list = lists.get(kind);
        let older = list.iter().find(|k| **k < key).and_then(|k| Period::parse(kind, k));
        let newer = list.iter().rev().find(|k| **k > key).and_then(|k| Period::parse(kind, k));
        (older, newer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> Date {
        parse_day(s).expect("date")
    }

    #[test]
    fn ranges_keys_and_labels() {
        let _guard = crate::i18n::TEST_LANG.lock().unwrap_or_else(|e| e.into_inner());
        crate::i18n::set_language("fr");
        assert_eq!(Period::Month(2024, 2).range(), ("2024-02-01".into(), "2024-02-29".into()));
        assert_eq!(Period::Year(2025).range(), ("2025-01-01".into(), "2025-12-31".into()));
        let w = Period::parse(Kind::Week, "2026-11-04").expect("semaine");
        assert_eq!(w, Period::Week(day("2026-11-02")));
        assert_eq!(w.range().1, "2026-11-08");
        assert_eq!(w.title(), "Semaine 45");
        assert_eq!(w.label(), "Semaine 45 · 2 – 8 novembre");
        assert_eq!(Period::Week(day("2026-09-28")).sub(), "28 sept. – 4 oct. 2026");
        assert_eq!(Period::Month(2026, 9).label(), "Septembre 2026");
        assert_eq!(Period::Day(day("2026-11-02")).title(), "Lundi 2");
        assert_eq!(Period::Day(day("2026-11-02")).sub(), "Novembre 2026");
        assert_eq!(Period::Month(2026, 11).file_name(), "control-month-2026-11");
        assert_eq!(Period::Year(2026).file_name(), "control-2026");
        let today = day("2026-11-20");
        assert_eq!(Period::Day(today).menu_label(today), "Aujourd’hui");
        assert_eq!(Period::Week(day("2026-11-09")).menu_label(today), "La semaine dernière");
        assert_eq!(Period::parse(Kind::Month, "2026-13"), None);
    }

    #[test]
    fn switching_and_stepping_between_periods() {
        let lists = Lists {
            years: vec!["2026".into(), "2025".into()],
            months: vec!["2026-11".into(), "2026-09".into(), "2025-12".into()],
            weeks: weeks_of(&["2026-11-18".into(), "2026-11-17".into(), "2026-09-03".into()]),
            days: vec!["2026-11-18".into(), "2026-11-17".into(), "2026-09-03".into()],
        };
        assert_eq!(lists.weeks, vec!["2026-11-16".to_string(), "2026-08-31".to_string()]);
        let m = Period::Month(2026, 9);
        assert_eq!(m.neighbours(&lists), (Some(Period::Month(2025, 12)), Some(Period::Month(2026, 11))));
        assert_eq!(m.switch_to(Kind::Day, &lists), Some(Period::Day(day("2026-09-03"))));
        assert_eq!(Period::Month(2026, 10).switch_to(Kind::Week, &lists), Some(Period::Week(day("2026-08-31"))));
        assert_eq!(Period::Year(2026).switch_to(Kind::Month, &lists), Some(Period::Month(2026, 11)));
        assert_eq!(m.switch_to(Kind::Month, &lists), None);
    }
}

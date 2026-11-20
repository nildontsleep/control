//! Jours, noms de jours et de mois, durées lisibles : les formats des statistiques et du récap
//! (`src/renderer/js/stats/format.js`, et les `toLocaleDateString` de l'original, en français et en anglais).

use time::{Date, Duration, Month};

use crate::format::count;
use crate::i18n::{self, Lang, t, tn};

const MONTHS_FR: [&str; 12] = [
    "janvier",
    "février",
    "mars",
    "avril",
    "mai",
    "juin",
    "juillet",
    "août",
    "septembre",
    "octobre",
    "novembre",
    "décembre",
];
const MONTHS_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTHS_SHORT_FR: [&str; 12] =
    ["janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc."];
const MONTHS_SHORT_EN: [&str; 12] =
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
/// Du lundi au dimanche.
const DAYS_FR: [&str; 7] = ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"];
const DAYS_EN: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const DAYS_SHORT_FR: [&str; 7] = ["lun", "mar", "mer", "jeu", "ven", "sam", "dim"];
const DAYS_SHORT_EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

fn fr() -> bool {
    i18n::current() == Lang::Fr
}

/// Aujourd'hui, en date locale.
pub fn today() -> Date {
    control_core::util::local_datetime(control_core::util::now_ms()).date()
}

/// « AAAA-MM-JJ » → date.
pub fn parse_day(s: &str) -> Option<Date> {
    let y: i32 = s.get(0..4)?.parse().ok()?;
    let m: u8 = s.get(5..7)?.parse().ok()?;
    let d: u8 = s.get(8..10)?.parse().ok()?;
    Date::from_calendar_date(y, Month::try_from(m).ok()?, d).ok()
}

/// Date → « AAAA-MM-JJ ».
pub fn day_str(d: Date) -> String {
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}

pub fn add_days(d: Date, n: i64) -> Date {
    d.checked_add(Duration::days(n)).unwrap_or(d)
}

/// Rang du jour dans la semaine, lundi = 0.
pub fn weekday_index(d: Date) -> usize {
    d.weekday().number_days_from_monday() as usize
}

/// Lundi de la semaine d'un jour.
pub fn monday_of(d: Date) -> Date {
    add_days(d, -(weekday_index(d) as i64))
}

/// Première lettre en capitale (« lundi » → « Lundi »).
pub fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Nom du mois (1 à 12), en minuscules en français.
pub fn month_name(m: u8) -> &'static str {
    let i = (m.clamp(1, 12) - 1) as usize;
    if fr() { MONTHS_FR[i] } else { MONTHS_EN[i] }
}

/// Nom court du mois (« nov. », « Nov »).
pub fn month_short(m: u8) -> &'static str {
    let i = (m.clamp(1, 12) - 1) as usize;
    if fr() { MONTHS_SHORT_FR[i] } else { MONTHS_SHORT_EN[i] }
}

/// Nom du jour de la semaine (lundi = 0).
pub fn weekday_name(i: usize) -> &'static str {
    let i = i.min(6);
    if fr() { DAYS_FR[i] } else { DAYS_EN[i] }
}

/// Nom court du jour (« lun », « Mon »).
pub fn weekday_short(i: usize) -> &'static str {
    let i = i.min(6);
    if fr() { DAYS_SHORT_FR[i] } else { DAYS_SHORT_EN[i] }
}

fn weekday_of(d: Date) -> &'static str {
    weekday_name(weekday_index(d))
}

/// « lundi 3 novembre 2026 » / « Monday, November 3, 2026 ».
pub fn day_label(d: Date) -> String {
    let m = u8::from(d.month());
    if fr() {
        format!("{} {} {} {}", weekday_of(d), d.day(), month_name(m), d.year())
    } else {
        format!("{}, {} {}, {}", weekday_of(d), month_name(m), d.day(), d.year())
    }
}

/// « lundi 3 novembre » / « Monday, November 3 ».
pub fn weekday_day_month(d: Date) -> String {
    let m = u8::from(d.month());
    if fr() {
        format!("{} {} {}", weekday_of(d), d.day(), month_name(m))
    } else {
        format!("{}, {} {}", weekday_of(d), month_name(m), d.day())
    }
}

/// « 3 novembre » / « November 3 ».
pub fn day_month(d: Date) -> String {
    let m = u8::from(d.month());
    if fr() { format!("{} {}", d.day(), month_name(m)) } else { format!("{} {}", month_name(m), d.day()) }
}

/// « 3 nov. » / « Nov 3 ».
pub fn day_month_short(d: Date) -> String {
    let m = u8::from(d.month());
    if fr() { format!("{} {}", d.day(), month_short(m)) } else { format!("{} {}", month_short(m), d.day()) }
}

/// Étiquette courte sous une barre : jour de la semaine sur 7 jours, sinon numéro (avec le mois le 1er).
pub fn short_day(d: Date, days: u32) -> String {
    if days <= 7 {
        return weekday_short(weekday_index(d)).to_string();
    }
    if d.day() == 1 { day_month_short(d) } else { d.day().to_string() }
}

/// 0 → « 0 min », 40 s → « < 1 min », 7 500 s → « 2 h 05 min ».
pub fn fmt_duration(seconds: f64) -> String {
    let s = if seconds.is_finite() { seconds.max(0.0).round() as i64 } else { 0 };
    if s == 0 {
        return "0 min".into();
    }
    if s < 60 {
        return t("stats.lessMinute");
    }
    let total = (s as f64 / 60.0).round() as i64;
    if total < 60 {
        return format!("{total} min");
    }
    let (hours, minutes) = (total / 60, total % 60);
    if hours >= 100 || minutes == 0 {
        return format!("{} h", count(hours));
    }
    format!("{} h {minutes:02} min", count(hours))
}

/// Texte avec un nombre au pluriel, le nombre écrit avec ses séparateurs (« 1 234 lectures »).
pub fn tcount(key: &str, n: i64) -> String {
    let raw = n.to_string();
    let s = tn(key, n);
    s.replacen(&raw, &count(n), 1)
}

/// « 3 lectures ».
pub fn plays(n: i64) -> String {
    tcount("stats.plays", n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_follow_the_original() {
        let _guard = crate::i18n::TEST_LANG.lock().unwrap_or_else(|e| e.into_inner());
        crate::i18n::set_language("fr");
        assert_eq!(fmt_duration(0.0), "0 min");
        assert_eq!(fmt_duration(40.0), "< 1 min");
        assert_eq!(fmt_duration(7500.0), "2 h 05 min");
        assert_eq!(fmt_duration(7200.0), "2 h");
        assert_eq!(fmt_duration(400.0 * 3600.0 + 120.0), "400 h");
        assert_eq!(plays(1234), "1\u{202f}234 lectures");
        assert_eq!(plays(1), "1 lecture");
        let d = parse_day("2026-11-02").expect("date");
        assert_eq!(day_str(d), "2026-11-02");
        assert_eq!(day_label(d), "lundi 2 novembre 2026");
        assert_eq!(short_day(d, 7), "lun");
        assert_eq!(short_day(d, 30), "2");
        assert_eq!(short_day(parse_day("2026-11-01").expect("date"), 30), "1 nov.");
        assert_eq!(monday_of(parse_day("2026-11-08").expect("date")), d);
        assert_eq!(d.weekday(), time::Weekday::Monday);
        crate::i18n::set_language("en");
        assert_eq!(day_label(d), "Monday, November 2, 2026");
        assert_eq!(plays(1234), "1,234 plays");
        crate::i18n::set_language("fr");
        assert_eq!(parse_day("2026-13-01"), None);
        assert_eq!(cap("été"), "Été");
    }
}

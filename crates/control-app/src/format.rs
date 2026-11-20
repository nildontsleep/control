//! Formats d'affichage : durées, tailles, dates.

use crate::i18n::{self, Lang};

/// « 3:07 », « 1:02:03 ».
pub fn time(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "0:00".into();
    }
    let s = seconds.floor() as u64;
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{sec:02}") } else { format!("{m}:{sec:02}") }
}

/// « 1 h 24 min », « 42 min ».
pub fn long(seconds: f64) -> String {
    let total = (seconds.max(0.0) / 60.0).round() as u64;
    let (h, m) = (total / 60, total % 60);
    match (h, m) {
        (0, m) => format!("{} min", m.max(1)),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// « 3,4 Mo » (« 3.4 MB » en anglais).
pub fn bytes(n: f64) -> String {
    if !n.is_finite() {
        return String::new();
    }
    let units = ["o", "Ko", "Mo", "Go"];
    let mut v = n;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    let fr = i18n::current() == Lang::Fr;
    let unit = if fr { units[i].to_string() } else { units[i].replace('o', "B") };
    let num = if v >= 100.0 || i == 0 { format!("{}", v.round() as u64) } else { format!("{v:.1}") };
    let num = if fr { num.replace('.', ",") } else { num };
    format!("{num} {unit}")
}

/// Nombre avec séparateur de milliers (espace fine en français, virgule en anglais).
pub fn count(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let sep = if i18n::current() == Lang::Fr { '\u{202f}' } else { ',' };
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(time(0.0), "0:00");
        assert_eq!(time(187.9), "3:07");
        assert_eq!(time(3723.0), "1:02:03");
        assert_eq!(time(f64::NAN), "0:00");
        assert_eq!(long(20.0), "1 min");
        assert_eq!(long(3600.0), "1 h");
        assert_eq!(long(5040.0), "1 h 24 min");
    }

    #[test]
    fn sizes_and_counts() {
        crate::i18n::set_language("fr");
        assert_eq!(bytes(512.0), "512 o");
        assert_eq!(bytes(3.4 * 1024.0 * 1024.0), "3,4 Mo");
        assert_eq!(count(1234567), "1\u{202f}234\u{202f}567");
        crate::i18n::set_language("en");
        assert_eq!(bytes(3.4 * 1024.0 * 1024.0), "3.4 MB");
        assert_eq!(count(1234), "1,234");
        crate::i18n::set_language("fr");
    }
}

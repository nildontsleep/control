//! Format LRC : « [mm:ss.xx] texte ». Plusieurs repères par ligne, décalage `[offset:]` pour tout le fichier,
//! mots minutés `<mm:ss.xx>` (LRC « enrichi »), et repli en texte brut quand il y a moins de trois repères.
//!
//! Tout est pur : l'interface analyse le texte reçu de [`crate::lyrics`], puis interroge [`Lrc::line_at`] et
//! [`Lrc::lit_words`] à chaque image.

use std::sync::LazyLock;

use regex::{Captures, Regex};

// `[0-9]` et non `\d` : en Rust `\d` reconnaît aussi les chiffres non latins, contrairement au JavaScript d'origine.
static STAMP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([0-9]{1,3}):([0-9]{2})(?:[.:]([0-9]{1,3}))?\]").expect("regex constante"));
static WORD_STAMP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<([0-9]{1,3}):([0-9]{2})(?:[.:]([0-9]{1,3}))?>").expect("regex constante"));
/// `[ar:…]`, `[ti:…]`, `[al:…]`, `[length:…]` : étiquettes d'en-tête, sans texte à afficher.
static META: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\[[a-z]{2,}:[^\]]*\]$").expect("regex constante"));
static OFFSET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\[offset:\s*([+-]?[0-9]+)\s*\]").expect("regex constante"));

/// Un mot (ou une syllabe) minuté d'une ligne en LRC enrichi.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Word {
    /// Début, en secondes, décalage compris.
    pub t: f64,
    /// Texte tel qu'il s'affiche, espaces compris : les mots mis bout à bout redonnent [`Line::text`].
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Line {
    /// Début, en secondes ; `None` pour des paroles sans minutage.
    pub t: Option<f64>,
    /// Texte sans repères. Vide dans des paroles minutées : un interlude (trois points à l'écran).
    pub text: String,
    /// Mots minutés, seulement si le fichier en donne (`<mm:ss.xx>`).
    pub words: Vec<Word>,
}

impl Line {
    /// Ligne vide : un blanc entre deux strophes, ou un interlude dans des paroles minutées.
    pub fn is_gap(&self) -> bool {
        self.text.is_empty()
    }

    /// Morceaux à allumer un à un, et s'il faut une espace entre eux : les mots minutés du fichier (qui portent
    /// déjà leurs espaces), sinon le découpage de [`split_words`].
    pub fn parts(&self) -> (Vec<&str>, bool) {
        if self.words.is_empty() {
            split_words(&self.text)
        } else {
            (self.words.iter().map(|w| w.text.as_str()).collect(), false)
        }
    }
}

/// Paroles analysées.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub struct Lrc {
    /// Vrai si au moins trois repères de temps : les lignes sont triées par début.
    pub synced: bool,
    pub lines: Vec<Line>,
}

/// Espaces au sens de `String.prototype.trim` (qui retire aussi l'indicateur d'ordre des octets U+FEFF).
fn is_js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

fn trim_js(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

fn seconds(c: &Captures<'_>) -> f64 {
    let num = |i: usize| c.get(i).and_then(|m| m.as_str().parse::<f64>().ok()).unwrap_or(0.0);
    // « 5 », « 50 » et « 500 » valent tous une demi-seconde, comme `Number('0.' + frac)` d'origine.
    let frac = c.get(3).and_then(|m| format!("0.{}", m.as_str()).parse::<f64>().ok()).unwrap_or(0.0);
    num(1) * 60.0 + num(2) + frac
}

/// Au moins trois repères de temps : c'est du LRC, pas du texte brut.
pub fn looks_synced(text: &str) -> bool {
    STAMP.find_iter(text).take(3).count() >= 3
}

/// Repère `[mm:ss.cc]` d'un instant en millisecondes.
pub fn format_stamp(ms: u64) -> String {
    format!("[{:02}:{:02}.{:02}]", ms / 60_000, (ms / 1000) % 60, (ms % 1000) / 10)
}

/// Sépare le texte d'une ligne (sans ses repères de ligne) de ses mots minutés.
/// Les temps des mots sont bruts (sans décalage) ; `None` pour un début de ligne non minuté.
fn split_timed(s: &str) -> (String, Vec<(Option<f64>, String)>) {
    let mut text = String::new();
    let mut words: Vec<(Option<f64>, String)> = Vec::new();
    let mut timed = false;
    let mut push = |t: Option<f64>, seg: &str, words: &mut Vec<(Option<f64>, String)>| {
        text.push_str(seg);
        if trim_js(seg).is_empty() {
            // Une espace seule entre deux repères appartient au mot d'avant.
            if let Some(last) = words.last_mut() {
                last.1.push_str(seg);
            }
        } else {
            words.push((t, seg.to_string()));
        }
    };
    let mut last = 0;
    let mut current = None;
    for c in WORD_STAMP.captures_iter(s) {
        let Some(whole) = c.get(0) else { continue };
        push(current, &s[last..whole.start()], &mut words);
        current = Some(seconds(&c));
        timed = true;
        last = whole.end();
    }
    push(current, &s[last..], &mut words);
    if !timed {
        words.clear();
    }
    // Les mots doivent redonner le texte de la ligne, sans ses espaces de bord.
    if let Some(first) = words.first_mut() {
        first.1 = first.1.trim_start_matches(is_js_space).to_string();
    }
    if let Some(last) = words.last_mut() {
        last.1 = last.1.trim_end_matches(is_js_space).to_string();
    }
    (trim_js(&text).to_string(), words)
}

/// Analyse un texte LRC (ou du texte brut).
///
/// Avec au moins trois repères, ne garde que les lignes minutées (une par repère), triées par début ; les lignes
/// vides minutées restent (interludes). Sinon, renvoie toutes les lignes sans repères, avec au plus un blanc entre
/// deux strophes et aucun au début ni à la fin.
pub fn parse(raw: &str) -> Lrc {
    let text = raw.strip_prefix('\u{feff}').unwrap_or(raw).replace("\r\n", "\n").replace('\r', "\n");
    // Le décalage (en ms, positif : paroles plus tôt) vaut pour tout le fichier, où qu'il soit déclaré.
    let offset = text
        .split('\n')
        .find_map(|l| OFFSET.captures(trim_js(l)))
        .and_then(|c| c.get(1)?.as_str().parse::<f64>().ok())
        .map_or(0.0, |ms| ms / 1000.0);

    let mut timed: Vec<Line> = Vec::new();
    let mut all: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let trimmed = trim_js(line);
        if trimmed.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("[offset:")) || META.is_match(trimmed) {
            continue;
        }
        let stamps: Vec<f64> = STAMP.captures_iter(line).map(|c| seconds(&c)).collect();
        let (body, words) = split_timed(&STAMP.replace_all(line, ""));
        if let Some(&first) = stamps.first() {
            for &s in &stamps {
                let t = (s - offset).max(0.0);
                // Une ligne répétée par plusieurs repères garde l'écart de ses mots avec son début.
                let shift = s - first;
                let words = words
                    .iter()
                    .map(|(wt, w)| Word {
                        t: wt.map_or(t, |wt| (wt + shift - offset).max(0.0)),
                        text: w.clone(),
                    })
                    .collect();
                timed.push(Line { t: Some(t), text: body.clone(), words });
            }
        }
        all.push(body);
    }

    if timed.len() >= 3 {
        // Tri stable : deux lignes au même instant gardent l'ordre du fichier.
        timed.sort_by(|a, b| a.t.unwrap_or(0.0).total_cmp(&b.t.unwrap_or(0.0)));
        return Lrc { synced: true, lines: timed };
    }

    let mut lines: Vec<Line> = Vec::new();
    for text in all {
        if text.is_empty() && lines.last().is_none_or(|l| l.text.is_empty()) {
            continue;
        }
        lines.push(Line { t: None, text, words: Vec::new() });
    }
    while lines.last().is_some_and(|l| l.text.is_empty()) {
        lines.pop();
    }
    Lrc { synced: false, lines }
}

/// Index de la ligne en cours à l'instant `time` (secondes) : la dernière qui a commencé. `None` avant la
/// première ligne, ou pour des paroles sans minutage. Recherche dichotomique : les lignes sont triées.
pub fn line_at(lines: &[Line], time: f64) -> Option<usize> {
    lines.partition_point(|l| l.t.is_some_and(|t| t <= time)).checked_sub(1)
}

/// Longueur à la manière de `String.length` en JavaScript (unités UTF-16), pour les mêmes durées que l'original.
fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Écritures sans espaces entre les mots (kana, idéogrammes CJC, hangûl).
fn is_asian(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{9fff}' | '\u{ac00}'..='\u{d7af}')
}

/// Découpe une ligne en mots, ou en caractères pour une écriture sans espaces (un seul « mot » asiatique de plus
/// de quatre caractères). Le booléen dit s'il faut une espace entre les morceaux.
pub fn split_words(text: &str) -> (Vec<&str>, bool) {
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() == 1 && text.chars().any(is_asian) && js_len(text) > 4 {
        return (text.char_indices().map(|(i, c)| &text[i..i + c.len_utf8()]).collect(), false);
    }
    (parts, true)
}

impl Lrc {
    pub fn parse(raw: &str) -> Lrc {
        parse(raw)
    }

    /// Voir [`line_at`] ; toujours `None` pour des paroles sans minutage.
    pub fn line_at(&self, time: f64) -> Option<usize> {
        if self.synced { line_at(&self.lines, time) } else { None }
    }

    /// Y a-t-il quelque chose à afficher (pas seulement des interludes) ?
    pub fn has_text(&self) -> bool {
        self.lines.iter().any(|l| !l.text.is_empty())
    }

    /// Début de la ligne suivante (la première qui commence strictement après la ligne `i`).
    pub fn next_start(&self, i: usize) -> Option<f64> {
        let t = self.lines.get(i)?.t?;
        self.lines.get(i + 1..)?.iter().filter_map(|l| l.t).find(|&n| n > t)
    }

    /// Nombre de morceaux de [`Line::parts`] à allumer dans la ligne `i` à l'instant `time`.
    ///
    /// Avec des mots minutés : ceux qui ont commencé. Sinon, une estimation (celle du mode immersif d'origine) :
    /// la ligne se lit en 0,07 s par caractère (au moins 1,1 s), sans déborder sur la ligne suivante, et chaque
    /// mot s'allume à proportion de sa longueur.
    pub fn lit_words(&self, i: usize, time: f64) -> usize {
        let Some(line) = self.lines.get(i) else { return 0 };
        let Some(t) = line.t else { return 0 };
        if !line.words.is_empty() {
            return line.words.iter().filter(|w| w.t <= time + 1e-6).count();
        }
        let (parts, _) = split_words(&line.text);
        let total = parts.iter().map(|p| js_len(p)).sum::<usize>().max(1) as f64;
        let room = self.next_start(i).unwrap_or(t + 5.0) - t - 0.15;
        let dur = room.min((js_len(&line.text) as f64 * 0.07).max(1.1)).max(0.5);
        let p = ((time - t) / dur).clamp(0.0, 1.0);
        let mut acc = 0usize;
        let mut lit = 0;
        for part in parts {
            if acc as f64 / total > p + 1e-6 {
                break;
            }
            lit += 1;
            acc += js_len(part);
        }
        lit
    }
}

#[cfg(test)]
mod tests;

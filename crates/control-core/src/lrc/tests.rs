use super::*;

fn times(l: &Lrc) -> Vec<f64> {
    l.lines.iter().map(|x| x.t.unwrap_or(-1.0)).collect()
}

#[track_caller]
fn assert_times(l: &Lrc, want: &[f64]) {
    let got = times(l);
    assert!(got.len() == want.len() && got.iter().zip(want).all(|(a, b)| close(*a, *b)), "{got:?} != {want:?}");
}

fn texts(l: &Lrc) -> Vec<&str> {
    l.lines.iter().map(|x| x.text.as_str()).collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[track_caller]
fn assert_words(line: &Line, want: &[(f64, &str)]) {
    assert_eq!(line.words.len(), want.len(), "{:?}", line.words);
    for (w, (t, text)) in line.words.iter().zip(want) {
        assert!(close(w.t, *t) && w.text == *text, "{w:?} != {t} {text:?}");
    }
}

#[test]
fn basic_synced() {
    let l = parse("[00:01.00]Un\n[00:02.50]Deux\n[01:03.00]Trois");
    assert!(l.synced);
    assert_eq!(texts(&l), ["Un", "Deux", "Trois"]);
    assert_times(&l, &[1.0, 2.5, 63.0]);
}

#[test]
fn fractions_like_the_original() {
    // Fraction lue comme « 0.<chiffres> » : 1, 2 ou 3 chiffres, séparateur point ou deux-points.
    let l = parse("[00:01.5]a\n[00:01.50]b\n[00:01.500]c\n[00:02.05]d\n[00:03:25]e\n[00:04]f\n[100:00.00]g");
    assert_times(&l, &[1.5, 1.5, 1.5, 2.05, 3.25, 4.0, 6000.0]);
}

#[test]
fn multiple_stamps_per_line_are_sorted() {
    let l = parse("[00:10.00][00:30.00]Refrain\n[00:20.00]Couplet\n[00:05.00]Intro");
    assert!(l.synced);
    assert_times(&l, &[5.0, 10.0, 20.0, 30.0]);
    assert_eq!(texts(&l), ["Intro", "Refrain", "Couplet", "Refrain"]);
}

#[test]
fn equal_times_keep_file_order() {
    let l = parse("[00:01.00]a\n[00:01.00]b\n[00:01.00]c\n[00:00.50]z");
    assert_eq!(texts(&l), ["z", "a", "b", "c"]);
}

#[test]
fn offset_applies_everywhere_and_clamps_at_zero() {
    // Déclaré après les lignes : vaut quand même pour tout le fichier.
    let l = parse("[00:00.20]a\n[00:02.00]b\n[00:03.00]c\n[offset:+500]");
    assert_times(&l, &[0.0, 1.5, 2.5]);
    let l = parse("[OFFSET: -1000 ]\n[00:01.00]a\n[00:02.00]b\n[00:03.00]c");
    assert_times(&l, &[2.0, 3.0, 4.0]);
    // Seul le premier décalage compte, et sa ligne n'est jamais affichée.
    let l = parse("[offset:1000]\n[offset:5000]\n[00:01.00]a\n[00:02.00]b\n[00:03.00]c");
    assert_times(&l, &[0.0, 1.0, 2.0]);
    assert_eq!(texts(&l), ["a", "b", "c"]);
}

#[test]
fn metadata_lines_are_skipped() {
    let raw =
        "[ar:Artiste]\n[ti:Titre]\n[al:Album]\n[length: 03:20]\n[by:quelqu'un]\n[00:01.00]a\n[00:02.00]b\n[00:03.00]c";
    let l = parse(raw);
    assert_eq!(texts(&l), ["a", "b", "c"]);
    // En texte brut aussi.
    let l = parse("[ar:Artiste]\nPremière\nSeconde");
    assert_eq!(texts(&l), ["Première", "Seconde"]);
    // Un crochet dans les paroles n'est pas une étiquette : il lui faut des lettres, deux-points, et rien après.
    let l = parse("[Refrain]\n[x:y]\n[ab:cd] suite");
    assert_eq!(texts(&l), ["[Refrain]", "[x:y]", "[ab:cd] suite"]);
}

#[test]
fn blank_timed_lines_are_interludes() {
    let l = parse("[00:01.00]a\n[00:05.00]\n[00:09.00]  \n[00:12.00]b");
    assert!(l.synced);
    assert_eq!(texts(&l), ["a", "", "", "b"]);
    assert!(l.lines[1].is_gap() && !l.lines[0].is_gap());
    assert!(l.has_text());
    // Lignes sans repère : ignorées dans des paroles minutées.
    let l = parse("Titre\n[00:01.00]a\n\n[00:02.00]b\n[00:03.00]c");
    assert_eq!(texts(&l), ["a", "b", "c"]);
}

#[test]
fn bom_and_line_endings() {
    let l = parse("\u{feff}[00:01.00]a\r\n[00:02.00]b\r[00:03.00]c\r\n");
    assert!(l.synced);
    assert_eq!(texts(&l), ["a", "b", "c"]);
}

#[test]
fn plain_text_cleanup() {
    let l = parse("\n\n  Une  \n\n\n\nDeux\n\nTrois\n\n\n");
    assert!(!l.synced);
    assert_eq!(texts(&l), ["Une", "", "Deux", "", "Trois"]);
    assert!(l.lines.iter().all(|x| x.t.is_none() && x.words.is_empty()));
    assert_eq!(parse(""), Lrc { synced: false, lines: vec![] });
    assert_eq!(parse("\n \n\u{feff}"), Lrc { synced: false, lines: vec![] });
}

#[test]
fn fewer_than_three_stamps_falls_back_to_text_in_order() {
    let l = parse("Intro\n[00:01.00]Un\nMilieu\n[00:02.00]Deux");
    assert!(!l.synced);
    assert_eq!(texts(&l), ["Intro", "Un", "Milieu", "Deux"]);
    let l = parse("[00:01.00][00:02.00]Seul");
    assert!(!l.synced);
    assert_eq!(texts(&l), ["Seul"]);
}

#[test]
fn stamps_anywhere_in_the_line() {
    let l = parse("Un [00:01.00]\n[00:02.00] Deux [00:04.00]\n[00:03.00]Trois");
    assert_times(&l, &[1.0, 2.0, 3.0, 4.0]);
    assert_eq!(texts(&l), ["Un", "Deux", "Trois", "Deux"]);
}

#[test]
fn non_latin_digits_are_not_stamps() {
    // `\d` de Rust reconnaîtrait « ٠١ » ; le JavaScript d'origine non.
    assert!(!looks_synced("[٠١:٠٢]a\n[٠١:٠٣]b\n[٠١:٠٤]c"));
    assert!(!parse("[٠١:٠٢]a\n[٠١:٠٣]b\n[٠١:٠٤]c").synced);
}

#[test]
fn looks_synced_needs_three_stamps() {
    assert!(!looks_synced("[00:01.00]a\n[00:02.00]b"));
    assert!(looks_synced("[00:01.00]a\n[00:02.00]b\n[00:03]c"));
    assert!(looks_synced("[00:01.00][00:02.00][00:03.00]a"));
    assert!(!looks_synced("[0:1]a [00:2] [000:03.0000]"));
}

#[test]
fn enhanced_word_timing() {
    let l = parse("[00:10.00]<00:10.00>Bonjour <00:10.80>le <00:11.20>monde\n[00:12.00]b\n[00:13.00]c");
    assert!(l.synced);
    let line = &l.lines[0];
    assert_eq!(line.text, "Bonjour le monde");
    assert_words(line, &[(10.0, "Bonjour "), (10.8, "le "), (11.2, "monde")]);
    assert_eq!(line.words.iter().map(|w| w.text.as_str()).collect::<String>(), line.text);
    assert!(l.lines[1].words.is_empty());
    assert_eq!(line.parts(), (vec!["Bonjour ", "le ", "monde"], false));
}

#[test]
fn enhanced_syllables_and_lone_spaces() {
    // Syllabes collées, espace seule entre deux repères, repère de fin sans texte, début non minuté.
    let l = parse("[00:01.00]Je <00:01.50>chan<00:01.70>te<00:02.00> <00:02.20>fort<00:03.00>\n[00:04]b\n[00:05]c");
    let line = &l.lines[0];
    assert_eq!(line.text, "Je chante fort");
    assert_words(line, &[(1.0, "Je "), (1.5, "chan"), (1.7, "te "), (2.2, "fort")]);
    assert_eq!(line.words.iter().map(|w| w.text.as_str()).collect::<String>(), line.text);
}

#[test]
fn enhanced_words_follow_offset_and_repeats() {
    let l = parse("[offset:500]\n[00:10.00][00:40.00]<00:10.00>la <00:11.00>la\n[00:20.00]b");
    assert!(l.synced);
    assert_times(&l, &[9.5, 19.5, 39.5]);
    let first: Vec<f64> = l.lines[0].words.iter().map(|w| w.t).collect();
    let repeat: Vec<f64> = l.lines[2].words.iter().map(|w| w.t).collect();
    assert!(close(first[0], 9.5) && close(first[1], 10.5));
    assert!(close(repeat[0], 39.5) && close(repeat[1], 40.5));
}

#[test]
fn word_stamps_in_plain_lines_are_removed() {
    let l = parse("<00:01.00>Mot <00:02.00>minuté");
    assert!(!l.synced);
    assert_eq!(texts(&l), ["Mot minuté"]);
    assert!(l.lines[0].words.is_empty());
}

#[test]
fn line_at_binary_search() {
    let l = parse("[00:01.00]a\n[00:02.00]b\n[00:02.00]b2\n[00:04.00]c");
    assert_eq!(l.line_at(0.0), None);
    assert_eq!(l.line_at(0.999), None);
    assert_eq!(l.line_at(1.0), Some(0));
    assert_eq!(l.line_at(1.5), Some(0));
    // Deux lignes au même instant : la dernière l'emporte, comme l'original.
    assert_eq!(l.line_at(2.0), Some(2));
    assert_eq!(l.line_at(3.99), Some(2));
    assert_eq!(l.line_at(4.0), Some(3));
    assert_eq!(l.line_at(1e9), Some(3));
    assert_eq!(l.line_at(f64::NAN), None);
    assert_eq!(line_at(&[], 3.0), None);
    // Sans minutage : jamais de ligne en cours.
    assert_eq!(parse("a\nb").line_at(10.0), None);
}

#[test]
fn line_at_large_input() {
    let raw: String = (0..5000).map(|i| format!("[{:02}:{:02}.00]l{i}\n", i / 60, i % 60)).collect();
    let l = parse(&raw);
    assert_eq!(l.lines.len(), 5000);
    for i in [0usize, 1, 59, 60, 2500, 4999] {
        assert_eq!(l.line_at(i as f64 + 0.5), Some(i));
    }
}

#[test]
fn next_start_skips_simultaneous_lines() {
    let l = parse("[00:01.00]a\n[00:01.00]b\n[00:03.00]c");
    assert_eq!(l.next_start(0), Some(3.0));
    assert_eq!(l.next_start(2), None);
    assert_eq!(l.next_start(9), None);
}

#[test]
fn split_words_spaces_and_asian_scripts() {
    assert_eq!(split_words("  un   deux trois "), (vec!["un", "deux", "trois"], true));
    assert_eq!(split_words("君の名は何"), (vec!["君", "の", "名", "は", "何"], false));
    // Quatre caractères ou moins, ou plusieurs mots : découpage par espaces.
    assert_eq!(split_words("君の名は"), (vec!["君の名は"], true));
    assert_eq!(split_words("사랑해 너를"), (vec!["사랑해", "너를"], true));
    assert_eq!(split_words("abcdefgh"), (vec!["abcdefgh"], true));
    assert_eq!(split_words(""), (vec![], true));
}

#[test]
fn lit_words_estimate_matches_immersive_mode() {
    // « aa bb cc dd » : 8 lettres, débuts à 0, 1/4, 1/2, 3/4. Durée : max(1,1 ; 11 × 0,07 = 0,77) = 1,1 s.
    let l = parse("[00:10.00]aa bb cc dd\n[00:20.00]x\n[00:30.00]y");
    assert_eq!(l.lit_words(0, 9.0), 1); // avant le début : p = 0, le premier mot est allumé
    assert_eq!(l.lit_words(0, 10.0), 1);
    assert_eq!(l.lit_words(0, 10.0 + 1.1 * 0.26), 2);
    assert_eq!(l.lit_words(0, 10.0 + 1.1 * 0.5), 3);
    assert_eq!(l.lit_words(0, 10.0 + 1.1 * 0.76), 4);
    assert_eq!(l.lit_words(0, 100.0), 4);
    assert_eq!(l.lit_words(9, 100.0), 0);
    assert_eq!(parse("a b").lit_words(0, 1.0), 0);
}

#[test]
fn lit_words_estimate_is_bounded_by_next_line() {
    // Ligne longue (60 caractères → 4,2 s) mais la suivante arrive 1 s après : durée 0,85 s.
    let text = "a".repeat(29) + " " + &"b".repeat(30);
    let l = parse(&format!("[00:00.00]{text}\n[00:01.00]x\n[00:02.00]y"));
    assert_eq!(l.lit_words(0, 0.40), 1);
    assert_eq!(l.lit_words(0, 0.43), 2);
    // Sans ligne suivante : 5 s de place.
    let l = parse("[00:00.00]x\n[00:01.00]y\n[00:02.00]un deux");
    assert_eq!(l.lit_words(2, 2.0 + 1.1 * 0.3), 1);
    assert_eq!(l.lit_words(2, 2.0 + 1.1 * 0.34), 2);
}

#[test]
fn lit_words_with_real_timing() {
    let l = parse("[00:10.00]<00:10.00>a <00:10.50>b <00:12.00>c\n[00:20.00]x\n[00:30.00]y");
    assert_eq!(l.lit_words(0, 9.9), 0);
    assert_eq!(l.lit_words(0, 10.0), 1);
    assert_eq!(l.lit_words(0, 11.9), 2);
    assert_eq!(l.lit_words(0, 12.0), 3);
}

#[test]
fn format_stamp_round_trips() {
    assert_eq!(format_stamp(0), "[00:00.00]");
    assert_eq!(format_stamp(63_456), "[01:03.45]");
    assert_eq!(format_stamp(6_000_000), "[100:00.00]");
    let raw: String = [1_000u64, 62_500, 3_599_990].iter().map(|&ms| format_stamp(ms) + "x\n").collect();
    assert_times(&parse(&raw), &[1.0, 62.5, 3599.99]);
}

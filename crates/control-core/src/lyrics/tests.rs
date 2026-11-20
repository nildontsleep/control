use std::collections::VecDeque;

use lofty::TextEncoding;
use lofty::config::WriteOptions;
use lofty::id3::v2::{
    BinaryFrame, Frame, FrameId, Id3v2Tag, SyncTextContentType, SynchronizedTextFrame, TimestampFormat,
    UnsynchronizedTextFrame,
};
use lofty::prelude::TagExt;
use parking_lot::Mutex;

use super::*;
use crate::db;

// ─── Réponses enregistrées de LRCLIB ─────────────────────────────────────────

const GET_HIT: &str = r#"{"id":3396226,"trackName":"I Want to Live","artistName":"Borislav Slavov","albumName":"Baldur's Gate 3 (Original Game Soundtrack)","duration":233,"instrumental":false,"plainLyrics":"I feel your breath upon my neck\nA soft caress as cold as death","syncedLyrics":"[00:17.12] I feel your breath upon my neck\n[00:20.41] A soft caress as cold as death\n[00:25.00] "}"#;

const SEARCH: &str = r#"[
  {"id":1,"trackName":"Song","artistName":"Aya","duration":300.0,"instrumental":false,"plainLyrics":"loin","syncedLyrics":"[00:01.00]loin\n[00:02.00]b\n[00:03.00]c"},
  {"id":2,"trackName":"Song","artistName":"Aya","duration":181.0,"instrumental":false,"plainLyrics":"texte seul","syncedLyrics":null},
  {"id":3,"trackName":"Song","artistName":"Aya","duration":183.0,"instrumental":false,"plainLyrics":"proche","syncedLyrics":"[00:01.00]proche\n[00:02.00]b\n[00:03.00]c"},
  {"id":4,"trackName":"Song","artistName":"Aya","duration":180.4,"instrumental":false,"plainLyrics":null,"syncedLyrics":""},
  "pas une fiche",
  {"id":5,"trackName":"Song","artistName":"Aya","duration":180.0,"instrumental":false,"plainLyrics":"exacte","syncedLyrics":"[00:01.00]exacte\n[00:02.00]b\n[00:03.00]c"}
]"#;

fn rec(synced: Option<&str>, plain: Option<&str>, duration: f64) -> Record {
    Record { synced: synced.map(Into::into), plain: plain.map(Into::into), instrumental: false, duration }
}

// ─── Transport factice ───────────────────────────────────────────────────────

type Reply = Result<(u16, String), String>;

struct Fake {
    db: SharedDb,
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<String>>,
}

impl Http for Fake {
    fn get(&self, url: &str) -> Reply {
        // La base ne doit jamais rester verrouillée pendant un appel réseau.
        assert!(self.db.try_lock().is_some(), "base verrouillée pendant un appel à LRCLIB");
        self.calls.lock().push(url.to_string());
        self.replies.lock().pop_front().unwrap_or_else(|| Err("aucune réponse prévue".into()))
    }
}

struct Env {
    lyrics: Lyrics,
    fake: Arc<Fake>,
    db: SharedDb,
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        let db: SharedDb = Arc::new(Mutex::new(db::open_in_memory().unwrap()));
        let fake =
            Arc::new(Fake { db: db.clone(), replies: Mutex::new(VecDeque::new()), calls: Mutex::new(Vec::new()) });
        let mut lyrics = Lyrics::with_http(db.clone(), fake.clone(), "https://lrclib.test/api/");
        lyrics.set_retry_delay(Duration::ZERO);
        Env { lyrics, fake, db, dir: tempfile::tempdir().unwrap() }
    }

    /// Ajoute un morceau (le fichier n'existe que si on l'écrit).
    fn track(&self, name: &str, title: &str, artist: &str, duration: f64) -> (TrackId, PathBuf) {
        let path = self.dir.path().join(name);
        let db = self.db.lock();
        db.execute(
            "INSERT INTO tracks (path, title, artist, album, duration, added_at) VALUES (?, ?, ?, 'Album', ?, 1)",
            params![path.to_string_lossy(), title, artist, duration],
        )
        .unwrap();
        (db.last_insert_rowid(), path)
    }

    fn reply(&self, status: u16, body: &str) {
        self.fake.replies.lock().push_back(Ok((status, body.to_string())));
    }

    fn calls(&self) -> Vec<String> {
        std::mem::take(&mut *self.fake.calls.lock())
    }

    fn stored(&self, id: TrackId) -> Option<Row> {
        self.lyrics.row(id).unwrap()
    }
}

// ─── Fonctions pures ─────────────────────────────────────────────────────────

#[test]
fn classify_text() {
    assert_eq!(classify(""), None);
    assert_eq!(classify(" \n\t "), None);
    assert_eq!(classify("\u{feff}  "), None);
    assert_eq!(classify("  Bonjour\nMonde \n"), Some(Text::Plain("Bonjour\nMonde".into())));
    assert_eq!(classify("[00:01.00]a\n[00:02.00]b"), Some(Text::Plain("[00:01.00]a\n[00:02.00]b".into())));
    assert_eq!(
        classify("\u{feff}[00:01.00]a\n[00:02.00]b\n[00:03.00]c\n"),
        Some(Text::Synced("[00:01.00]a\n[00:02.00]b\n[00:03.00]c".into()))
    );
}

#[test]
fn query_seconds_round_like_math_round() {
    let q = |d| Query { duration: d, ..Query::default() };
    assert_eq!(q(199.5).seconds(), 200);
    assert_eq!(q(199.49).seconds(), 199);
    assert_eq!(q(0.0).seconds(), 0);
    assert_eq!(q(f64::NAN).seconds(), 0);
    assert_eq!(q(-3.0).seconds(), 0);
}

#[test]
fn urls() {
    let q = Query { title: "Ça va & toi?".into(), artist: "Aya Nakamura".into(), album: "".into(), duration: 180.4 };
    assert_eq!(
        exact_url("https://x/api", &q).unwrap(),
        "https://x/api/get?track_name=%C3%87a+va+%26+toi%3F&artist_name=Aya+Nakamura&duration=180"
    );
    let full = Query { album: "Nakamura".into(), duration: 0.0, ..q.clone() };
    assert_eq!(
        exact_url("https://x/api", &full).unwrap(),
        "https://x/api/get?track_name=%C3%87a+va+%26+toi%3F&artist_name=Aya+Nakamura&album_name=Nakamura"
    );
    assert_eq!(
        search_url("https://x/api", &q),
        "https://x/api/search?track_name=%C3%87a+va+%26+toi%3F&artist_name=Aya+Nakamura"
    );
    let anon = Query { artist: String::new(), ..q.clone() };
    assert_eq!(exact_url("https://x/api", &anon), None);
    assert_eq!(search_url("https://x/api", &anon), "https://x/api/search?q=%C3%87a+va+%26+toi%3F");
    assert_eq!(exact_url("https://x/api", &Query { title: String::new(), ..q }), None);
}

#[test]
fn parse_get_response() {
    let hit = parse_get(GET_HIT).unwrap().unwrap();
    assert!(hit.synced.as_deref().unwrap().starts_with("[00:17.12] I feel"));
    assert!(hit.plain.as_deref().unwrap().starts_with("I feel"));
    assert!(!hit.instrumental && hit.has_lyrics());
    assert_eq!(hit.duration, 233.0);

    let inst =
        parse_get(r#"{"id":9,"instrumental":true,"plainLyrics":null,"syncedLyrics":null,"duration":null}"#).unwrap();
    assert_eq!(inst, Some(Record { synced: None, plain: None, instrumental: true, duration: 0.0 }));
    assert!(inst.unwrap().has_lyrics());

    // Champs vides ou d'un autre type : comme absents.
    let empty = parse_get(r#"{"plainLyrics":"","syncedLyrics":42,"instrumental":"oui"}"#).unwrap().unwrap();
    assert!(!empty.has_lyrics());

    assert_eq!(parse_get("null").unwrap(), None);
    assert_eq!(parse_get("[1]").unwrap(), None);
    assert!(parse_get("<html>").is_err());
}

#[test]
fn parse_search_response() {
    let list = parse_search(SEARCH).unwrap();
    assert_eq!(list.len(), 5); // la chaîne isolée est ignorée
    assert_eq!(list[3], rec(None, None, 180.4));
    assert_eq!(parse_search(r#"{"message":"x"}"#).unwrap(), vec![]);
    assert_eq!(parse_search("[]").unwrap(), vec![]);
    assert!(parse_search("").is_err());
}

#[test]
fn pick_prefers_synced_then_closest_duration() {
    let best = pick(parse_search(SEARCH).unwrap(), 180).unwrap();
    assert_eq!(best.plain.as_deref(), Some("exacte"));
    // À 300 s, seule la première fiche est assez proche.
    assert_eq!(pick(parse_search(SEARCH).unwrap(), 300).unwrap().plain.as_deref(), Some("loin"));
    // Durée inconnue : pas de filtre ; les minutées d'abord, puis l'écart à 0 (comme l'original).
    let list = vec![rec(None, Some("brut"), 10.0), rec(Some("[1:00]"), None, 50.0), rec(Some("[2:00]"), None, 20.0)];
    assert_eq!(pick(list, 0).unwrap().synced.as_deref(), Some("[2:00]"));
    // À égalité, l'ordre de LRCLIB.
    let list = vec![rec(Some("[1:00]"), None, 182.0), rec(Some("[2:00]"), None, 178.0)];
    assert_eq!(pick(list, 180).unwrap().synced.as_deref(), Some("[1:00]"));
    // Une minutée sans texte ne gagne pas sur une brute.
    let list = vec![rec(None, None, 180.0), rec(None, Some("brut"), 182.0)];
    assert_eq!(pick(list, 180).unwrap().plain.as_deref(), Some("brut"));
    // Trop loin (plus de 4 s), ou rien d'utile.
    assert_eq!(pick(vec![rec(None, Some("x"), 185.0)], 180), None);
    assert_eq!(pick(vec![rec(None, None, 180.0)], 180), None);
    assert_eq!(pick(vec![], 180), None);
    let inst = Record { instrumental: true, ..rec(None, None, 176.0) };
    assert_eq!(pick(vec![inst.clone()], 180), Some(inst));
}

#[test]
fn fetch_retries_when_busy() {
    let env = Env::new();
    env.reply(503, "");
    env.reply(429, "");
    env.reply(200, "[]");
    let got = lrclib::fetch(env.fake.as_ref(), "u", Duration::ZERO).unwrap();
    assert_eq!(got.as_deref(), Some("[]"));
    assert_eq!(env.calls().len(), 3);

    for _ in 0..3 {
        env.reply(502, "");
    }
    let err = lrclib::fetch(env.fake.as_ref(), "u", Duration::ZERO).unwrap_err();
    assert!(matches!(err, LyricsError::Busy(502)) && err.code() == Some("busy"));
    assert_eq!(env.calls().len(), 3);

    // Erreur réseau puis réponse : réessayé ; toujours en erreur : code « network ».
    env.fake.replies.lock().push_back(Err("connexion refusée".into()));
    env.reply(404, "");
    assert_eq!(lrclib::fetch(env.fake.as_ref(), "u", Duration::ZERO).unwrap(), None);
    let err = lrclib::fetch(env.fake.as_ref(), "u", Duration::ZERO).unwrap_err();
    assert_eq!(err.code(), Some("network"));
    env.calls();

    // Autre erreur HTTP : pas de nouvel essai.
    env.reply(400, "");
    let err = lrclib::fetch(env.fake.as_ref(), "u", Duration::ZERO).unwrap_err();
    assert!(matches!(err, LyricsError::Http(400)) && err.code() == Some("http"));
    assert_eq!(env.calls().len(), 1);
}

#[test]
fn sylt_conversion() {
    let c = |v: &[(u32, &str)]| v.iter().map(|(t, s)| (*t, s.to_string())).collect::<Vec<_>>();
    // Une entrée par ligne ; une entrée sur deux lignes donne deux lignes au même instant.
    assert_eq!(
        sylt_to_lrc(&c(&[(1000, "Un"), (62_500, " Deux "), (63_000, "Trois\nQuatre"), (64_000, "")])),
        "[00:01.00]Un\n[01:02.50]Deux\n[01:03.00]Trois\n[01:03.00]Quatre\n[01:04.00]"
    );
    // Une entrée par syllabe, un saut de ligne en tête ouvre une ligne.
    let lrc = sylt_to_lrc(&c(&[(1000, "Je "), (1500, "chan"), (1700, "te"), (3000, "\nFort "), (3400, "et clair")]));
    assert_eq!(lrc, "[00:01.00]<00:01.00>Je <00:01.50>chan<00:01.70>te\n[00:03.00]<00:03.00>Fort <00:03.40>et clair");
    let parsed = crate::lrc::parse(&(lrc + "\n[00:05.00]fin"));
    assert_eq!(parsed.lines[0].text, "Je chante");
    assert_eq!(parsed.lines[0].words.len(), 3);
    assert_eq!(sylt_to_lrc(&[]), "");
}

#[test]
fn result_json_matches_the_original() {
    let r = Row::new(Some("[00:01]a".into()), Some(String::new()), Source::Lrclib, false).result();
    assert_eq!(
        serde_json::to_value(&r).unwrap(),
        serde_json::json!({"status": "ok", "source": "lrclib", "synced": "[00:01]a", "plain": null, "instrumental": false})
    );
    let r = Row::new(None, None, Source::None, false).result();
    assert_eq!(serde_json::to_value(&r).unwrap()["status"], "none");
    let r = Row::new(None, None, Source::Lrclib, true).result();
    assert_eq!((r.status, r.instrumental), (Status::Ok, true));
    for s in [Source::File, Source::Tags, Source::Lrclib, Source::Manual, Source::None] {
        assert_eq!(Source::parse(s.as_str()), s);
    }
}

// ─── Recherche complète ──────────────────────────────────────────────────────

#[test]
fn online_exact_hit_is_cached() {
    let env = Env::new();
    let (id, _) = env.track("a.mp3", "I Want to Live", "Borislav Slavov", 233.2);
    env.reply(200, GET_HIT);
    let r = env.lyrics.get(id, false).unwrap();
    assert_eq!((r.status, r.source), (Status::Ok, Source::Lrclib));
    assert!(r.synced.is_some() && r.plain.is_some());
    assert!(r.parsed().synced);
    let calls = env.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].starts_with("https://lrclib.test/api/get?track_name=I+Want+to+Live&artist_name=Borislav+Slavov&album_name=Album&duration=233"));
    // Deuxième demande : depuis la base, sans réseau.
    assert_eq!(env.lyrics.get(id, false).unwrap(), r);
    assert!(env.calls().is_empty());
    // `refresh` repasse par LRCLIB.
    env.reply(404, "");
    env.reply(200, "[]");
    let r = env.lyrics.get(id, true).unwrap();
    assert_eq!((r.status, r.source), (Status::None, Source::None));
    assert_eq!(env.calls().len(), 2);
}

#[test]
fn falls_back_to_search() {
    let env = Env::new();
    let (id, _) = env.track("a.mp3", "Song", "Aya", 180.2);
    env.reply(200, r#"{"instrumental":false,"plainLyrics":null,"syncedLyrics":null}"#);
    env.reply(200, SEARCH);
    let r = env.lyrics.get(id, false).unwrap();
    assert_eq!(r.plain.as_deref(), Some("exacte"));
    let calls = env.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1], "https://lrclib.test/api/search?track_name=Song&artist_name=Aya");

    // Sans artiste : directement la recherche libre.
    let (id, _) = env.track("b.mp3", "Song", "", 0.0);
    env.reply(200, SEARCH);
    // Durée inconnue : pas de filtre, et la plus courte des minutées l'emporte (écart à 0), comme l'original.
    assert_eq!(env.lyrics.get(id, false).unwrap().plain.as_deref(), Some("exacte"));
    assert_eq!(env.calls(), ["https://lrclib.test/api/search?q=Song"]);
}

#[test]
fn negative_cache_expires_after_three_days() {
    let env = Env::new();
    let (id, _) = env.track("a.mp3", "Rien", "Personne", 100.0);
    env.reply(404, "");
    env.reply(404, "");
    let r = env.lyrics.get(id, false).unwrap();
    assert_eq!((r.status, r.source), (Status::None, Source::None));
    assert_eq!(env.stored(id).unwrap().source, Source::None);
    env.calls();
    assert_eq!(env.lyrics.get(id, false).unwrap().status, Status::None);
    assert!(env.calls().is_empty());

    let old = now_ms() - RETRY_NONE_AFTER_MS - 1000;
    env.db.lock().execute("UPDATE lyrics SET fetched_at = ? WHERE track_id = ?", params![old, id]).unwrap();
    env.reply(200, GET_HIT);
    assert_eq!(env.lyrics.get(id, false).unwrap().source, Source::Lrclib);
    assert_eq!(env.calls().len(), 1);
}

#[test]
fn errors_are_not_cached() {
    let env = Env::new();
    let (id, _) = env.track("a.mp3", "T", "A", 100.0);
    for _ in 0..3 {
        env.reply(503, "");
    }
    let err = env.lyrics.get(id, false).unwrap_err();
    assert_eq!(err.code(), Some("busy"));
    assert_eq!(err.to_string(), "LRCLIB 503");
    assert_eq!(env.stored(id), None);
    env.reply(200, "<html>");
    assert_eq!(env.lyrics.get(id, false).unwrap_err().code(), Some("network"));
    assert!(matches!(env.lyrics.get(999, false), Err(LyricsError::TrackNotFound)));
    assert_eq!(LyricsError::TrackNotFound.to_string(), "Morceau introuvable.");
}

#[test]
fn sidecar_always_wins() {
    let env = Env::new();
    let (id, path) = env.track("Morceau.v2.flac", "T", "A", 100.0);
    env.lyrics.set(id, "à la main").unwrap();
    std::fs::write(env.dir.path().join("Morceau.v2.lrc"), "\u{feff}[00:01.00]a\r\n[00:02.00]b\r\n[00:03.00]c\r\n")
        .unwrap();
    let r = env.lyrics.get(id, false).unwrap();
    assert_eq!(r.source, Source::File);
    assert_eq!(r.synced.as_deref(), Some("[00:01.00]a\r\n[00:02.00]b\r\n[00:03.00]c"));
    assert_eq!(env.stored(id).unwrap().source, Source::File);

    // Un .lrc vide ne compte pas ; le .txt voisin, si.
    std::fs::write(sibling(&path, ".lrc"), "  \n").unwrap();
    std::fs::write(sibling(&path, ".txt"), "texte brut").unwrap();
    let r = env.lyrics.get(id, false).unwrap();
    assert_eq!((r.source, r.plain.as_deref()), (Source::File, Some("texte brut")));

    // Le fichier disparaît : le cache « fichier » ne compte plus, on cherche ailleurs.
    std::fs::remove_file(sibling(&path, ".lrc")).unwrap();
    std::fs::remove_file(sibling(&path, ".txt")).unwrap();
    env.reply(200, GET_HIT);
    assert_eq!(env.lyrics.get(id, false).unwrap().source, Source::Lrclib);
    assert_eq!(env.calls().len(), 1);
}

#[test]
fn set_and_save_sidecar() {
    let env = Env::new();
    let (id, path) = env.track("x.mp3", "T", "A", 100.0);
    assert!(matches!(env.lyrics.save_sidecar(id), Err(LyricsError::NothingToSave)));

    let r = env.lyrics.set(id, "  ligne un\nligne deux  ").unwrap();
    assert_eq!((r.status, r.source, r.plain.as_deref()), (Status::Ok, Source::Manual, Some("ligne un\nligne deux")));
    // Gardées : pas de recherche en ligne ensuite.
    assert_eq!(env.lyrics.get(id, false).unwrap(), r);
    assert!(env.calls().is_empty());

    let target = env.lyrics.save_sidecar(id).unwrap();
    assert_eq!(target, env.dir.path().join("x.txt"));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "ligne un\nligne deux");
    assert_eq!(env.stored(id).unwrap().source, Source::File);

    let r = env.lyrics.set(id, "[00:01.00]a\n[00:02.00]b\n[00:03.00]c").unwrap();
    assert!(r.synced.is_some() && r.plain.is_none());
    assert_eq!(env.lyrics.save_sidecar(id).unwrap(), sibling(&path, ".lrc"));
    assert!(std::fs::read_to_string(sibling(&path, ".lrc")).unwrap().starts_with("[00:01.00]a"));

    // Texte vide : les paroles gardées sont effacées.
    let r = env.lyrics.set(id, " \n ").unwrap();
    assert_eq!(r, LyricsResult::none());
    assert_eq!(env.stored(id), None);

    // Texte trop long : coupé.
    let r = env.lyrics.set(id, &"é".repeat(MAX_MANUAL_CHARS + 10)).unwrap();
    assert_eq!(r.plain.unwrap().chars().count(), MAX_MANUAL_CHARS);
}

// ─── Paroles intégrées ───────────────────────────────────────────────────────

/// Un petit WAV valide (PCM 16 bits, 8 kHz, mono, 0,1 s de silence).
fn write_wav(path: &Path) {
    let data = vec![0u8; 1600];
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&8000u32.to_le_bytes());
    b.extend_from_slice(&16000u32.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data.len() as u32).to_le_bytes());
    b.extend_from_slice(&data);
    std::fs::write(path, b).unwrap();
}

fn uslt(text: &str) -> Frame<'static> {
    Frame::UnsynchronizedText(UnsynchronizedTextFrame::new(TextEncoding::UTF8, *b"fra", String::new(), text.into()))
}

fn sylt(format: TimestampFormat) -> Frame<'static> {
    let content = vec![(1000, "Un".to_string()), (2000, "Deux".to_string()), (3500, "Trois".to_string())];
    let f = SynchronizedTextFrame::new(TextEncoding::UTF8, *b"fra", format, SyncTextContentType::Lyrics, None, content);
    Frame::Binary(BinaryFrame::new(FrameId::Valid("SYLT".into()), f.as_bytes().unwrap()))
}

fn tagged_wav(path: &Path, frames: Vec<Frame<'static>>) {
    write_wav(path);
    let mut tag = Id3v2Tag::new();
    for f in frames {
        tag.insert(f);
    }
    tag.save_to_path(path, WriteOptions::default()).unwrap();
}

#[test]
fn embedded_unsynced_lyrics() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.wav");
    tagged_wav(&p, vec![uslt("  Paroles intégrées\nDeuxième ligne ")]);
    assert_eq!(embedded_lyrics(&p).as_deref(), Some("Paroles intégrées\nDeuxième ligne"));
    // Sans paroles, fichier illisible, ou absent.
    let q = dir.path().join("b.wav");
    write_wav(&q);
    assert_eq!(embedded_lyrics(&q), None);
    std::fs::write(dir.path().join("c.mp3"), b"pas un mp3").unwrap();
    assert_eq!(embedded_lyrics(&dir.path().join("c.mp3")), None);
    assert_eq!(embedded_lyrics(&dir.path().join("absent.flac")), None);
}

#[test]
fn embedded_sylt_beats_plain_uslt() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.wav");
    tagged_wav(&p, vec![uslt("texte brut"), sylt(TimestampFormat::MS)]);
    assert_eq!(embedded_lyrics(&p).as_deref(), Some("[00:01.00]Un\n[00:02.00]Deux\n[00:03.50]Trois"));
    // Repères en trames MPEG : ignorés, le texte brut reste.
    let q = dir.path().join("b.wav");
    tagged_wav(&q, vec![uslt("texte brut"), sylt(TimestampFormat::MPEG)]);
    assert_eq!(embedded_lyrics(&q).as_deref(), Some("texte brut"));
    // Un USLT déjà au format LRC l'emporte.
    let r = dir.path().join("c.wav");
    tagged_wav(&r, vec![uslt("[00:05.00]x\n[00:06.00]y\n[00:07.00]z"), sylt(TimestampFormat::MS)]);
    assert!(embedded_lyrics(&r).unwrap().starts_with("[00:05.00]x"));
}

#[test]
fn get_uses_embedded_lyrics_unless_refreshing() {
    let env = Env::new();
    let (id, path) = env.track("t.wav", "T", "A", 100.0);
    tagged_wav(&path, vec![sylt(TimestampFormat::MS)]);
    let r = env.lyrics.get(id, false).unwrap();
    assert_eq!(r.source, Source::Tags);
    assert!(r.parsed().synced);
    assert!(env.calls().is_empty());
    // `refresh` ignore les étiquettes et cherche en ligne.
    env.reply(200, GET_HIT);
    assert_eq!(env.lyrics.get(id, true).unwrap().source, Source::Lrclib);
}

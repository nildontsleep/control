//! Modèle de la bibliothèque d'un compte pour l'API Subsonic : artistes, albums et genres déduits des tags des
//! morceaux (gardé tant que les morceaux ne changent pas).

use std::collections::{HashMap, HashSet};

use base64::Engine as _;
use serde_json::{Map, Value, json};
use sha1::{Digest, Sha1};

use crate::compare::fold;
use control_core::library::Library;
use control_core::model::{Kind, Track};

/// Identifiant court et stable d'un texte (SHA-1 en base64url, 12 caractères).
pub fn short_hash(s: &str) -> String {
    let digest = Sha1::digest(s.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest).chars().take(12).collect()
}

pub struct Artist {
    pub id: String,
    pub name: String,
    pub albums: Vec<String>,
    pub cover: Option<String>,
}

pub struct Album {
    pub id: String,
    pub name: String,
    pub artist_id: String,
    pub artist: String,
    pub year: Option<i64>,
    pub genre: String,
    pub cover: Option<String>,
    pub tracks: Vec<Track>,
    pub added_at: i64,
    pub last_played: i64,
    pub play_count: i64,
    pub duration: f64,
}

pub struct GenreEntry {
    pub value: String,
    pub song_count: i64,
    pub albums: HashSet<String>,
}

pub struct Model {
    pub rev: u64,
    /// Tous les morceaux (vidéos comprises), dans l'ordre de la base.
    pub songs: Vec<Track>,
    index: HashMap<i64, usize>,
    /// Morceau → (album, artiste).
    pub parents: HashMap<i64, (String, String)>,
    pub artists: Vec<Artist>,
    artist_index: HashMap<String, usize>,
    pub albums: Vec<Album>,
    album_index: HashMap<String, usize>,
    pub genres: Vec<GenreEntry>,
}

/// Les genres d'un morceau (« Pop, Rock » → deux genres).
pub fn genres_of(genre: &str) -> impl Iterator<Item = &str> {
    genre.split([',', ';', '/']).map(str::trim).filter(|g| !g.is_empty())
}

impl Model {
    pub fn build(library: &Library) -> Model {
        let rev = library.rev();
        let songs = library.tracks();
        let mut m = Model {
            rev,
            index: songs.iter().enumerate().map(|(i, t)| (t.id, i)).collect(),
            songs: Vec::new(),
            parents: HashMap::new(),
            artists: Vec::new(),
            artist_index: HashMap::new(),
            albums: Vec::new(),
            album_index: HashMap::new(),
            genres: Vec::new(),
        };
        let mut genre_index: HashMap<String, usize> = HashMap::new();
        for t in &songs {
            if t.kind != Kind::Audio {
                continue;
            }
            let artist_name = if !t.album_artist.is_empty() { &t.album_artist } else { &t.artist };
            let artist_name = if artist_name.is_empty() { "[Artiste inconnu]" } else { artist_name }.trim().to_string();
            let album_name = if t.album.is_empty() { "[Album inconnu]" } else { &t.album }.trim().to_string();
            let artist_id = format!("ar-{}", short_hash(&fold(&artist_name)));
            let album_id = format!("al-{}", short_hash(&format!("{}\0{}", fold(&artist_name), fold(&album_name))));
            let ai = *m.artist_index.entry(artist_id.clone()).or_insert_with(|| {
                m.artists.push(Artist {
                    id: artist_id.clone(),
                    name: artist_name.clone(),
                    albums: Vec::new(),
                    cover: None,
                });
                m.artists.len() - 1
            });
            let li = match m.album_index.get(&album_id) {
                Some(&i) => i,
                None => {
                    m.albums.push(Album {
                        id: album_id.clone(),
                        name: album_name,
                        artist_id: artist_id.clone(),
                        artist: artist_name,
                        year: t.year.filter(|y| *y != 0),
                        genre: t.genre.clone(),
                        cover: t.cover.clone(),
                        tracks: Vec::new(),
                        added_at: 0,
                        last_played: 0,
                        play_count: 0,
                        duration: 0.0,
                    });
                    m.album_index.insert(album_id.clone(), m.albums.len() - 1);
                    m.artists[ai].albums.push(album_id.clone());
                    m.albums.len() - 1
                }
            };
            let album = &mut m.albums[li];
            album.tracks.push(t.clone());
            album.added_at = album.added_at.max(t.added_at);
            album.last_played = album.last_played.max(t.last_played.unwrap_or(0));
            album.play_count += t.play_count;
            album.duration += t.duration;
            if album.cover.is_none() && t.cover.is_some() {
                album.cover.clone_from(&t.cover);
            }
            if album.year.is_none() && t.year.is_some_and(|y| y != 0) {
                album.year = t.year;
            }
            if m.artists[ai].cover.is_none() && album.cover.is_some() {
                m.artists[ai].cover.clone_from(&album.cover);
            }
            m.parents.insert(t.id, (album_id.clone(), artist_id));
            for g in genres_of(&t.genre) {
                let gi = *genre_index.entry(g.to_string()).or_insert_with(|| {
                    m.genres.push(GenreEntry { value: g.to_string(), song_count: 0, albums: HashSet::new() });
                    m.genres.len() - 1
                });
                m.genres[gi].song_count += 1;
                m.genres[gi].albums.insert(album_id.clone());
            }
        }
        let disc = |t: &Track| t.disc_no.filter(|n| *n != 0).unwrap_or(1);
        for album in &mut m.albums {
            album.tracks.sort_by(|a, b| {
                disc(a)
                    .cmp(&disc(b))
                    .then(a.track_no.unwrap_or(0).cmp(&b.track_no.unwrap_or(0)))
                    .then_with(|| fold(&a.title).cmp(&fold(&b.title)))
            });
        }
        m.songs = songs;
        m
    }

    pub fn song(&self, id: i64) -> Option<&Track> {
        self.index.get(&id).map(|&i| &self.songs[i])
    }

    pub fn artist(&self, id: &str) -> Option<&Artist> {
        self.artist_index.get(id).map(|&i| &self.artists[i])
    }

    pub fn album(&self, id: &str) -> Option<&Album> {
        self.album_index.get(id).map(|&i| &self.albums[i])
    }
}

/// `new Date(ms || Date.now()).toISOString()`.
pub fn iso(ms: i64) -> String {
    let ms = if ms == 0 { control_core::util::now_ms() } else { ms };
    let t = time::OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        t.year(),
        t.month() as u8,
        t.day(),
        t.hour(),
        t.minute(),
        t.second(),
        t.millisecond()
    )
}

/// Objet sans les valeurs vides (`undefined`, `null`, `''`), comme `clean()` de l'original.
pub fn clean(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.into_iter().filter(|(_, v)| !v.is_null() && v.as_str() != Some("")).collect::<Map<String, Value>>(),
        ),
        other => other,
    }
}

/// Nombre non nul, sinon rien (`x || undefined`).
pub fn nz(v: Option<i64>) -> Value {
    v.filter(|n| *n != 0).map(Value::from).unwrap_or(Value::Null)
}

pub fn cover_id(hash: Option<&str>) -> Value {
    hash.filter(|h| !h.is_empty()).map(|h| json!(format!("cv-{h}"))).unwrap_or(Value::Null)
}

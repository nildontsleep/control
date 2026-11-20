//! Catalogue dérivé de la bibliothèque : albums, artistes, index de recherche, playlists (dont intelligentes).
//!
//! Construit en une passe à partir des morceaux et playlists de la base, sur un thread de travail ; l'interface le
//! garde derrière un `Arc` et en lit des vues sans rien recalculer.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use crate::artists::{artist_key, split_artists};
use crate::model::{Playlist, PlaylistId, PlaylistItem, PlaylistKind, SmartSpec, Track, TrackId};
use crate::smart;
use crate::util::{fold, natural_cmp};

/// Un morceau et ce qu'on en déduit pour l'affichage et la recherche.
#[derive(Debug, Clone)]
pub struct Entry {
    pub track: Track,
    /// Artistes nommés par l'étiquette (artiste de l'album en priorité), sans doublon ni pseudo.
    pub artist_names: Vec<String>,
    /// Étiquette d'artiste à afficher (« A, B »).
    pub artist_label: String,
    /// Texte replié (sans accents ni casse) pour la recherche.
    pub search: String,
    pub album_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Album {
    pub key: String,
    pub title: String,
    /// Indices dans [`Catalog::entries`], dans l'ordre disque puis piste.
    pub tracks: Vec<usize>,
    pub artist: String,
    /// Clé de l'artiste principal, `None` pour une compilation (« Artistes divers »).
    pub artist_key: Option<String>,
    pub various: bool,
    pub year: Option<i64>,
    pub cover: Option<String>,
    pub added_at: i64,
    pub duration: f64,
    pub play_count: i64,
    pub search: String,
}

#[derive(Debug, Clone)]
pub struct Artist {
    pub key: String,
    pub name: String,
    /// Indices dans [`Catalog::entries`], les plus écoutés d'abord.
    pub tracks: Vec<usize>,
    /// Indices dans [`Catalog::albums`], du plus récent au plus ancien.
    pub albums: Vec<usize>,
    pub cover: Option<String>,
    pub duration: f64,
    pub play_count: i64,
    pub search: String,
}

/// Une ligne de playlist : le morceau, et l'entrée (playlists classiques seulement).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaylistRow {
    pub entry: usize,
    pub item_id: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    pub by_id: HashMap<TrackId, usize>,
    pub audio: Vec<usize>,
    pub videos: Vec<usize>,
    pub albums: Vec<Album>,
    pub album_by_key: HashMap<String, usize>,
    pub artists: Vec<Artist>,
    pub artist_by_key: HashMap<String, usize>,
    pub playlists: Vec<Playlist>,
    pub playlist_items: HashMap<PlaylistId, Vec<PlaylistItem>>,
}

/// Résultats d'une recherche (indices dans le catalogue).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchResults {
    pub tracks: Vec<usize>,
    pub albums: Vec<usize>,
    pub artists: Vec<usize>,
    pub videos: Vec<usize>,
    pub playlists: Vec<usize>,
}

impl SearchResults {
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
            && self.albums.is_empty()
            && self.artists.is_empty()
            && self.videos.is_empty()
            && self.playlists.is_empty()
    }
}

fn dir_of(p: &str) -> &str {
    match p.rfind(['/', '\\']) {
        Some(i) => &p[..i],
        None => "",
    }
}

/// Clé d'un album : son titre et son artiste d'album, à défaut le dossier (deux albums homonymes restent séparés).
pub fn album_key_of(t: &Track) -> Option<String> {
    if t.album.is_empty() {
        return None;
    }
    let who = if t.album_artist.is_empty() { dir_of(&t.path).to_lowercase() } else { fold(&t.album_artist) };
    Some(format!("{}\u{0}{}", fold(&t.album), who))
}

impl Catalog {
    pub fn build(tracks: Vec<Track>, playlists: Vec<Playlist>, items: Vec<PlaylistItem>) -> Catalog {
        let mut c = Catalog { playlists, ..Catalog::default() };
        for item in items {
            c.playlist_items.entry(item.playlist_id).or_default().push(item);
        }
        for list in c.playlist_items.values_mut() {
            list.sort_by_key(|i| (i.position, i.id));
        }
        c.entries = tracks
            .into_iter()
            .map(|t| {
                let main = if t.album_artist.is_empty() { &t.artist } else { &t.album_artist };
                let artist_names = split_artists(main);
                let label = split_artists(&t.artist).join(", ");
                let artist_label = if label.is_empty() { t.artist.clone() } else { label };
                let search = fold(&format!(
                    "{} {} {} {} {} {}",
                    t.title, artist_label, t.artist, t.album, t.album_artist, t.genre
                ));
                let album_key = if t.is_video() { None } else { album_key_of(&t) };
                Entry { track: t, artist_names, artist_label, search, album_key }
            })
            .collect();
        for (i, e) in c.entries.iter().enumerate() {
            c.by_id.insert(e.track.id, i);
            if e.track.is_video() { c.videos.push(i) } else { c.audio.push(i) }
        }
        c.derive_albums();
        c.derive_artists();
        c
    }

    fn derive_albums(&mut self) {
        struct Acc {
            title: String,
            tracks: Vec<usize>,
            artists: HashSet<String>,
            main_name: Option<String>,
            album_artist: String,
            year: Option<i64>,
            cover: Option<String>,
            added_at: i64,
            duration: f64,
            play_count: i64,
        }
        let mut order: Vec<String> = Vec::new();
        let mut accs: HashMap<String, Acc> = HashMap::new();
        for &i in &self.audio {
            let e = &self.entries[i];
            let Some(key) = e.album_key.clone() else { continue };
            let t = &e.track;
            let a = accs.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                Acc {
                    title: t.album.clone(),
                    tracks: Vec::new(),
                    artists: HashSet::new(),
                    main_name: None,
                    album_artist: t.album_artist.clone(),
                    year: t.year,
                    cover: None,
                    added_at: 0,
                    duration: 0.0,
                    play_count: 0,
                }
            });
            a.tracks.push(i);
            a.artists.insert(artist_key(e.artist_names.first().map(String::as_str).unwrap_or("")));
            if a.main_name.is_none() {
                a.main_name = e.artist_names.first().cloned();
            }
            a.duration += t.duration;
            a.play_count += t.play_count;
            a.added_at = a.added_at.max(t.added_at);
            if let Some(y) = t.year
                && a.year.is_none_or(|ay| y < ay)
            {
                a.year = Some(y);
            }
            if a.cover.is_none() && t.cover.is_some() {
                a.cover.clone_from(&t.cover);
            }
        }
        let entries = &self.entries;
        for key in order {
            let Some(mut a) = accs.remove(&key) else { continue };
            a.tracks.sort_by(|&x, &y| {
                let (tx, ty) = (&entries[x].track, &entries[y].track);
                tx.disc_no
                    .unwrap_or(1)
                    .cmp(&ty.disc_no.unwrap_or(1))
                    .then(tx.track_no.unwrap_or(0).cmp(&ty.track_no.unwrap_or(0)))
                    .then_with(|| natural_cmp(&tx.title, &ty.title))
            });
            let various = a.album_artist.is_empty() && a.artists.len() > 1;
            let artist = {
                let joined = split_artists(&a.album_artist).join(", ");
                if !joined.is_empty() {
                    joined
                } else if a.artists.len() == 1 {
                    a.main_name.clone().unwrap_or_default()
                } else {
                    String::new()
                }
            };
            let artist_key = if various {
                None
            } else {
                let base = if a.album_artist.is_empty() { artist.clone() } else { a.album_artist.clone() };
                Some(artist_key(split_artists(&base).first().map(String::as_str).unwrap_or("")))
            };
            let search = fold(&format!("{} {}", a.title, artist));
            self.album_by_key.insert(key.clone(), self.albums.len());
            self.albums.push(Album {
                key,
                title: a.title,
                tracks: a.tracks,
                artist,
                artist_key,
                various,
                year: a.year,
                cover: a.cover,
                added_at: a.added_at,
                duration: a.duration,
                play_count: a.play_count,
                search,
            });
        }
    }

    fn derive_artists(&mut self) {
        let mut order: Vec<String> = Vec::new();
        // Par clé : l'artiste en construction, ses albums (clés) et ses orthographes (nombre d'apparitions).
        type Acc = (Artist, Vec<String>, HashMap<String, usize>);
        let mut accs: HashMap<String, Acc> = HashMap::new();
        for &i in &self.audio {
            let e = &self.entries[i];
            for name in &e.artist_names {
                let key = artist_key(name);
                let (a, album_keys, spellings) = accs.entry(key.clone()).or_insert_with(|| {
                    order.push(key.clone());
                    (
                        Artist {
                            key: key.clone(),
                            name: name.clone(),
                            tracks: Vec::new(),
                            albums: Vec::new(),
                            cover: None,
                            duration: 0.0,
                            play_count: 0,
                            search: String::new(),
                        },
                        Vec::new(),
                        HashMap::new(),
                    )
                });
                *spellings.entry(name.clone()).or_default() += 1;
                a.tracks.push(i);
                a.duration += e.track.duration;
                a.play_count += e.track.play_count;
                if let Some(k) = &e.album_key
                    && !album_keys.contains(k)
                {
                    album_keys.push(k.clone());
                }
                if a.cover.is_none() && e.track.cover.is_some() {
                    a.cover.clone_from(&e.track.cover);
                }
            }
        }
        for key in order {
            let Some((mut a, album_keys, spellings)) = accs.remove(&key) else { continue };
            // L'orthographe la plus courante ; à égalité, la première rencontrée (ordre stable).
            let mut best: Option<(&String, usize)> = None;
            for (name, n) in &spellings {
                if best.is_none_or(|(bn, bc)| *n > bc || (*n == bc && name < bn)) {
                    best = Some((name, *n));
                }
            }
            if let Some((name, _)) = best {
                a.name.clone_from(name);
            }
            let mut albums: Vec<usize> = album_keys.iter().filter_map(|k| self.album_by_key.get(k).copied()).collect();
            albums.sort_by(|&x, &y| {
                let (ax, ay) = (&self.albums[x], &self.albums[y]);
                ay.year.unwrap_or(0).cmp(&ax.year.unwrap_or(0)).then_with(|| natural_cmp(&ax.title, &ay.title))
            });
            a.albums = albums;
            let entries = &self.entries;
            a.tracks.sort_by(|&x, &y| {
                let (tx, ty) = (&entries[x].track, &entries[y].track);
                ty.play_count.cmp(&tx.play_count).then_with(|| natural_cmp(&tx.title, &ty.title))
            });
            a.search = fold(&a.name);
            self.artist_by_key.insert(a.key.clone(), self.artists.len());
            self.artists.push(a);
        }
    }

    pub fn entry(&self, id: TrackId) -> Option<&Entry> {
        self.by_id.get(&id).map(|&i| &self.entries[i])
    }

    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.entry(id).map(|e| &e.track)
    }

    pub fn playlist(&self, id: PlaylistId) -> Option<&Playlist> {
        self.playlists.iter().find(|p| p.id == id)
    }

    /// Morceaux d'une playlist, évaluée si elle est intelligente.
    pub fn playlist_rows(&self, playlist: &Playlist, now_ms: i64) -> Vec<PlaylistRow> {
        if playlist.kind == PlaylistKind::Smart {
            let spec = playlist.rules.clone().unwrap_or_default();
            return self.eval_smart(&spec, now_ms).into_iter().map(|entry| PlaylistRow { entry, item_id: None }).collect();
        }
        self.playlist_items
            .get(&playlist.id)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|it| self.by_id.get(&it.track_id).map(|&entry| PlaylistRow { entry, item_id: Some(it.id) }))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Évalue des règles de playlist intelligente sur les morceaux audio.
    pub fn eval_smart(&self, spec: &SmartSpec, now_ms: i64) -> Vec<usize> {
        let rules: Vec<_> = spec.rules.iter().filter(|r| !r.field.is_empty()).collect();
        let any = spec.match_ == "any";
        let mut list: Vec<usize> = self
            .audio
            .iter()
            .copied()
            .filter(|&i| {
                let t = &self.entries[i].track;
                if rules.is_empty() {
                    true
                } else if any {
                    rules.iter().any(|r| smart::test_rule(t, r, now_ms))
                } else {
                    rules.iter().all(|r| smart::test_rule(t, r, now_ms))
                }
            })
            .collect();
        let key = if spec.sort.key.is_empty() { "title" } else { spec.sort.key.as_str() };
        let desc = spec.sort.dir == "desc";
        if key == "random" {
            use rand::seq::SliceRandom;
            list.shuffle(&mut rand::rng());
        } else {
            let entries = &self.entries;
            list.sort_by(|&a, &b| {
                let ord = smart::compare_by(&entries[a].track, &entries[b].track, key);
                if desc { ord.reverse() } else { ord }
            });
        }
        if spec.limit > 0 {
            list.truncate(spec.limit as usize);
        }
        list
    }

    /// Recherche instantanée : chaque mot doit apparaître (sans accents ni casse).
    pub fn search(&self, query: &str) -> Option<SearchResults> {
        let folded = fold(query);
        let tokens: Vec<&str> = folded.split_whitespace().collect();
        if tokens.is_empty() {
            return None;
        }
        let all = |text: &str| tokens.iter().all(|tok| text.contains(tok));
        Some(SearchResults {
            tracks: self.audio.iter().copied().filter(|&i| all(&self.entries[i].search)).take(200).collect(),
            albums: (0..self.albums.len()).filter(|&i| all(&self.albums[i].search)).take(40).collect(),
            artists: (0..self.artists.len()).filter(|&i| all(&self.artists[i].search)).take(40).collect(),
            videos: self.videos.iter().copied().filter(|&i| all(&self.entries[i].search)).take(40).collect(),
            playlists: (0..self.playlists.len()).filter(|&i| all(&fold(&self.playlists[i].name))).collect(),
        })
    }
}

fn text_field<'a>(t: &'a Track, key: &str) -> &'a str {
    match key {
        "title" => &t.title,
        "artist" => &t.artist,
        "album" => &t.album,
        "albumArtist" => &t.album_artist,
        "genre" => &t.genre,
        _ => "",
    }
}

/// Trie la liste des morceaux par colonne (`title`, `artist`, `album`, `playCount`, `duration`, `addedAt`), comme la
/// vue d'origine : à égalité, par titre.
pub fn sort_tracks(c: &Catalog, list: &mut [usize], key: &str, desc: bool) {
    list.sort_by(|&a, &b| {
        let (ta, tb) = (&c.entries[a].track, &c.entries[b].track);
        let r = match key {
            "playCount" => ta.play_count.cmp(&tb.play_count),
            "duration" => ta.duration.partial_cmp(&tb.duration).unwrap_or(Ordering::Equal),
            "addedAt" => ta.added_at.cmp(&tb.added_at),
            _ => natural_cmp(text_field(ta, key), text_field(tb, key)).then_with(|| natural_cmp(&ta.title, &tb.title)),
        };
        let r = if desc { r.reverse() } else { r };
        r.then_with(|| natural_cmp(&ta.title, &tb.title))
    });
}

/// Trie les albums (`title`, `artist`, `year`, `addedAt`, `playCount`).
pub fn sort_albums(c: &Catalog, list: &mut [usize], key: &str) {
    list.sort_by(|&a, &b| {
        let (x, y) = (&c.albums[a], &c.albums[b]);
        match key {
            "artist" => {
                let ax = if x.artist.is_empty() { "~" } else { &x.artist };
                let ay = if y.artist.is_empty() { "~" } else { &y.artist };
                natural_cmp(ax, ay).then_with(|| natural_cmp(&x.title, &y.title))
            }
            "year" => y.year.unwrap_or(0).cmp(&x.year.unwrap_or(0)).then_with(|| natural_cmp(&x.title, &y.title)),
            "addedAt" => y.added_at.cmp(&x.added_at),
            "playCount" => y.play_count.cmp(&x.play_count).then_with(|| natural_cmp(&x.title, &y.title)),
            _ => natural_cmp(&x.title, &y.title),
        }
    });
}

/// Trie les vidéos (`addedAt`, `title`, `duration`, `playCount`).
pub fn sort_videos(c: &Catalog, list: &mut [usize], key: &str) {
    list.sort_by(|&a, &b| {
        let (x, y) = (&c.entries[a].track, &c.entries[b].track);
        match key {
            "title" => natural_cmp(&x.title, &y.title),
            "duration" => y.duration.partial_cmp(&x.duration).unwrap_or(Ordering::Equal),
            "playCount" => y.play_count.cmp(&x.play_count).then_with(|| natural_cmp(&x.title, &y.title)),
            _ => y.added_at.cmp(&x.added_at),
        }
    });
}

#[cfg(test)]
mod tests;

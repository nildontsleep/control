//! Les méthodes de l'API Subsonic. Ni transcodage, ni podcasts, ni radios, ni partages : ce que le serveur ne fait
//! pas est annoncé comme vide ou refusé avec le code d'erreur prévu.

use std::cmp::Ordering;
use std::path::Path;

use serde_json::{Value, json};

use super::model::{Album, Artist, Model, clean, cover_id, genres_of, iso, nz};
use super::{Ctx, FAVORITES, Outcome, SubResult, Subsonic, code, sub_err};
use crate::compare::fold;
use control_core::model::{Kind, PlaylistKind, Track};

const METHODS: &[&str] = &[
    "ping",
    "getlicense",
    "getopensubsonicextensions",
    "tokeninfo",
    "getmusicfolders",
    "getuser",
    "getindexes",
    "getartists",
    "getartist",
    "getalbum",
    "getsong",
    "getvideos",
    "getmusicdirectory",
    "getgenres",
    "getalbumlist",
    "getalbumlist2",
    "getrandomsongs",
    "getsongsbygenre",
    "gettopsongs",
    "getsimilarsongs",
    "getsimilarsongs2",
    "getartistinfo",
    "getartistinfo2",
    "getalbuminfo",
    "getalbuminfo2",
    "search2",
    "search3",
    "getstarred",
    "getstarred2",
    "star",
    "unstar",
    "setrating",
    "getplaylists",
    "getplaylist",
    "createplaylist",
    "updateplaylist",
    "deleteplaylist",
    "stream",
    "download",
    "getcoverart",
    "getavatar",
    "getlyricsbysongid",
    "getlyrics",
    "scrobble",
    "getnowplaying",
    "getscanstatus",
    "startscan",
    "getbookmarks",
    "createbookmark",
    "deletebookmark",
    "getplayqueue",
    "saveplayqueue",
    "getinternetradiostations",
    "getpodcasts",
    "getnewestpodcasts",
    "getshares",
    "jukeboxcontrol",
    "getusers",
];

pub fn exists(method: &str) -> bool {
    METHODS.contains(&method)
}

fn ok(v: Value) -> SubResult<Outcome> {
    Ok(Outcome::Payload(v))
}

fn by_fold(a: &str, b: &str) -> Ordering {
    fold(a).cmp(&fold(b))
}

fn has_genre(t: &Track, g: &str) -> bool {
    genres_of(&t.genre).any(|x| fold(x) == g)
}

// ─── Mise en forme ─────────────────────────────────────────────────────────────

pub fn song(t: &Track, m: &Model) -> Value {
    let path = Path::new(&t.path);
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let parent = m.parents.get(&t.id);
    let album = parent.and_then(|(a, _)| m.album(a));
    let base = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    clean(json!({
        "id": t.id.to_string(),
        "parent": parent.map(|(a, _)| a.clone()),
        "isDir": false,
        "title": t.title,
        "album": album.map(|a| a.name.clone()).unwrap_or_else(|| t.album.clone()),
        "artist": if t.artist.is_empty() { album.map(|a| Value::from(a.artist.clone())).unwrap_or(Value::Null) } else { Value::from(t.artist.clone()) },
        "track": nz(t.track_no),
        "discNumber": nz(t.disc_no),
        "year": nz(t.year),
        "genre": t.genre,
        "coverArt": cover_id(t.cover.as_deref()),
        "size": t.size,
        "contentType": crate::http::mime_of(path),
        "suffix": ext,
        "duration": t.duration.round() as i64,
        "bitRate": nz(t.bitrate),
        "path": format!("{}/{}/{base}", album.map(|a| a.artist.as_str()).unwrap_or("Inconnu"), album.map(|a| a.name.as_str()).unwrap_or("Inconnu")),
        "playCount": nz(Some(t.play_count)),
        "created": iso(t.added_at),
        "albumId": parent.map(|(a, _)| a.clone()),
        "artistId": parent.map(|(_, a)| a.clone()),
        "type": if t.kind == Kind::Video { "video" } else { "music" },
        "isVideo": t.kind == Kind::Video,
        "mediaType": "song",
    }))
}

fn album_id3(a: &Album) -> Value {
    clean(json!({
        "id": a.id, "name": a.name, "artist": a.artist, "artistId": a.artist_id, "coverArt": cover_id(a.cover.as_deref()),
        "songCount": a.tracks.len(), "duration": a.duration.round() as i64, "playCount": nz(Some(a.play_count)),
        "created": iso(a.added_at), "year": nz(a.year), "genre": a.genre,
    }))
}

fn album_dir(a: &Album) -> Value {
    clean(json!({
        "id": a.id, "parent": a.artist_id, "isDir": true, "title": a.name, "album": a.name, "artist": a.artist,
        "year": nz(a.year), "genre": a.genre, "coverArt": cover_id(a.cover.as_deref()), "playCount": nz(Some(a.play_count)),
        "created": iso(a.added_at),
    }))
}

fn artist_id3(ar: &Artist) -> Value {
    clean(
        json!({ "id": ar.id, "name": ar.name, "albumCount": ar.albums.len(), "coverArt": cover_id(ar.cover.as_deref()) }),
    )
}

fn indexes(m: &Model, id3: bool) -> Value {
    let mut list: Vec<&Artist> = m.artists.iter().collect();
    list.sort_by(|a, b| by_fold(&a.name, &b.name));
    let mut letters: Vec<(String, Vec<Value>)> = Vec::new();
    for ar in list {
        let first = fold(&ar.name).chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        let letter = if first.len() == 1 && first.chars().all(|c| c.is_ascii_uppercase()) { first } else { "#".into() };
        let item = if id3 { artist_id3(ar) } else { clean(json!({ "id": ar.id, "name": ar.name })) };
        match letters.iter_mut().find(|(l, _)| *l == letter) {
            Some((_, v)) => v.push(item),
            None => letters.push((letter, vec![item])),
        }
    }
    Value::Array(letters.into_iter().map(|(name, artist)| json!({ "name": name, "artist": artist })).collect())
}

fn page<T: Clone>(list: &[T], ctx: &Ctx<'_>, size: &str, offset: &str, max: f64) -> Vec<T> {
    let size = ctx.params.num(size, 10.0).min(max).max(0.0) as usize;
    let offset = ctx.params.num(offset, 0.0).max(0.0) as usize;
    list.iter().skip(offset).take(size).cloned().collect()
}

fn favorites(ctx: &Ctx<'_>) -> (Option<i64>, Vec<i64>) {
    let (playlists, items) = ctx.entry.core.library.playlists();
    let pl = playlists.iter().find(|p| p.name == FAVORITES && p.kind == PlaylistKind::Manual).map(|p| p.id);
    let ids =
        pl.map(|id| items.iter().filter(|i| i.playlist_id == id).map(|i| i.track_id).collect()).unwrap_or_default();
    (pl, ids)
}

fn playlist_info(p: &control_core::model::Playlist, owner: &str, track_ids: &[i64], m: &Model) -> Value {
    let tracks: Vec<&Track> = track_ids.iter().filter_map(|id| m.song(*id)).collect();
    let cover = p.cover.clone().or_else(|| tracks.iter().find_map(|t| t.cover.clone()));
    clean(json!({
        "id": format!("pl-{}", p.id), "name": p.name, "comment": p.description, "owner": owner, "public": p.shared,
        "songCount": tracks.len(), "duration": tracks.iter().map(|t| t.duration).sum::<f64>().round() as i64,
        "created": iso(p.created_at), "changed": iso(p.updated_at), "coverArt": cover_id(cover.as_deref()),
    }))
}

fn playlist_id(ctx: &Ctx<'_>, name: &str) -> SubResult<i64> {
    let v = ctx.params.need(name)?;
    v.strip_prefix("pl-").and_then(|n| n.parse().ok()).ok_or_else(|| sub_err(code::NOT_FOUND, "Playlist introuvable."))
}

fn track_file(ctx: &Ctx<'_>, id: &str) -> SubResult<std::path::PathBuf> {
    let library = &ctx.entry.core.library;
    let file = id.parse::<i64>().ok().and_then(|id| library.track_path(id));
    match file {
        Some(f) if ctx.hub.is_inside_roots(&ctx.user.id, &f) => Ok(f.into()),
        _ => Err(sub_err(code::NOT_FOUND, "Morceau introuvable.")),
    }
}

fn get_playlist(ctx: &Ctx<'_>, m: &Model, id: i64) -> SubResult<Outcome> {
    let (playlists, items) = ctx.entry.core.library.playlists();
    let p = playlists
        .iter()
        .find(|p| p.id == id && p.kind == PlaylistKind::Manual)
        .ok_or_else(|| sub_err(code::NOT_FOUND, "Playlist introuvable."))?;
    let ids: Vec<i64> = items.iter().filter(|i| i.playlist_id == id).map(|i| i.track_id).collect();
    let mut info = playlist_info(p, &ctx.user.username, &ids, m);
    info["entry"] = Value::Array(ids.iter().filter_map(|id| m.song(*id)).map(|t| song(t, m)).collect());
    ok(json!({ "playlist": info }))
}

fn manual_exists(ctx: &Ctx<'_>, id: i64) -> bool {
    ctx.entry.core.library.playlists().0.iter().any(|p| p.id == id && p.kind == PlaylistKind::Manual)
}

fn album_list(ctx: &Ctx<'_>, m: &Model, id3: bool) -> SubResult<Outcome> {
    let kind = ctx.params.get("type").unwrap_or("alphabeticalByName");
    let mut list: Vec<&Album> = m.albums.iter().collect();
    let by_name = |a: &&Album, b: &&Album| by_fold(&a.name, &b.name);
    match kind {
        "random" => {
            for i in (1..list.len()).rev() {
                list.swap(i, rand::random_range(0..=i));
            }
        }
        "newest" => list.sort_by_key(|a| std::cmp::Reverse(a.added_at)),
        "frequent" => {
            list.retain(|a| a.play_count != 0);
            list.sort_by_key(|a| std::cmp::Reverse(a.play_count));
        }
        "recent" => {
            list.retain(|a| a.last_played != 0);
            list.sort_by_key(|a| std::cmp::Reverse(a.last_played));
        }
        "starred" => list.clear(),
        "alphabeticalByArtist" => list.sort_by(|a, b| by_fold(&a.artist, &b.artist).then_with(|| by_name(a, b))),
        "byYear" => {
            let from = ctx.params.num("fromYear", 0.0) as i64;
            let to = ctx.params.num("toYear", 9999.0) as i64;
            list.retain(|a| a.year.is_some_and(|y| y >= from.min(to) && y <= from.max(to)));
            list.sort_by(|a, b| {
                let (x, y) = (a.year.unwrap_or(0), b.year.unwrap_or(0));
                (if from <= to { x.cmp(&y) } else { y.cmp(&x) }).then_with(|| by_name(a, b))
            });
        }
        "byGenre" => {
            let g = fold(ctx.params.get("genre").unwrap_or(""));
            list.retain(|a| a.tracks.iter().any(|t| has_genre(t, &g)));
            list.sort_by(by_name);
        }
        _ => list.sort_by(by_name),
    }
    let rows: Vec<Value> = page(&list, ctx, "size", "offset", 500.0)
        .into_iter()
        .map(|a| if id3 { album_id3(a) } else { album_dir(a) })
        .collect();
    ok(if id3 { json!({ "albumList2": { "album": rows } }) } else { json!({ "albumList": { "album": rows } }) })
}

fn search(ctx: &Ctx<'_>, m: &Model, id3: bool) -> SubResult<Outcome> {
    let q = fold(ctx.params.get("query").unwrap_or("")).replace('*', " ").trim().to_string();
    let tokens: Vec<&str> = q.split(' ').filter(|t| !t.is_empty()).collect();
    let matches = |text: &str| tokens.iter().all(|tk| text.contains(tk));
    let rank = |text: &str| if text.starts_with(&q) { 0 } else { 1 };
    let take = |count: &str, offset: &str| {
        let off = ctx.params.num(offset, 0.0).max(0.0) as usize;
        let n = ctx.params.num(count, 20.0).clamp(0.0, 500.0) as usize;
        (off, n)
    };
    let sorted = |key: &dyn Fn(usize) -> String, len: usize, hay: &dyn Fn(usize) -> String| {
        let mut idx: Vec<usize> = (0..len).filter(|&i| tokens.is_empty() || matches(&hay(i))).collect();
        idx.sort_by(|&a, &b| {
            let (ka, kb) = (fold(&key(a)), fold(&key(b)));
            rank(&ka).cmp(&rank(&kb)).then(ka.cmp(&kb))
        });
        idx
    };
    let artists = sorted(&|i| m.artists[i].name.clone(), m.artists.len(), &|i| fold(&m.artists[i].name));
    let albums = sorted(&|i| m.albums[i].name.clone(), m.albums.len(), &|i| {
        fold(&format!("{} {}", m.albums[i].name, m.albums[i].artist))
    });
    let songs = sorted(&|i| m.songs[i].title.clone(), m.songs.len(), &|i| {
        let t = &m.songs[i];
        fold(&format!("{} {} {}", t.title, t.artist, t.album))
    });
    let (ao, an) = take("artistCount", "artistOffset");
    let (lo, ln) = take("albumCount", "albumOffset");
    let (so, sn) = take("songCount", "songOffset");
    let result = json!({
        "artist": artists.iter().skip(ao).take(an).map(|&i| {
            let a = &m.artists[i];
            if id3 { artist_id3(a) } else { clean(json!({ "id": a.id, "name": a.name })) }
        }).collect::<Vec<_>>(),
        "album": albums.iter().skip(lo).take(ln).map(|&i| if id3 { album_id3(&m.albums[i]) } else { album_dir(&m.albums[i]) }).collect::<Vec<_>>(),
        "song": songs.iter().skip(so).take(sn).map(|&i| song(&m.songs[i], m)).collect::<Vec<_>>(),
    });
    ok(if id3 { json!({ "searchResult3": result }) } else { json!({ "searchResult2": result }) })
}

fn starred(ctx: &Ctx<'_>, m: &Model) -> Value {
    let (_, ids) = favorites(ctx);
    json!({ "song": ids.iter().filter_map(|id| m.song(*id)).map(|t| song(t, m)).collect::<Vec<_>>(), "album": [], "artist": [] })
}

fn annotate(ctx: &Ctx<'_>, on: bool) -> SubResult<Outcome> {
    let library = &ctx.entry.core.library;
    let ids: Vec<i64> = ctx.params.all("id").iter().filter_map(|v| v.parse().ok()).collect();
    let (pl, _) = favorites(ctx);
    let pl = match (pl, on) {
        (None, false) => return ok(json!({})),
        (None, true) => library.create_playlist(FAVORITES, PlaylistKind::Manual, None),
        (Some(id), _) => Some(id),
    };
    let Some(pl) = pl else { return ok(json!({})) };
    let items = library.playlists().1;
    let present: Vec<(i64, i64)> = items.iter().filter(|i| i.playlist_id == pl).map(|i| (i.track_id, i.id)).collect();
    if on {
        let mut add: Vec<i64> = Vec::new();
        for id in ids {
            if !present.iter().any(|(t, _)| *t == id) && !add.contains(&id) {
                add.push(id);
            }
        }
        library.add_to_playlist(pl, &add);
    } else {
        let remove: Vec<i64> =
            ids.iter().filter_map(|id| present.iter().find(|(t, _)| t == id).map(|(_, e)| *e)).collect();
        library.remove_from_playlist(&remove);
    }
    ok(json!({}))
}

fn lines_of(res: &control_core::lyrics::LyricsResult) -> (Vec<Value>, Vec<Value>) {
    let synced: Vec<Value> = res
        .synced
        .as_deref()
        .unwrap_or("")
        .split('\n')
        .filter_map(|l| {
            let rest = l.strip_prefix('[')?;
            let (stamp, text) = rest.split_once(']')?;
            let (min, sec) = stamp.split_once(':')?;
            let (min, sec) = (min.parse::<f64>().ok()?, sec.parse::<f64>().ok()?);
            if !stamp.chars().all(|c| c.is_ascii_digit() || c == ':' || c == '.') {
                return None;
            }
            Some(json!({ "start": ((min * 60.0 + sec) * 1000.0).round() as i64, "value": text.trim_start() }))
        })
        .collect();
    let plain: Vec<Value> =
        res.plain.as_deref().map(|p| p.split('\n').map(|v| json!({ "value": v })).collect()).unwrap_or_default();
    (synced, plain)
}

/// Appelle une méthode (son existence a été vérifiée).
pub fn call(sub: &Subsonic, ctx: &Ctx<'_>, method: &str) -> SubResult<Outcome> {
    let p = &ctx.params;
    let lib = &ctx.entry.core.library;
    let model = || sub.model(&ctx.entry);
    match method {
        "ping" | "setrating" | "createbookmark" | "deletebookmark" | "saveplayqueue" => ok(json!({})),
        "getlicense" => {
            ok(json!({ "license": { "valid": true, "email": "", "licenseExpires": "2099-12-31T00:00:00.000Z" } }))
        }
        "getopensubsonicextensions" => ok(json!({ "openSubsonicExtensions": [
            { "name": "apiKeyAuthentication", "versions": [1] }, { "name": "formPost", "versions": [1] },
            { "name": "songLyrics", "versions": [1] },
        ] })),
        "tokeninfo" => ok(json!({ "tokenInfo": { "username": ctx.user.username } })),
        "getmusicfolders" => ok(json!({ "musicFolders": { "musicFolder": [{ "id": 1, "name": "Bibliothèque" }] } })),
        "getuser" => ok(json!({ "user": {
            "username": ctx.user.username, "email": "", "scrobblingEnabled": true, "adminRole": false, "settingsRole": false,
            "downloadRole": true, "uploadRole": false, "playlistRole": true, "coverArtRole": false, "commentRole": false,
            "podcastRole": false, "streamRole": true, "jukeboxRole": false, "shareRole": false, "videoConversionRole": false,
            "folder": [1],
        } })),
        "getindexes" => ok(
            json!({ "indexes": { "lastModified": crate::util::now_ms(), "ignoredArticles": "", "index": indexes(&model(), false) } }),
        ),
        "getartists" => ok(
            json!({ "artists": { "ignoredArticles": "", "lastModified": crate::util::now_ms(), "index": indexes(&model(), true) } }),
        ),
        "getartist" => {
            let m = model();
            let ar = m.artist(p.need("id")?).ok_or_else(|| sub_err(code::NOT_FOUND, "Artiste introuvable."))?;
            let mut albums: Vec<&Album> = ar.albums.iter().filter_map(|id| m.album(id)).collect();
            albums
                .sort_by(|a, b| a.year.unwrap_or(0).cmp(&b.year.unwrap_or(0)).then_with(|| by_fold(&a.name, &b.name)));
            let mut out = artist_id3(ar);
            out["album"] = Value::Array(albums.into_iter().map(album_id3).collect());
            ok(json!({ "artist": out }))
        }
        "getalbum" => {
            let m = model();
            let a = m.album(p.need("id")?).ok_or_else(|| sub_err(code::NOT_FOUND, "Album introuvable."))?;
            let mut out = album_id3(a);
            out["song"] = Value::Array(a.tracks.iter().map(|t| song(t, &m)).collect());
            ok(json!({ "album": out }))
        }
        "getsong" => {
            let m = model();
            let id = p.need("id")?.parse::<i64>().unwrap_or(-1);
            let t = m.song(id).ok_or_else(|| sub_err(code::NOT_FOUND, "Morceau introuvable."))?;
            ok(json!({ "song": song(t, &m) }))
        }
        "getvideos" => {
            let m = model();
            ok(
                json!({ "videos": { "video": m.songs.iter().filter(|t| t.kind == Kind::Video).map(|t| song(t, &m)).collect::<Vec<_>>() } }),
            )
        }
        "getmusicdirectory" => {
            let m = model();
            let id = p.need("id")?;
            if let Some(ar) = m.artist(id) {
                let child: Vec<Value> = ar.albums.iter().filter_map(|a| m.album(a)).map(album_dir).collect();
                return ok(json!({ "directory": { "id": ar.id, "name": ar.name, "child": child } }));
            }
            if let Some(a) = m.album(id) {
                let child: Vec<Value> = a.tracks.iter().map(|t| song(t, &m)).collect();
                return ok(
                    json!({ "directory": { "id": a.id, "parent": a.artist_id, "name": a.name, "child": child } }),
                );
            }
            Err(sub_err(code::NOT_FOUND, "Dossier introuvable."))
        }
        "getgenres" => {
            let m = model();
            let mut list: Vec<_> = m.genres.iter().collect();
            list.sort_by(|a, b| by_fold(&a.value, &b.value));
            let genre: Vec<Value> = list
                .iter()
                .map(|g| json!({ "value": g.value, "songCount": g.song_count, "albumCount": g.albums.len() }))
                .collect();
            ok(json!({ "genres": { "genre": genre } }))
        }
        "getalbumlist" => album_list(ctx, &model(), false),
        "getalbumlist2" => album_list(ctx, &model(), true),
        "getrandomsongs" => {
            let m = model();
            let g = p.get("genre").filter(|s| !s.is_empty()).map(fold);
            let (from, to) = (p.num("fromYear", 0.0) as i64, p.num("toYear", 9999.0) as i64);
            let mut pool: Vec<&Track> = m
                .songs
                .iter()
                .filter(|t| t.kind == Kind::Audio)
                .filter(|t| g.as_ref().is_none_or(|g| has_genre(t, g)))
                .filter(|t| t.year.filter(|y| *y != 0).is_none_or(|y| y >= from && y <= to))
                .collect();
            let want = p.num("size", 10.0).clamp(0.0, 500.0) as usize;
            let mut out = Vec::new();
            while out.len() < want && !pool.is_empty() {
                out.push(pool.swap_remove(rand::random_range(0..pool.len())));
            }
            ok(json!({ "randomSongs": { "song": out.into_iter().map(|t| song(t, &m)).collect::<Vec<_>>() } }))
        }
        "getsongsbygenre" => {
            let m = model();
            let g = fold(p.need("genre")?);
            let list: Vec<&Track> = m.songs.iter().filter(|t| t.kind == Kind::Audio && has_genre(t, &g)).collect();
            ok(
                json!({ "songsByGenre": { "song": page(&list, ctx, "count", "offset", 500.0).into_iter().map(|t| song(t, &m)).collect::<Vec<_>>() } }),
            )
        }
        "gettopsongs" => {
            let m = model();
            let name = fold(p.need("artist")?);
            let mut list: Vec<&Track> =
                m.songs.iter().filter(|t| fold(&t.artist) == name || fold(&t.album_artist) == name).collect();
            list.sort_by_key(|a| std::cmp::Reverse(a.play_count));
            let n = p.num("count", 50.0).max(0.0) as usize;
            ok(json!({ "topSongs": { "song": list.into_iter().take(n).map(|t| song(t, &m)).collect::<Vec<_>>() } }))
        }
        "getsimilarsongs" => ok(json!({ "similarSongs": {} })),
        "getsimilarsongs2" => ok(json!({ "similarSongs2": {} })),
        "getartistinfo" => ok(json!({ "artistInfo": {} })),
        "getartistinfo2" => ok(json!({ "artistInfo2": {} })),
        "getalbuminfo" | "getalbuminfo2" => ok(json!({ "albumInfo": {} })),
        "search2" => search(ctx, &model(), false),
        "search3" => search(ctx, &model(), true),
        "getstarred" => ok(json!({ "starred": starred(ctx, &model()) })),
        "getstarred2" => ok(json!({ "starred2": starred(ctx, &model()) })),
        "star" => annotate(ctx, true),
        "unstar" => annotate(ctx, false),
        "getplaylists" => {
            let m = model();
            let (playlists, items) = lib.playlists();
            let list: Vec<Value> = playlists
                .iter()
                .filter(|pl| pl.kind == PlaylistKind::Manual)
                .map(|pl| {
                    let ids: Vec<i64> = items.iter().filter(|i| i.playlist_id == pl.id).map(|i| i.track_id).collect();
                    playlist_info(pl, &ctx.user.username, &ids, &m)
                })
                .collect();
            ok(json!({ "playlists": { "playlist": list } }))
        }
        "getplaylist" => get_playlist(ctx, &model(), playlist_id(ctx, "id")?),
        "createplaylist" => {
            let songs: Vec<i64> = p.all("songId").iter().filter_map(|v| v.parse().ok()).collect();
            let id = if p.get("playlistId").is_some_and(|s| !s.is_empty()) {
                // Remplace le contenu d'une playlist existante.
                let id = playlist_id(ctx, "playlistId")?;
                if !manual_exists(ctx, id) {
                    return Err(sub_err(code::NOT_FOUND, "Playlist introuvable."));
                }
                let entries: Vec<i64> =
                    lib.playlists().1.iter().filter(|i| i.playlist_id == id).map(|i| i.id).collect();
                lib.remove_from_playlist(&entries);
                if let Some(name) = p.get("name").filter(|s| !s.is_empty()) {
                    lib.rename_playlist(id, name);
                }
                id
            } else {
                lib.create_playlist(p.need("name")?, PlaylistKind::Manual, None)
                    .ok_or_else(|| sub_err(code::GENERIC, "Playlist impossible à créer."))?
            };
            if !songs.is_empty() {
                lib.add_to_playlist(id, &songs);
            }
            get_playlist(ctx, &model(), id)
        }
        "updateplaylist" => {
            let id = playlist_id(ctx, "playlistId")?;
            if !manual_exists(ctx, id) {
                return Err(sub_err(code::NOT_FOUND, "Playlist introuvable."));
            }
            let items = lib.playlists().1;
            let name = p.get("name").filter(|s| !s.is_empty());
            let comment = p.get("comment");
            let public = p.get("public").map(|v| v == "true");
            if name.is_some() || comment.is_some() || public.is_some() {
                lib.update_playlist(id, name, comment, public);
            }
            let mut entries: Vec<_> = items.iter().filter(|i| i.playlist_id == id).collect();
            entries.sort_by_key(|i| i.position);
            let remove: Vec<i64> = p
                .all("songIndexToRemove")
                .iter()
                .filter_map(|v| v.parse::<usize>().ok())
                .filter_map(|i| entries.get(i).map(|e| e.id))
                .collect();
            if !remove.is_empty() {
                lib.remove_from_playlist(&remove);
            }
            let add: Vec<i64> = p.all("songIdToAdd").iter().filter_map(|v| v.parse().ok()).collect();
            if !add.is_empty() {
                lib.add_to_playlist(id, &add);
            }
            ok(json!({}))
        }
        "deleteplaylist" => {
            let id = playlist_id(ctx, "id")?;
            if !manual_exists(ctx, id) {
                return Err(sub_err(code::NOT_FOUND, "Playlist introuvable."));
            }
            lib.delete_playlist(id);
            ok(json!({}))
        }
        "stream" => Ok(Outcome::File {
            path: track_file(ctx, p.need("id")?)?,
            kind: None,
            cache: "private, no-store",
            headers: Vec::new(),
        }),
        "download" => {
            let file = track_file(ctx, p.need("id")?)?;
            let name: String = file
                .file_name()
                .map(|n| {
                    n.to_string_lossy()
                        .chars()
                        .map(|c| if c.is_alphanumeric() && c.is_ascii() || " ._-".contains(c) { c } else { '_' })
                        .collect()
                })
                .unwrap_or_default();
            Ok(Outcome::File {
                path: file,
                kind: None,
                cache: "private, no-store",
                headers: vec![("content-disposition", format!("attachment; filename=\"{name}\""))],
            })
        }
        "getcoverart" => {
            let id = p.need("id")?;
            let m = model();
            let hashed = if let Some(h) = id.strip_prefix("cv-") {
                Some(h.to_string())
            } else if id.starts_with("al-") {
                m.album(id).and_then(|a| a.cover.clone())
            } else if id.starts_with("ar-") {
                m.artist(id).and_then(|a| a.cover.clone())
            } else if let Some(n) = id.strip_prefix("pl-") {
                let (playlists, items) = lib.playlists();
                let pl = playlists.iter().find(|x| Some(x.id) == n.parse().ok());
                pl.and_then(|pl| {
                    pl.cover.clone().or_else(|| {
                        items
                            .iter()
                            .filter(|i| i.playlist_id == pl.id)
                            .find_map(|i| m.song(i.track_id).and_then(|t| t.cover.clone()))
                    })
                })
            } else {
                id.parse::<i64>().ok().and_then(|i| m.song(i)).and_then(|t| t.cover.clone())
            };
            let Some(h) =
                hashed.filter(|h| h.len() == 20 && h.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()))
            else {
                return Err(sub_err(code::NOT_FOUND, "Pas de pochette."));
            };
            let size = p.num("size", 0.0);
            let small = ctx.hub.covers_dir.join(format!("{h}-s.jpg"));
            let file = if size > 0.0 && size <= 256.0 && small.exists() {
                small
            } else {
                ctx.hub.covers_dir.join(format!("{h}.jpg"))
            };
            if !file.exists() {
                return Err(sub_err(code::NOT_FOUND, "Pochette introuvable."));
            }
            Ok(Outcome::File {
                path: file,
                kind: Some("image/jpeg".into()),
                cache: "private, max-age=86400",
                headers: Vec::new(),
            })
        }
        "getavatar" => {
            let name = p.need("username")?;
            let avatar: Option<String> = ctx
                .hub
                .db
                .lock()
                .query_row("SELECT avatar FROM users WHERE username = ? COLLATE NOCASE", [name], |r| r.get(0))
                .ok()
                .flatten();
            let Some(a) = avatar.filter(|h| h.len() == 20 && h.chars().all(|c| c.is_ascii_hexdigit())) else {
                return Err(sub_err(code::NOT_FOUND, "Pas d’avatar."));
            };
            Ok(Outcome::File {
                path: ctx.hub.avatars_dir.join(format!("{a}.jpg")),
                kind: Some("image/jpeg".into()),
                cache: "private, max-age=86400",
                headers: Vec::new(),
            })
        }
        "getlyricsbysongid" => {
            let m = model();
            let id = p.need("id")?.parse::<i64>().unwrap_or(-1);
            let t = m.song(id).ok_or_else(|| sub_err(code::NOT_FOUND, "Morceau introuvable."))?;
            let Ok(res) = ctx.entry.core.lyrics.get(id, false) else { return ok(json!({ "lyricsList": {} })) };
            let (synced, plain) = lines_of(&res);
            let is_synced = !synced.is_empty();
            let lines = if is_synced { synced } else { plain };
            if lines.is_empty() {
                return ok(json!({ "lyricsList": {} }));
            }
            ok(json!({ "lyricsList": { "structuredLyrics": [{
                "displayArtist": t.artist, "displayTitle": t.title, "lang": "xxx", "synced": is_synced, "line": lines,
            }] } }))
        }
        "getlyrics" => {
            let m = model();
            let artist = fold(p.get("artist").unwrap_or(""));
            let title = fold(p.get("title").unwrap_or(""));
            let t =
                m.songs.iter().find(|x| fold(&x.title) == title && (artist.is_empty() || fold(&x.artist) == artist));
            let res = t.and_then(|t| ctx.entry.core.lyrics.get(t.id, false).ok());
            let text = res
                .map(|r| {
                    r.plain.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| {
                        r.synced
                            .unwrap_or_default()
                            .split('\n')
                            .map(|l| {
                                if l.starts_with('[') {
                                    l.split_once(']').map(|(_, rest)| rest.trim_start()).unwrap_or(l)
                                } else {
                                    l
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                })
                .unwrap_or_default();
            ok(
                json!({ "lyrics": clean(json!({ "artist": t.map(|t| t.artist.clone()), "title": t.map(|t| t.title.clone()), "value": text })) }),
            )
        }
        "scrobble" => {
            // Un lecteur annonce qu'il joue un morceau (submission=false) ou qu'il l'a écouté (comptée comme écoute entière).
            let ids: Vec<i64> = p.all("id").iter().filter_map(|v| v.parse().ok()).collect();
            if ids.is_empty() {
                return Err(sub_err(code::MISSING, "Paramètre manquant : id."));
            }
            let times: Vec<f64> = p.all("time").iter().map(|v| v.parse().unwrap_or(f64::NAN)).collect();
            let submission = p.get("submission") != Some("false");
            let core = &ctx.entry.core;
            let m = model();
            for (i, id) in ids.iter().enumerate() {
                let Some(t) = m.song(*id) else { continue };
                if !submission {
                    core.scrobbler.now_playing(*id);
                    continue;
                }
                if let Some(play) = core.library.play_start(*id, t.duration) {
                    core.library.play_end(play, t.duration, true);
                }
                let at =
                    times.get(i).copied().filter(|v| v.is_finite() && *v > 0.0).map(|v| (v / 1000.0).floor() as i64);
                core.scrobbler.submit(*id, at.unwrap_or_else(|| crate::util::now_ms() / 1000));
            }
            ok(json!({}))
        }
        "getnowplaying" => ok(json!({ "nowPlaying": {} })),
        "getscanstatus" => ok(json!({ "scanStatus": { "scanning": lib.is_scanning(), "count": model().songs.len() } })),
        "startscan" => {
            let entry = ctx.entry.clone();
            let roots = ctx.hub.roots_for(&ctx.user.id);
            let _ = std::thread::Builder::new().name("control-scan".into()).spawn(move || {
                entry.core.library.scan(&roots);
            });
            ok(json!({ "scanStatus": { "scanning": true, "count": model().songs.len() } }))
        }
        "getbookmarks" => ok(json!({ "bookmarks": {} })),
        "getplayqueue" => ok(json!({})),
        "getinternetradiostations" => ok(json!({ "internetRadioStations": {} })),
        "getpodcasts" => ok(json!({ "podcasts": {} })),
        "getnewestpodcasts" => ok(json!({ "newestPodcasts": {} })),
        "getshares" => ok(json!({ "shares": {} })),
        "jukeboxcontrol" => Err(sub_err(code::FORBIDDEN, "Le mode jukebox n’est pas pris en charge.")),
        "getusers" => Err(sub_err(code::FORBIDDEN, "Réservé à l’administration du serveur.")),
        _ => Err(sub_err(code::GENERIC, format!("Méthode inconnue : {method}"))),
    }
}

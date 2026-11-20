//! Statistiques d'écoute et de visionnage, calculées à la demande depuis la base (`plays`, `watch_time`).
//! Mêmes requêtes et même forme JSON que l'original.

use std::collections::{BTreeSet, HashMap};

use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::Value;

use crate::util::local_day;

const DAY_MS: i64 = 86_400_000;
/// Une lecture n'est comptée qu'après 30 s : à défaut de durée exacte, c'est le minimum sûr.
const LISTENED: &str = "COALESCE(NULLIF(p.seconds, 0), MIN(p.duration, 30))";
const LOCAL: &str = "'unixepoch', 'localtime'";
const ARTIST: &str = "COALESCE(NULLIF(t.album_artist, ''), NULLIF(t.artist, ''))";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Range {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
pub struct Streak {
    pub current: u32,
    pub longest: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TopTrack {
    pub id: i64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub cover: Option<String>,
    pub plays: i64,
    pub seconds: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TopArtist {
    pub name: String,
    pub plays: i64,
    pub seconds: f64,
    pub tracks: i64,
    pub cover: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TopAlbum {
    pub album: String,
    pub artist: Option<String>,
    pub plays: i64,
    pub seconds: f64,
    pub cover: Option<String>,
    pub track_id: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Genre {
    pub name: String,
    pub plays: i64,
    pub seconds: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DayPlays {
    pub day: String,
    pub plays: i64,
    pub seconds: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Music {
    pub plays: i64,
    pub seconds: f64,
    pub tracks: i64,
    pub artists: i64,
    pub albums: i64,
    pub completed: i64,
    pub top_tracks: Vec<TopTrack>,
    pub top_artists: Vec<TopArtist>,
    pub top_albums: Vec<TopAlbum>,
    pub genres: Vec<Genre>,
    pub by_day: Vec<DayPlays>,
    /// Secondes par heure locale (0 à 23).
    pub by_hour: Vec<f64>,
    /// Secondes par jour de la semaine, du lundi au dimanche.
    pub by_weekday: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Watched {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub seconds: f64,
    pub title: Option<String>,
    pub poster: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VideoGenre {
    pub name: String,
    pub seconds: f64,
    pub titles: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DaySeconds {
    pub day: String,
    pub seconds: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Video {
    pub seconds: f64,
    pub titles: i64,
    pub finished: i64,
    pub top: Vec<Watched>,
    pub genres: Vec<VideoGenre>,
    pub by_day: Vec<DaySeconds>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub range: Range,
    pub years: Vec<i64>,
    pub months: Vec<String>,
    pub days: Vec<String>,
    pub streak: Streak,
    pub music: Music,
    pub video: Video,
}

/// Bornes d'une période : `from` et `to` au format « AAAA-MM-JJ » (jours locaux, inclus). Sans `from`, depuis 2000 ;
/// sans `to`, jusqu'à aujourd'hui. Renvoie (début ms, fin exclue ms, jour de début, jour de fin).
pub fn bounds_of(from: &str, to: &str, now_ms: i64) -> (i64, i64, String, String) {
    let parse = |s: &str| -> Option<(i32, u8, u8)> {
        let b = s.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return None;
        }
        Some((s[0..4].parse().ok()?, s[5..7].parse().ok()?, s[8..10].parse().ok()?))
    };
    let today = crate::util::local_datetime(now_ms);
    let start = parse(from).unwrap_or((2000, 1, 1));
    let end = parse(to).unwrap_or((today.year(), u8::from(today.month()), today.day()));
    let start_ms = local_midnight(start.0, start.1, start.2, 0);
    let end_ms = local_midnight(end.0, end.1, end.2, 1);
    (start_ms, end_ms, local_day(start_ms), local_day(end_ms - 1))
}

/// Minuit local du jour donné (plus `plus_days` jours), en millisecondes. Les jours hors limites débordent sur le
/// mois suivant, comme en JavaScript.
fn local_midnight(y: i32, m: u8, d: u8, plus_days: i64) -> i64 {
    use time::{Date, Month, PrimitiveDateTime, Time};
    let month = Month::try_from(m.clamp(1, 12)).unwrap_or(Month::January);
    let base = Date::from_calendar_date(y, month, 1).unwrap_or(Date::MIN);
    let date = base.checked_add(time::Duration::days(d as i64 - 1 + plus_days)).unwrap_or(base);
    let naive = PrimitiveDateTime::new(date, Time::MIDNIGHT);
    // Décalage local à cette date (heure d'été comprise) : on le lit pour l'instant UTC correspondant.
    let guess = naive.assume_utc();
    let offset = time::UtcOffset::local_offset_at(guess).unwrap_or(time::UtcOffset::UTC);
    let at = naive.assume_offset(offset);
    // Si le décalage a changé entre-temps (jour de changement d'heure), on le relit une fois.
    let offset2 = time::UtcOffset::local_offset_at(at).unwrap_or(offset);
    let at = naive.assume_offset(offset2);
    (at.unix_timestamp_nanos() / 1_000_000) as i64
}

/// Plus longue série de jours consécutifs, et série en cours (aujourd'hui ou hier comptent).
pub fn streaks(days: &[String], now_ms: i64) -> Streak {
    let sorted: Vec<&String> = days.iter().collect::<BTreeSet<_>>().into_iter().collect();
    if sorted.is_empty() {
        return Streak::default();
    }
    let ordinal = |s: &str| -> Option<i64> {
        let y: i32 = s.get(0..4)?.parse().ok()?;
        let m: u8 = s.get(5..7)?.parse().ok()?;
        let d: u8 = s.get(8..10)?.parse().ok()?;
        let date = time::Date::from_calendar_date(y, time::Month::try_from(m).ok()?, d).ok()?;
        Some(date.to_julian_day() as i64)
    };
    let jd: Vec<Option<i64>> = sorted.iter().map(|s| ordinal(s)).collect();
    let consecutive = |i: usize| matches!((jd[i - 1], jd[i]), (Some(a), Some(b)) if b - a == 1);
    let mut longest = 1;
    let mut run = 1;
    for i in 1..sorted.len() {
        run = if consecutive(i) { run + 1 } else { 1 };
        longest = longest.max(run);
    }
    let today = local_day(now_ms);
    let yesterday = local_day(now_ms - DAY_MS);
    let last = sorted[sorted.len() - 1];
    let mut current = 0;
    if *last == today || *last == yesterday {
        current = 1;
        for i in (1..sorted.len()).rev() {
            if consecutive(i) {
                current += 1;
            } else {
                break;
            }
        }
    }
    Streak { current, longest }
}

/// Calcule les statistiques d'une période. `limit` : taille des tops (1 à 200).
pub fn get(db: &Connection, from: &str, to: &str, limit: usize, now_ms: i64) -> rusqlite::Result<Stats> {
    let (start, end, from_day, to_day) = bounds_of(from, to, now_ms);
    let top = limit.clamp(1, 200);
    let audio = "t.kind = 'audio' AND p.played_at >= ?1 AND p.played_at < ?2";

    let (plays, seconds, tracks, artists, albums, completed): (i64, f64, i64, i64, i64, i64) = db.query_row(
        &format!(
            "SELECT COUNT(*), COALESCE(SUM({LISTENED}), 0), COUNT(DISTINCT t.id), COUNT(DISTINCT {ARTIST}),
                    COUNT(DISTINCT NULLIF(t.album, '')), COALESCE(SUM(p.completed), 0)
             FROM plays p JOIN tracks t ON t.id = p.track_id WHERE {audio}"
        ),
        params![start, end],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
    )?;

    let top_tracks = collect(
        db,
        &format!(
            "SELECT t.id, t.title, t.artist, t.album, t.cover, COUNT(*) AS plays, SUM({LISTENED}) AS seconds
             FROM plays p JOIN tracks t ON t.id = p.track_id WHERE {audio}
             GROUP BY t.id ORDER BY plays DESC, seconds DESC, t.title LIMIT {top}"
        ),
        params![start, end],
        |r| {
            Ok(TopTrack {
                id: r.get(0)?,
                title: r.get(1)?,
                artist: r.get(2)?,
                album: r.get(3)?,
                cover: r.get(4)?,
                plays: r.get(5)?,
                seconds: r.get(6)?,
            })
        },
    )?;

    let top_artists = collect(
        db,
        &format!(
            "SELECT {ARTIST} AS name, COUNT(*) AS plays, SUM({LISTENED}) AS seconds, COUNT(DISTINCT t.id),
                    (SELECT x.cover FROM tracks x WHERE COALESCE(NULLIF(x.album_artist, ''), NULLIF(x.artist, '')) = {ARTIST}
                       AND x.cover IS NOT NULL ORDER BY x.play_count DESC LIMIT 1)
             FROM plays p JOIN tracks t ON t.id = p.track_id WHERE {audio} AND {ARTIST} IS NOT NULL
             GROUP BY name ORDER BY plays DESC, seconds DESC, name LIMIT {top}"
        ),
        params![start, end],
        |r| Ok(TopArtist { name: r.get(0)?, plays: r.get(1)?, seconds: r.get(2)?, tracks: r.get(3)?, cover: r.get(4)? }),
    )?;

    let top_albums = collect(
        db,
        &format!(
            "SELECT t.album AS album, {ARTIST} AS artist, COUNT(*) AS plays, SUM({LISTENED}) AS seconds,
                    MAX(t.cover), MIN(t.id)
             FROM plays p JOIN tracks t ON t.id = p.track_id WHERE {audio} AND t.album <> ''
             GROUP BY t.album, artist ORDER BY plays DESC, seconds DESC, album LIMIT {top}"
        ),
        params![start, end],
        |r| {
            Ok(TopAlbum {
                album: r.get(0)?,
                artist: r.get(1)?,
                plays: r.get(2)?,
                seconds: r.get(3)?,
                cover: r.get(4)?,
                track_id: r.get(5)?,
            })
        },
    )?;

    let genre_rows: Vec<(String, i64, f64)> = collect(
        db,
        &format!(
            "SELECT t.genre, COUNT(*), SUM({LISTENED}) FROM plays p JOIN tracks t ON t.id = p.track_id
             WHERE {audio} AND t.genre <> '' GROUP BY t.genre"
        ),
        params![start, end],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let mut genre_order: Vec<String> = Vec::new();
    let mut genres: HashMap<String, Genre> = HashMap::new();
    for (genre, plays, seconds) in genre_rows {
        for g in genre.split([',', ';', '/']).map(str::trim).filter(|g| !g.is_empty()) {
            let key = g.to_lowercase();
            let cur = genres.entry(key.clone()).or_insert_with(|| {
                genre_order.push(key.clone());
                Genre { name: g.to_string(), plays: 0, seconds: 0.0 }
            });
            cur.plays += plays;
            cur.seconds += seconds;
        }
    }
    let mut genres: Vec<Genre> = genre_order.into_iter().filter_map(|k| genres.remove(&k)).collect();
    genres.sort_by(|a, b| b.plays.cmp(&a.plays).then(b.seconds.partial_cmp(&a.seconds).unwrap_or(std::cmp::Ordering::Equal)));
    genres.truncate(8);

    let by_day = collect(
        db,
        &format!(
            "SELECT strftime('%Y-%m-%d', p.played_at / 1000, {LOCAL}) AS day, COUNT(*), SUM({LISTENED})
             FROM plays p JOIN tracks t ON t.id = p.track_id WHERE {audio} GROUP BY day ORDER BY day"
        ),
        params![start, end],
        |r| Ok(DayPlays { day: r.get(0)?, plays: r.get(1)?, seconds: r.get(2)? }),
    )?;

    let mut by_hour = vec![0.0; 24];
    for (h, s) in collect(
        db,
        &format!(
            "SELECT CAST(strftime('%H', p.played_at / 1000, {LOCAL}) AS INTEGER) AS h, SUM({LISTENED})
             FROM plays p JOIN tracks t ON t.id = p.track_id WHERE {audio} GROUP BY h"
        ),
        params![start, end],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)),
    )? {
        if let Some(slot) = by_hour.get_mut(h as usize) {
            *slot = s;
        }
    }
    let mut by_weekday = vec![0.0; 7];
    for (d, s) in collect(
        db,
        &format!(
            "SELECT CAST(strftime('%w', p.played_at / 1000, {LOCAL}) AS INTEGER) AS d, SUM({LISTENED})
             FROM plays p JOIN tracks t ON t.id = p.track_id WHERE {audio} GROUP BY d"
        ),
        params![start, end],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)),
    )? {
        by_weekday[((d + 6) % 7) as usize] = s;
    }

    // ─── Films, séries, animés (et vidéos locales) ───────────────────────────
    let (wt_seconds, wt_titles): (f64, i64) = db.query_row(
        "SELECT COALESCE(SUM(seconds), 0), COUNT(DISTINCT media_type || media_id) FROM watch_time WHERE day >= ?1 AND day <= ?2",
        params![from_day, to_day],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (local_seconds, local_titles): (f64, i64) = db.query_row(
        &format!(
            "SELECT COALESCE(SUM({LISTENED}), 0), COUNT(DISTINCT t.id)
             FROM plays p JOIN tracks t ON t.id = p.track_id WHERE t.kind = 'video' AND p.played_at >= ?1 AND p.played_at < ?2"
        ),
        params![start, end],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let finished: i64 = db.query_row(
        "SELECT COUNT(*) FROM watch_progress WHERE finished = 1 AND updated_at >= ?1 AND updated_at < ?2",
        params![start, end],
        |r| r.get(0),
    )?;
    let top_watched: Vec<(Watched, String)> = collect(
        db,
        "SELECT w.media_type, w.media_id, SUM(w.seconds) AS seconds,
           (SELECT title FROM watch_progress x WHERE x.media_type = w.media_type AND x.media_id = w.media_id ORDER BY updated_at DESC LIMIT 1),
           (SELECT poster FROM watch_progress x WHERE x.media_type = w.media_type AND x.media_id = w.media_id AND poster IS NOT NULL ORDER BY updated_at DESC LIMIT 1),
           (SELECT genres FROM watch_progress x WHERE x.media_type = w.media_type AND x.media_id = w.media_id ORDER BY updated_at DESC LIMIT 1)
         FROM watch_time w WHERE w.day >= ?1 AND w.day <= ?2
         GROUP BY w.media_type, w.media_id ORDER BY seconds DESC LIMIT 40",
        params![from_day, to_day],
        |r| {
            Ok((
                Watched { kind: r.get(0)?, id: r.get(1)?, seconds: r.get(2)?, title: r.get(3)?, poster: r.get(4)? },
                r.get::<_, Option<String>>(5)?.unwrap_or_else(|| "[]".into()),
            ))
        },
    )?;
    let top_local: Vec<Watched> = collect(
        db,
        &format!(
            "SELECT CAST(t.id AS TEXT), SUM({LISTENED}) AS seconds, t.title
             FROM plays p JOIN tracks t ON t.id = p.track_id WHERE t.kind = 'video' AND p.played_at >= ?1 AND p.played_at < ?2
             GROUP BY t.id ORDER BY seconds DESC LIMIT 10"
        ),
        params![start, end],
        |r| Ok(Watched { kind: "local".into(), id: r.get(0)?, seconds: r.get(1)?, title: r.get(2)?, poster: None }),
    )?;
    let mut top_video: Vec<Watched> = top_watched.iter().map(|(w, _)| w.clone()).chain(top_local).collect();
    top_video.sort_by(|a, b| b.seconds.partial_cmp(&a.seconds).unwrap_or(std::cmp::Ordering::Equal));
    top_video.truncate(10);

    let mut vg_order: Vec<String> = Vec::new();
    let mut vgenres: HashMap<String, VideoGenre> = HashMap::new();
    for (item, genres_json) in &top_watched {
        let list: Vec<Value> = serde_json::from_str(genres_json).unwrap_or_default();
        for g in list {
            let name = match g {
                Value::String(s) => s,
                other => other.to_string(),
            };
            let key = name.to_lowercase();
            let cur = vgenres.entry(key.clone()).or_insert_with(|| {
                vg_order.push(key.clone());
                VideoGenre { name, seconds: 0.0, titles: 0 }
            });
            cur.seconds += item.seconds;
            cur.titles += 1;
        }
    }
    let mut video_genres: Vec<VideoGenre> = vg_order.into_iter().filter_map(|k| vgenres.remove(&k)).collect();
    video_genres.sort_by(|a, b| b.seconds.partial_cmp(&a.seconds).unwrap_or(std::cmp::Ordering::Equal));
    video_genres.truncate(6);

    let video_by_day = collect(
        db,
        "SELECT day, SUM(seconds) FROM watch_time WHERE day >= ?1 AND day <= ?2 GROUP BY day ORDER BY day",
        params![from_day, to_day],
        |r| Ok(DaySeconds { day: r.get(0)?, seconds: r.get(1)? }),
    )?;

    // ─── Séries et périodes disponibles (sur tout l'historique) ──────────────
    let mut all_days: Vec<String> = collect(
        db,
        &format!("SELECT DISTINCT strftime('%Y-%m-%d', p.played_at / 1000, {LOCAL}) FROM plays p"),
        [],
        |r| r.get(0),
    )?;
    all_days.extend(collect(db, "SELECT DISTINCT day FROM watch_time WHERE seconds >= 30", [], |r| r.get(0))?);
    let set: BTreeSet<&String> = all_days.iter().collect();
    let mut years: Vec<i64> =
        set.iter().filter_map(|d| d.get(0..4)?.parse().ok()).collect::<BTreeSet<_>>().into_iter().collect();
    years.reverse();
    let mut months: Vec<String> =
        set.iter().filter_map(|d| d.get(0..7).map(str::to_string)).collect::<BTreeSet<_>>().into_iter().collect();
    months.reverse();
    let mut days: Vec<String> = set.iter().map(|d| (*d).clone()).collect();
    days.reverse();

    Ok(Stats {
        range: Range { from: from_day, to: to_day },
        years,
        months,
        days,
        streak: streaks(&all_days, now_ms),
        music: Music {
            plays,
            seconds,
            tracks,
            artists,
            albums,
            completed,
            top_tracks,
            top_artists,
            top_albums,
            genres,
            by_day,
            by_hour,
            by_weekday,
        },
        video: Video {
            seconds: wt_seconds + local_seconds,
            titles: wt_titles + local_titles,
            finished,
            top: top_video,
            genres: video_genres,
            by_day: video_by_day,
        },
    })
}

fn collect<T, P: rusqlite::Params>(
    db: &Connection,
    sql: &str,
    p: P,
    f: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> rusqlite::Result<Vec<T>> {
    let mut stmt = db.prepare(sql)?;
    let rows = stmt.query_map(p, f)?;
    rows.collect()
}

#[cfg(test)]
mod tests;

//! Base SQLite locale. Même schéma et mêmes migrations (`PRAGMA user_version`) que la version d'origine :
//! un `library.db` existant s'ouvre sans conversion, et une base créée ici reste lisible par l'ancienne version.

use std::path::Path;

use rusqlite::{Connection, Transaction};

pub type Db = Connection;

const MIGRATIONS: &[&str] = &[
    r#"
  CREATE TABLE tracks (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL DEFAULT 'audio',
    title TEXT NOT NULL,
    artist TEXT NOT NULL DEFAULT '',
    album TEXT NOT NULL DEFAULT '',
    album_artist TEXT NOT NULL DEFAULT '',
    genre TEXT NOT NULL DEFAULT '',
    year INTEGER,
    track_no INTEGER,
    disc_no INTEGER,
    duration REAL NOT NULL DEFAULT 0,
    bitrate INTEGER,
    codec TEXT,
    size INTEGER NOT NULL DEFAULT 0,
    mtime INTEGER NOT NULL DEFAULT 0,
    cover TEXT,
    width INTEGER,
    height INTEGER,
    added_at INTEGER NOT NULL,
    source_url TEXT,
    play_count INTEGER NOT NULL DEFAULT 0,
    last_played INTEGER
  );
  CREATE INDEX idx_tracks_kind ON tracks(kind);
  CREATE INDEX idx_tracks_added ON tracks(added_at);

  CREATE TABLE playlists (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'manual',
    rules TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
  );

  CREATE TABLE playlist_tracks (
    id INTEGER PRIMARY KEY,
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    added_at INTEGER NOT NULL
  );
  CREATE INDEX idx_pt_playlist ON playlist_tracks(playlist_id, position);

  CREATE TABLE plays (
    id INTEGER PRIMARY KEY,
    track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    played_at INTEGER NOT NULL,
    seconds REAL NOT NULL DEFAULT 0,
    duration REAL NOT NULL DEFAULT 0,
    completed INTEGER NOT NULL DEFAULT 0
  );
  CREATE INDEX idx_plays_at ON plays(played_at);
  CREATE INDEX idx_plays_track ON plays(track_id);
  "#,
    r#"
  CREATE TABLE media_cache (
    key TEXT PRIMARY KEY,
    payload TEXT NOT NULL,
    expires_at INTEGER NOT NULL
  );

  CREATE TABLE watch_progress (
    id INTEGER PRIMARY KEY,
    media_type TEXT NOT NULL,
    media_id TEXT NOT NULL,
    season INTEGER NOT NULL DEFAULT -1,
    episode INTEGER NOT NULL DEFAULT -1,
    position REAL NOT NULL DEFAULT 0,
    duration REAL NOT NULL DEFAULT 0,
    finished INTEGER NOT NULL DEFAULT 0,
    title TEXT NOT NULL DEFAULT '',
    poster TEXT,
    backdrop TEXT,
    year INTEGER,
    genres TEXT NOT NULL DEFAULT '[]',
    first_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (media_type, media_id, season, episode)
  );
  CREATE INDEX idx_wp_updated ON watch_progress(updated_at);

  CREATE TABLE watchlist (
    media_type TEXT NOT NULL,
    media_id TEXT NOT NULL,
    title TEXT NOT NULL,
    poster TEXT,
    backdrop TEXT,
    year INTEGER,
    rating REAL,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (media_type, media_id)
  );

  CREATE TABLE watch_time (
    day TEXT NOT NULL,
    media_type TEXT NOT NULL,
    media_id TEXT NOT NULL,
    season INTEGER NOT NULL DEFAULT -1,
    episode INTEGER NOT NULL DEFAULT -1,
    seconds REAL NOT NULL DEFAULT 0,
    PRIMARY KEY (day, media_type, media_id, season, episode)
  );
  "#,
    r#"
  CREATE TABLE lyrics (
    track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    synced TEXT,
    plain TEXT,
    source TEXT NOT NULL,
    instrumental INTEGER NOT NULL DEFAULT 0,
    fetched_at INTEGER NOT NULL
  );
  "#,
    r#"
  ALTER TABLE playlists ADD COLUMN description TEXT NOT NULL DEFAULT '';
  ALTER TABLE playlists ADD COLUMN cover TEXT;
  ALTER TABLE playlists ADD COLUMN shared INTEGER NOT NULL DEFAULT 0;
  "#,
    r#"
  ALTER TABLE tracks ADD COLUMN loudness REAL;
  ALTER TABLE tracks ADD COLUMN true_peak REAL;
  ALTER TABLE tracks ADD COLUMN loudness_checked INTEGER NOT NULL DEFAULT 0;
  "#,
];

/// Version du schéma que cette version sait lire.
pub const SCHEMA_VERSION: usize = MIGRATIONS.len();

/// Ouvre (et crée ou met à niveau) la base.
pub fn open(file: &Path) -> rusqlite::Result<Db> {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut db = Connection::open(file)?;
    configure(&mut db)?;
    Ok(db)
}

/// Base en mémoire (tests, aperçus).
pub fn open_in_memory() -> rusqlite::Result<Db> {
    let mut db = Connection::open_in_memory()?;
    configure(&mut db)?;
    Ok(db)
}

fn configure(db: &mut Db) -> rusqlite::Result<()> {
    db.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;")?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    let current: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (v, sql) in MIGRATIONS.iter().enumerate().skip(current.max(0) as usize) {
        let tx = db.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute_batch(&format!("PRAGMA user_version = {}", v + 1))?;
        tx.commit()?;
    }
    Ok(())
}

/// Exécute `f` dans une transaction (annulée si `f` échoue).
pub fn transaction<T>(db: &mut Db, f: impl FnOnce(&Transaction<'_>) -> rusqlite::Result<T>) -> rusqlite::Result<T> {
    let tx = db.transaction()?;
    let out = f(&tx)?;
    tx.commit()?;
    Ok(out)
}

/// Colonnes d'un morceau, dans l'ordre que lit [`crate::model::Track::from_row`].
pub const TRACK_COLUMNS: &str = "id, path, kind, title, artist, album, album_artist, genre, year, track_no, disc_no, \
    duration, bitrate, codec, size, cover, width, height, added_at, source_url, play_count, last_played, loudness, \
    true_peak, mtime";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_once() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("library.db");
        {
            let db = open(&f).unwrap();
            let v: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
            assert_eq!(v as usize, SCHEMA_VERSION);
        }
        let db = open(&f).unwrap();
        db.execute("INSERT INTO tracks (path, title, added_at) VALUES ('/a.mp3', 'a', 1)", []).unwrap();
        let n: i64 = db.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }
}

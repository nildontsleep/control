'use strict';

// Base locale SQLite (module intégré à Node/Electron : aucun module natif à compiler).

const { DatabaseSync } = require('node:sqlite');
const fs = require('node:fs');
const path = require('node:path');

const MIGRATIONS = [
  `
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
  `,
  `
  -- Films, séries et animés : cache des catalogues, progression, liste et temps de visionnage.
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
  `,
  `
  -- Paroles : texte brut et/ou synchronisé (LRC), avec la provenance. source = 'none' garde en mémoire une recherche vaine.
  CREATE TABLE lyrics (
    track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    synced TEXT,
    plain TEXT,
    source TEXT NOT NULL,
    instrumental INTEGER NOT NULL DEFAULT 0,
    fetched_at INTEGER NOT NULL
  );
  `,
  `
  -- Playlists personnalisables : description, pochette choisie (empreinte, sinon mosaïque) et visibilité pour les autres comptes (web).
  ALTER TABLE playlists ADD COLUMN description TEXT NOT NULL DEFAULT '';
  ALTER TABLE playlists ADD COLUMN cover TEXT;
  ALTER TABLE playlists ADD COLUMN shared INTEGER NOT NULL DEFAULT 0;
  `,
];

function open(file) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const db = new DatabaseSync(file);
  db.exec('PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;');
  const { user_version: current } = db.prepare('PRAGMA user_version').get();
  for (let v = current; v < MIGRATIONS.length; v++) {
    db.exec('BEGIN');
    try {
      db.exec(MIGRATIONS[v]);
      db.exec(`PRAGMA user_version = ${v + 1}`);
      db.exec('COMMIT');
    } catch (err) {
      db.exec('ROLLBACK');
      throw err;
    }
  }
  return db;
}

/** Exécute fn dans une transaction. */
function transaction(db, fn) {
  db.exec('BEGIN');
  try {
    const result = fn();
    db.exec('COMMIT');
    return result;
  } catch (err) {
    db.exec('ROLLBACK');
    throw err;
  }
}

const TRACK_COLUMNS = `
  id, path, kind, title, artist, album, album_artist AS albumArtist, genre, year,
  track_no AS trackNo, disc_no AS discNo, duration, bitrate, codec, size, cover,
  width, height, added_at AS addedAt, source_url AS sourceUrl,
  play_count AS playCount, last_played AS lastPlayed`;

const plain = (rows) => rows.map((r) => ({ ...r }));

module.exports = { open, transaction, TRACK_COLUMNS, plain };

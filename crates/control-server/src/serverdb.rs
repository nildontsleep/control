//! Base du serveur : comptes, sessions, réglages généraux. La musique de chaque compte est dans sa propre base.
//! Même schéma et mêmes migrations (`PRAGMA user_version`) que `server/serverdb.js` : une base existante se rouvre.

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::Connection;

pub type ServerDb = Arc<Mutex<Connection>>;

const MIGRATIONS: &[&str] = &[
    "
  CREATE TABLE users (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'user',
    disabled INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    last_login INTEGER
  );
  CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    user_agent TEXT,
    ip TEXT
  );
  CREATE INDEX idx_sessions_user ON sessions(user_id);
  CREATE TABLE kv (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
  );
  ",
    "
  ALTER TABLE users ADD COLUMN display_name TEXT NOT NULL DEFAULT '';
  ALTER TABLE users ADD COLUMN bio TEXT NOT NULL DEFAULT '';
  ALTER TABLE users ADD COLUMN avatar TEXT;
  ALTER TABLE users ADD COLUMN color TEXT;
  ALTER TABLE users ADD COLUMN folders TEXT NOT NULL DEFAULT '[]';
  CREATE TABLE shares (
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    viewer_id TEXT NOT NULL,
    browse INTEGER NOT NULL DEFAULT 1,
    compare INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (owner_id, viewer_id)
  );
  ",
    "
  ALTER TABLE users ADD COLUMN share_listening INTEGER NOT NULL DEFAULT 0;
  ",
    "
  ALTER TABLE users ADD COLUMN subsonic_secret TEXT;
  ALTER TABLE users ADD COLUMN subsonic_key_hash TEXT;
  CREATE INDEX idx_users_subsonic_key ON users(subsonic_key_hash);
  ",
];

pub fn open(file: &Path) -> rusqlite::Result<Connection> {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut db = Connection::open(file)?;
    db.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;")?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    let current: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (v, sql) in MIGRATIONS.iter().enumerate().skip(current.max(0) as usize) {
        let tx = db.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute_batch(&format!("PRAGMA user_version = {}", v + 1))?;
        tx.commit()?;
    }
    Ok(db)
}

/// Lit une valeur de la table `kv`.
pub fn kv_get(db: &Connection, key: &str) -> Option<String> {
    db.query_row("SELECT value FROM kv WHERE key = ?", [key], |r| r.get(0)).ok()
}

pub fn kv_set(db: &Connection, key: &str, value: &str) {
    if let Err(err) = db.execute(
        "INSERT INTO kv (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    ) {
        log::error!("Base du serveur : {err}");
    }
}

/// `VACUUM INTO` : copie cohérente, utilisable pendant que le serveur écrit.
pub fn vacuum_into(db: &Connection, file: &Path) -> rusqlite::Result<()> {
    let _ = std::fs::remove_file(file);
    let target = file.to_string_lossy().replace('\'', "''");
    db.execute_batch(&format!("VACUUM INTO '{target}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_run_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("server.db");
        {
            let db = open(&file).unwrap();
            kv_set(&db, "x", "1");
        }
        let db = open(&file).unwrap();
        let v: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, MIGRATIONS.len() as i64);
        assert_eq!(kv_get(&db, "x").as_deref(), Some("1"));
        db.execute("INSERT INTO users (id, username, password_hash, created_at) VALUES ('a', 'Nil', 'x', 1)", [])
            .unwrap();
        let n: i64 = db.query_row("SELECT COUNT(*) FROM users WHERE username = 'nil'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "noms sans tenir compte de la casse");
    }
}

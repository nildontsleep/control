'use strict';

// Base du serveur : comptes, sessions, réglages généraux. La musique de chaque compte est dans sa propre base.

const fs = require('node:fs');
const path = require('node:path');
const { DatabaseSync } = require('node:sqlite');

const MIGRATIONS = [
  `
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
  `,
  `
  -- Profil, dossiers de musique propres à chaque compte, et qui peut voir quoi.
  ALTER TABLE users ADD COLUMN display_name TEXT NOT NULL DEFAULT '';
  ALTER TABLE users ADD COLUMN bio TEXT NOT NULL DEFAULT '';
  ALTER TABLE users ADD COLUMN avatar TEXT;
  ALTER TABLE users ADD COLUMN color TEXT;
  ALTER TABLE users ADD COLUMN folders TEXT NOT NULL DEFAULT '[]';

  -- owner_id partage sa bibliothèque (browse) et ses statistiques (compare) avec viewer_id ; '*' = tout le monde.
  CREATE TABLE shares (
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    viewer_id TEXT NOT NULL,
    browse INTEGER NOT NULL DEFAULT 1,
    compare INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (owner_id, viewer_id)
  );
  `,
  `
  -- « En écoute » : le compte montre ce qu'il écoute à ceux qui peuvent parcourir sa bibliothèque.
  ALTER TABLE users ADD COLUMN share_listening INTEGER NOT NULL DEFAULT 0;
  `,
  `
  -- API compatible Subsonic : mot de passe d'application du compte (chiffré, la signature « token » en a besoin en clair
  -- côté serveur) et empreinte de ce secret pour la connexion par clé d'API.
  ALTER TABLE users ADD COLUMN subsonic_secret TEXT;
  ALTER TABLE users ADD COLUMN subsonic_key_hash TEXT;
  CREATE INDEX idx_users_subsonic_key ON users(subsonic_key_hash);
  `,
];

function openServerDb(file) {
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

module.exports = { openServerDb };

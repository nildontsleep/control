'use strict';

// Cache des réponses de catalogue dans SQLite : TTL, requêtes identiques regroupées, et repli sur une
// réponse périmée quand le réseau est coupé.

class Cache {
  constructor(db) {
    this.db = db;
    this.inflight = new Map();
    this.purge();
  }

  #row(key) {
    return this.db.prepare('SELECT payload, expires_at AS expiresAt FROM media_cache WHERE key = ?').get(key);
  }

  get(key) {
    const row = this.#row(key);
    return row && row.expiresAt > Date.now() ? JSON.parse(row.payload) : undefined;
  }

  stale(key) {
    const row = this.#row(key);
    return row ? JSON.parse(row.payload) : undefined;
  }

  set(key, value, ttl) {
    this.db.prepare(`INSERT INTO media_cache (key, payload, expires_at) VALUES (?, ?, ?)
      ON CONFLICT(key) DO UPDATE SET payload = excluded.payload, expires_at = excluded.expires_at`)
      .run(key, JSON.stringify(value), Date.now() + ttl);
  }

  /** Renvoie le cache s'il est frais ; sinon appelle fn, mémorise, et se rabat sur le périmé en cas d'échec réseau. */
  async memo(key, ttl, fn) {
    const hit = this.get(key);
    if (hit !== undefined) return hit;
    if (this.inflight.has(key)) return this.inflight.get(key);
    const promise = (async () => {
      try {
        const value = await fn();
        this.set(key, value, ttl);
        return value;
      } catch (err) {
        if (['network', 'timeout', 'ratelimit'].includes(err.code)) {
          const old = this.stale(key);
          if (old !== undefined) return old;
        }
        throw err;
      }
    })().finally(() => this.inflight.delete(key));
    this.inflight.set(key, promise);
    return promise;
  }

  purge() {
    this.db.prepare('DELETE FROM media_cache WHERE expires_at < ?').run(Date.now() - 7 * 24 * 3600 * 1000);
  }

  clear() {
    this.db.exec('DELETE FROM media_cache');
  }
}

module.exports = { Cache };

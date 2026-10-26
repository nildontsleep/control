'use strict';

// Comptes et sessions. Mots de passe hachés avec scrypt (module intégré à Node, aucune dépendance), sessions à jeton
// aléatoire dont seule l'empreinte est gardée : une déconnexion, un compte désactivé ou supprimé les coupe aussitôt.

const crypto = require('node:crypto');
const { promisify } = require('node:util');

const scrypt = promisify(crypto.scrypt);
const N = 16384;
const R = 8;
const P = 1;
const KEYLEN = 64;

const sha256 = (s) => crypto.createHash('sha256').update(s).digest('hex');
const USERNAME = /^[a-zA-Z0-9_.-]{3,24}$/;

async function hashPassword(password) {
  const salt = crypto.randomBytes(16);
  const hash = await scrypt(password, salt, KEYLEN, { N, r: R, p: P, maxmem: 64 * 1024 * 1024 });
  return `scrypt:${N}:${R}:${P}:${salt.toString('hex')}:${hash.toString('hex')}`;
}

async function verifyPassword(password, stored) {
  const [kind, n, r, p, salt, hash] = String(stored).split(':');
  if (kind !== 'scrypt' || !hash) return false;
  const expected = Buffer.from(hash, 'hex');
  const actual = await scrypt(password, Buffer.from(salt, 'hex'), expected.length, { N: Number(n), r: Number(r), p: Number(p), maxmem: 64 * 1024 * 1024 });
  return actual.length === expected.length && crypto.timingSafeEqual(actual, expected);
}

/** Limite les tentatives de connexion : par adresse, et par couple adresse + compte. */
class Throttle {
  constructor({ max = 8, windowMs = 10 * 60 * 1000 } = {}) {
    this.max = max;
    this.windowMs = windowMs;
    this.hits = new Map();
  }

  #prune(key, now) {
    const list = (this.hits.get(key) || []).filter((t) => now - t < this.windowMs);
    if (list.length) this.hits.set(key, list);
    else this.hits.delete(key);
    return list;
  }

  /** Secondes à attendre, ou 0 si la tentative est permise. */
  wait(key, now = Date.now()) {
    const list = this.#prune(key, now);
    if (list.length < this.max) return 0;
    return Math.ceil((list[0] + this.windowMs - now) / 1000);
  }

  fail(key, now = Date.now()) {
    const list = this.#prune(key, now);
    list.push(now);
    this.hits.set(key, list);
  }

  clear(key) {
    this.hits.delete(key);
  }
}

class Accounts {
  constructor(db, { sessionDays = 30 } = {}) {
    this.db = db;
    this.sessionMs = sessionDays * 24 * 3600 * 1000;
  }

  count() {
    return this.db.prepare('SELECT COUNT(*) AS n FROM users').get().n;
  }

  adminCount() {
    return this.db.prepare("SELECT COUNT(*) AS n FROM users WHERE role = 'admin' AND disabled = 0").get().n;
  }

  byName(name) {
    return this.db.prepare('SELECT * FROM users WHERE username = ?').get(String(name));
  }

  byId(id) {
    return this.db.prepare('SELECT * FROM users WHERE id = ?').get(String(id));
  }

  list() {
    return this.db.prepare(`
      SELECT u.id, u.username, u.role, u.disabled, u.created_at AS createdAt, u.last_login AS lastLogin,
             u.display_name AS displayName, u.avatar, u.color, u.folders,
             (SELECT COUNT(*) FROM sessions s WHERE s.user_id = u.id AND s.expires_at > ?) AS sessions
      FROM users u ORDER BY u.created_at`).all(Date.now()).map(({ folders, ...r }) => {
      let list = [];
      try { list = JSON.parse(folders || '[]'); } catch { /* liste illisible */ }
      return { ...r, disabled: !!r.disabled, folders: list };
    });
  }

  async create({ username, password, role = 'user' }) {
    if (!USERNAME.test(String(username))) throw httpError(400, 'invalid_username', 'Le nom doit faire 3 à 24 caractères : lettres, chiffres, « _ », « . » ou « - ».');
    if (typeof password !== 'string' || password.length < 8 || password.length > 200) throw httpError(400, 'invalid_password', 'Le mot de passe doit faire au moins 8 caractères.');
    if (this.byName(username)) throw httpError(409, 'username_taken', 'Ce nom est déjà pris.');
    const id = crypto.randomBytes(8).toString('hex');
    this.db.prepare('INSERT INTO users (id, username, password_hash, role, created_at) VALUES (?, ?, ?, ?, ?)')
      .run(id, username, await hashPassword(password), role === 'admin' ? 'admin' : 'user', Date.now());
    return this.byId(id);
  }

  async setPassword(id, password) {
    if (typeof password !== 'string' || password.length < 8 || password.length > 200) throw httpError(400, 'invalid_password', 'Le mot de passe doit faire au moins 8 caractères.');
    this.db.prepare('UPDATE users SET password_hash = ? WHERE id = ?').run(await hashPassword(password), id);
    this.revokeUser(id);
  }

  update(id, { role, disabled }) {
    const user = this.byId(id);
    if (!user) throw httpError(404, 'not_found', 'Compte introuvable.');
    const nextRole = role === 'admin' || role === 'user' ? role : user.role;
    const nextDisabled = disabled === undefined ? user.disabled : disabled ? 1 : 0;
    // Il doit toujours rester un administrateur actif.
    if ((user.role === 'admin' && !user.disabled) && (nextRole !== 'admin' || nextDisabled) && this.adminCount() <= 1) {
      throw httpError(409, 'last_admin', 'Il faut garder au moins un administrateur actif.');
    }
    this.db.prepare('UPDATE users SET role = ?, disabled = ? WHERE id = ?').run(nextRole, nextDisabled, id);
    if (nextDisabled) this.revokeUser(id);
  }

  remove(id) {
    const user = this.byId(id);
    if (!user) throw httpError(404, 'not_found', 'Compte introuvable.');
    if (user.role === 'admin' && !user.disabled && this.adminCount() <= 1) throw httpError(409, 'last_admin', 'Il faut garder au moins un administrateur actif.');
    this.db.prepare('DELETE FROM users WHERE id = ?').run(id);
  }

  /** Vérifie les identifiants ; renvoie le compte ou null (même durée de calcul si le nom n'existe pas). */
  async check(username, password) {
    const user = this.byName(username);
    const stored = user?.password_hash || `scrypt:${N}:${R}:${P}:${'0'.repeat(32)}:${'0'.repeat(KEYLEN * 2)}`;
    const ok = await verifyPassword(String(password), stored);
    return user && ok && !user.disabled ? user : null;
  }

  // ─── Sessions ──────────────────────────────────────────────────────────────
  open(userId, { userAgent = '', ip = '' } = {}) {
    const token = crypto.randomBytes(32).toString('base64url');
    const now = Date.now();
    this.db.prepare('INSERT INTO sessions (token_hash, user_id, created_at, expires_at, last_seen, user_agent, ip) VALUES (?, ?, ?, ?, ?, ?, ?)')
      .run(sha256(token), userId, now, now + this.sessionMs, now, String(userAgent).slice(0, 200), String(ip).slice(0, 60));
    this.db.prepare('UPDATE users SET last_login = ? WHERE id = ?').run(now, userId);
    return { token, maxAge: Math.floor(this.sessionMs / 1000) };
  }

  /** Le compte de cette session, ou null. La session se prolonge tant qu'elle sert. */
  lookup(token) {
    if (!token) return null;
    const hash = sha256(token);
    const now = Date.now();
    const row = this.db.prepare(`
      SELECT s.expires_at AS expiresAt, s.last_seen AS lastSeen, u.*
      FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token_hash = ?`).get(hash);
    if (!row || row.expiresAt < now || row.disabled) return null;
    if (now - row.lastSeen > 60 * 1000) {
      this.db.prepare('UPDATE sessions SET last_seen = ?, expires_at = ? WHERE token_hash = ?').run(now, now + this.sessionMs, hash);
    }
    return row;
  }

  close(token) {
    if (token) this.db.prepare('DELETE FROM sessions WHERE token_hash = ?').run(sha256(token));
  }

  revokeUser(id) {
    this.db.prepare('DELETE FROM sessions WHERE user_id = ?').run(id);
  }

  purge() {
    this.db.prepare('DELETE FROM sessions WHERE expires_at < ?').run(Date.now());
  }
}

function httpError(status, code, message) {
  return Object.assign(new Error(message), { status, code });
}

module.exports = { Accounts, Throttle, hashPassword, verifyPassword, httpError, USERNAME };

'use strict';

// Profils et partages : le profil de chaque compte, et qui peut parcourir la bibliothèque d'un compte (browse) ou
// comparer ses récaps avec lui (compare). Un compte partage avec un autre compte précis, ou avec tout le monde ('*').

const { httpError } = require('./auth');
const images = require('../src/main/images');

const COLOR = /^#[0-9a-f]{6}$/i;
const EVERYONE = '*';

class Social {
  /**
   * accounts : Accounts ; avatarsDir : dossier des avatars (JPEG nommés par empreinte).
   */
  constructor(db, accounts, { avatarsDir }) {
    this.db = db;
    this.accounts = accounts;
    this.avatarsDir = avatarsDir;
  }

  /** Ce que n'importe quel compte connecté voit d'un autre : nom, avatar, présentation. */
  profile(row) {
    if (!row) return null;
    return {
      id: row.id,
      username: row.username,
      displayName: row.display_name || '',
      name: row.display_name || row.username,
      bio: row.bio || '',
      avatar: row.avatar || null,
      color: row.color || null,
      role: row.role,
      createdAt: row.created_at,
    };
  }

  profiles() {
    return this.db.prepare('SELECT * FROM users WHERE disabled = 0 ORDER BY username COLLATE NOCASE').all().map((r) => this.profile(r));
  }

  setProfile(id, patch = {}) {
    const row = this.accounts.byId(id);
    if (!row) throw httpError(404, 'not_found', 'Compte introuvable.');
    const next = {
      display_name: row.display_name,
      bio: row.bio,
      color: row.color,
      avatar: row.avatar,
    };
    if ('displayName' in patch) next.display_name = String(patch.displayName ?? '').trim().slice(0, 40);
    if ('bio' in patch) next.bio = String(patch.bio ?? '').trim().slice(0, 280);
    if ('color' in patch) {
      const color = String(patch.color ?? '').trim();
      if (color && !COLOR.test(color)) throw httpError(400, 'invalid_color', 'Couleur invalide.');
      next.color = color || null;
    }
    if (patch.avatar === null) next.avatar = null;
    else if (patch.avatar && typeof patch.avatar === 'object') {
      const buffer = images.toBuffer(patch.avatar.data);
      if (!buffer) throw httpError(400, 'invalid_image', 'Image refusée (JPEG, PNG ou WebP, 12 Mo au plus).');
      const jpeg = images.squareJpeg(buffer, 320);
      if (!jpeg) throw httpError(400, 'invalid_image', 'Ce format d’image n’est pas pris en charge (utilisez JPEG ou PNG).');
      next.avatar = images.saveByHash(this.avatarsDir, jpeg);
    }
    this.db.prepare('UPDATE users SET display_name = ?, bio = ?, color = ?, avatar = ? WHERE id = ?')
      .run(next.display_name, next.bio, next.color, next.avatar, id);
    return this.profile(this.accounts.byId(id));
  }

  // ─── Partages ──────────────────────────────────────────────────────────────
  /** Les partages d'un compte : [{ viewerId, browse, compare }]. */
  grantsOf(ownerId) {
    return this.db.prepare('SELECT viewer_id AS viewerId, browse, compare FROM shares WHERE owner_id = ? ORDER BY viewer_id').all(ownerId)
      .map((r) => ({ viewerId: r.viewerId, browse: !!r.browse, compare: !!r.compare }));
  }

  /** Remplace les partages d'un compte. Les entrées sans aucun droit, vers lui-même ou vers un compte inconnu sont ignorées. */
  setGrants(ownerId, list) {
    if (!this.accounts.byId(ownerId)) throw httpError(404, 'not_found', 'Compte introuvable.');
    const known = new Set(this.accounts.list().map((u) => u.id));
    const clean = new Map();
    for (const item of Array.isArray(list) ? list.slice(0, 500) : []) {
      const viewerId = String(item?.viewerId ?? '');
      if (viewerId === ownerId || (viewerId !== EVERYONE && !known.has(viewerId))) continue;
      const browse = !!item.browse;
      const compare = !!item.compare;
      if (browse || compare) clean.set(viewerId, { browse, compare });
    }
    this.db.exec('BEGIN');
    try {
      this.db.prepare('DELETE FROM shares WHERE owner_id = ?').run(ownerId);
      const insert = this.db.prepare('INSERT INTO shares (owner_id, viewer_id, browse, compare) VALUES (?, ?, ?, ?)');
      for (const [viewerId, g] of clean) insert.run(ownerId, viewerId, g.browse ? 1 : 0, g.compare ? 1 : 0);
      this.db.exec('COMMIT');
    } catch (err) {
      this.db.exec('ROLLBACK');
      throw err;
    }
    return this.grantsOf(ownerId);
  }

  /** Ce que `viewer` (une ligne de la table users) peut faire avec le compte `owner`. */
  access(viewer, owner) {
    if (!viewer || !owner) return { browse: false, compare: false };
    if (viewer.id === owner.id) return { browse: true, compare: true, self: true };
    if (owner.disabled) return { browse: false, compare: false };
    let browse = viewer.role === 'admin'; // l'administrateur parcourt toutes les bibliothèques, pas les statistiques
    let compare = false;
    for (const r of this.db.prepare('SELECT browse, compare FROM shares WHERE owner_id = ? AND viewer_id IN (?, ?)').all(owner.id, viewer.id, EVERYONE)) {
      browse ||= !!r.browse;
      compare ||= !!r.compare;
    }
    return { browse, compare };
  }

  /** Les autres comptes, avec ce que `viewer` peut en faire. */
  others(viewer) {
    return this.db.prepare('SELECT * FROM users WHERE disabled = 0 AND id <> ? ORDER BY username COLLATE NOCASE').all(viewer.id)
      .map((row) => ({ ...this.profile(row), access: this.access(viewer, row) }));
  }

  /** Supprime tout ce qui concerne un compte supprimé (les partages vers lui). */
  forget(id) {
    this.db.prepare('DELETE FROM shares WHERE viewer_id = ?').run(id);
  }
}

module.exports = { Social, EVERYONE };

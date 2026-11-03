'use strict';

// « En écoute » : ce que les comptes écoutent en ce moment. Un compte ne le montre que s'il l'a choisi (désactivé par
// défaut), et seulement à ceux avec qui il partage sa bibliothèque (un partage explicite, pas l'accès de l'administrateur).
// Rien n'est gardé sur le disque : c'est de la mémoire vive, qui s'efface à la fin du morceau.

const GRACE_MS = 60 * 1000; // une pause un peu longue ne fait pas disparaître le morceau tout de suite

class Presence {
  constructor(hub) {
    this.hub = hub;
    this.now = new Map(); // ownerId → { track, startedAt, expiresAt, playId }
    this.timer = setInterval(() => this.sweep(), 15 * 1000);
    this.timer.unref?.();
  }

  stop() {
    clearInterval(this.timer);
  }

  enabled(ownerId) {
    return !!this.hub.db.prepare('SELECT share_listening AS on_ FROM users WHERE id = ? AND disabled = 0').get(ownerId)?.on_;
  }

  /** Les comptes à qui `ownerId` montre ce qu'il écoute : ceux qui ont un partage de bibliothèque explicite (ou « tout le monde »). */
  viewers(ownerId) {
    const rows = this.hub.db.prepare('SELECT viewer_id AS id FROM shares WHERE owner_id = ? AND browse = 1').all(ownerId);
    if (rows.some((r) => r.id === '*')) return this.hub.db.prepare('SELECT id FROM users WHERE disabled = 0 AND id <> ?').all(ownerId).map((r) => r.id);
    return rows.map((r) => r.id).filter((id) => id !== ownerId);
  }

  visibleTo(viewerId, ownerId) {
    return viewerId !== ownerId && !!this.hub.db.prepare("SELECT 1 FROM shares WHERE owner_id = ? AND browse = 1 AND viewer_id IN (?, '*')").get(ownerId, viewerId);
  }

  #view(ownerId, rec) {
    const t = rec.track;
    return { id: ownerId, now: { trackId: t.id, title: t.title, artist: t.artist, album: t.album, cover: t.cover, duration: t.duration, startedAt: rec.startedAt } };
  }

  #notify(ownerId, payload) {
    for (const viewerId of this.viewers(ownerId)) this.hub.entries.get(viewerId)?.emit('social:listening', payload);
  }

  /** Un morceau commence chez ce compte. */
  start(ownerId, playId, track) {
    if (!track || track.kind !== 'audio' || !this.enabled(ownerId)) return;
    const rec = { track, playId, startedAt: Date.now(), expiresAt: Date.now() + (track.duration || 240) * 1000 + GRACE_MS };
    this.now.set(ownerId, rec);
    this.#notify(ownerId, this.#view(ownerId, rec));
  }

  /** Il s'arrête (fin du morceau ou passage au suivant : un nouveau `start` suit aussitôt). */
  end(ownerId, playId) {
    const rec = this.now.get(ownerId);
    if (!rec || (playId !== undefined && rec.playId !== playId)) return;
    this.clear(ownerId);
  }

  clear(ownerId) {
    if (!this.now.delete(ownerId)) return;
    this.#notify(ownerId, { id: ownerId, now: null });
  }

  sweep() {
    const t = Date.now();
    for (const [ownerId, rec] of this.now) if (rec.expiresAt < t) this.clear(ownerId);
  }

  /** Ce que `viewerId` voit : { [ownerId]: { trackId, title, artist, … } }. */
  snapshotFor(viewerId) {
    const out = {};
    for (const [ownerId, rec] of this.now) {
      if (rec.expiresAt >= Date.now() && this.visibleTo(viewerId, ownerId)) out[ownerId] = this.#view(ownerId, rec).now;
    }
    return out;
  }
}

module.exports = { Presence };

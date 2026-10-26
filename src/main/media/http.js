'use strict';

// Requêtes JSON vers les catalogues (TMDB, AniList) : délai, erreurs typées, aucune dépendance.

const { net } = require('../platform');

class MediaError extends Error {
  constructor(code, message, status = 0) {
    super(message);
    this.code = code;
    this.status = status;
  }
}

function codeForStatus(status) {
  if (status === 401 || status === 403) return 'auth';
  if (status === 404) return 'notfound';
  if (status === 429) return 'ratelimit';
  return 'http';
}

async function fetchJson(url, { method = 'GET', headers = {}, body, timeout = 15000 } = {}) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeout);
  try {
    const res = await net.fetch(url, { method, headers, body, signal: controller.signal });
    const text = await res.text();
    let data = null;
    try {
      data = text ? JSON.parse(text) : null;
    } catch { /* corps non JSON */ }
    if (!res.ok) {
      const detail = data?.status_message || data?.errors?.[0]?.message || `HTTP ${res.status}`;
      throw new MediaError(codeForStatus(res.status), detail, res.status);
    }
    return data;
  } catch (err) {
    if (err instanceof MediaError) throw err;
    throw new MediaError(err.name === 'AbortError' ? 'timeout' : 'network', err.message || 'Réseau indisponible');
  } finally {
    clearTimeout(timer);
  }
}

module.exports = { MediaError, fetchJson };

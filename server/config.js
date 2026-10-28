'use strict';

// Configuration du serveur web : variables d'environnement (voir le README, section « Version web »).

const path = require('node:path');

const flag = (v, fallback = false) => (v === undefined || v === '' ? fallback : /^(1|true|yes|on)$/i.test(String(v)));

function loadConfig(env = process.env) {
  return {
    port: Number(env.PORT || env.CONTROL_PORT) || 5180,
    host: env.CONTROL_HOST || '0.0.0.0',
    // Tout ce que le serveur écrit : comptes, réglages, bibliothèques, pochettes, outils.
    dataDir: path.resolve(env.CONTROL_DATA || path.join(process.cwd(), 'data')),
    // Dossiers de musique proposés au premier lancement (séparés par « : » sous Linux, « ; » sous Windows).
    libraryFolders: String(env.CONTROL_LIBRARY || '').split(path.delimiter).map((s) => s.trim()).filter(Boolean),
    // Derrière un reverse proxy (nginx, Caddy, Traefik…) : fait confiance à X-Forwarded-For / X-Forwarded-Proto.
    trustProxy: flag(env.CONTROL_TRUST_PROXY),
    // « auto » : cookie Secure dès que la connexion est en HTTPS.
    cookieSecure: String(env.CONTROL_COOKIE_SECURE || 'auto').toLowerCase(),
    // Minutes entre deux analyses complètes des dossiers (en plus de la surveillance des fichiers).
    rescanMinutes: Number(env.CONTROL_RESCAN_MINUTES) || 30,
    // Durée d'une session de connexion, en jours.
    sessionDays: Number(env.CONTROL_SESSION_DAYS) || 30,
    // Clé TMDB (films et séries) fournie d'office aux comptes qui n'ont pas la leur.
    tmdbKey: env.CONTROL_TMDB_KEY || '',
    // Si défini, la création du premier compte (administrateur) exige ce code : utile tant que le serveur est exposé.
    setupToken: env.CONTROL_SETUP_TOKEN || '',
    // Mise à jour automatique : « off », « notify » ou « auto » force le réglage (vide : choisi par l'administrateur dans /admin).
    update: ['off', 'notify', 'auto'].includes(String(env.CONTROL_UPDATE || '').toLowerCase()) ? String(env.CONTROL_UPDATE).toLowerCase() : '',
    // Jeton GitHub en lecture, si le dépôt des releases est privé.
    updateToken: env.CONTROL_UPDATE_TOKEN || '',
  };
}

module.exports = { loadConfig };

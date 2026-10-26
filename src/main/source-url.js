'use strict';

// Adresse d'un serveur control saisie à la main : origines à essayer (module sans Electron, testé avec les tests du serveur).

/** Adresses à essayer pour ce que l'utilisateur a saisi : sans schéma, https d'abord (http d'abord pour une adresse locale). */
function candidates(input) {
  const raw = String(input || '').trim().replace(/\/+$/, '');
  if (!raw) throw new Error('Saisissez l’adresse du serveur.');
  const make = (text) => {
    let url;
    try {
      url = new URL(text);
    } catch {
      throw new Error('Adresse invalide.');
    }
    if (!/^https?:$/.test(url.protocol)) throw new Error('Seules les adresses http et https sont acceptées.');
    return url.origin;
  };
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(raw)) return [make(raw)];
  const host = raw.split(/[/?#]/)[0];
  const local = /^(localhost|\[.*\]|\d{1,3}(\.\d{1,3}){3}|[^.:]+|.*\.(local|lan|home|internal))(:\d+)?$/i.test(host);
  return (local ? ['http', 'https'] : ['https', 'http']).map((scheme) => make(`${scheme}://${raw}`));
}

module.exports = { candidates };

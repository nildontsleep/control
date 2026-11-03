'use strict';

// Liens que le serveur accepte de faire télécharger par yt-dlp : http(s) uniquement, et jamais une adresse du réseau
// interne (le serveur ne doit pas servir de relais vers 127.0.0.1, le réseau local ou les services de métadonnées d'un
// hébergeur). C'est une première barrière : une redirection ou un changement de DNS après la vérification y échappe,
// l'isolation réseau du conteneur reste la vraie protection. L'application de bureau n'est pas concernée.

const dns = require('node:dns').promises;
const net = require('node:net');

const blocked = new net.BlockList();
for (const [address, prefix] of [
  ['0.0.0.0', 8], ['10.0.0.0', 8], ['100.64.0.0', 10], ['127.0.0.0', 8], ['169.254.0.0', 16], ['172.16.0.0', 12],
  ['192.0.0.0', 24], ['192.0.2.0', 24], ['192.168.0.0', 16], ['198.18.0.0', 15], ['198.51.100.0', 24],
  ['203.0.113.0', 24], ['224.0.0.0', 4], ['240.0.0.0', 4],
]) blocked.addSubnet(address, prefix, 'ipv4');
for (const [address, prefix] of [['::', 128], ['::1', 128], ['fc00::', 7], ['fe80::', 10], ['ff00::', 8], ['2001:db8::', 32], ['64:ff9b::', 96], ['2002::', 16]]) blocked.addSubnet(address, prefix, 'ipv6');

/** L'adresse (IPv4 ou IPv6) est-elle privée, locale ou réservée ? */
function isPrivateAddress(address) {
  const family = net.isIP(address);
  return !family || blocked.check(address, family === 6 ? 'ipv6' : 'ipv4');
}

/** Vérifie un lien : renvoie l'adresse nettoyée, ou lève une erreur lisible. `resolve` sert aux tests. */
async function checkUrl(input, { resolve = (host) => dns.lookup(host, { all: true, verbatim: true }) } = {}) {
  const text = typeof input === 'string' ? input.trim() : '';
  if (!text || text.length > 2048) throw new Error('Adresse invalide.');
  let url;
  try {
    url = new URL(text);
  } catch {
    throw new Error('Adresse invalide.');
  }
  if (!['http:', 'https:'].includes(url.protocol)) throw new Error('Seules les adresses http et https sont acceptées.');
  if (url.username || url.password) throw new Error('Adresse invalide.');
  const host = url.hostname.replace(/^\[|\]$/g, '');
  if (net.isIP(host)) {
    if (isPrivateAddress(host)) throw new Error('Cette adresse appartient au réseau du serveur : refusée.');
    return url.href;
  }
  let addresses;
  try {
    addresses = await resolve(host);
  } catch {
    throw new Error('Adresse introuvable.');
  }
  if (!addresses.length || addresses.some((a) => isPrivateAddress(a.address))) throw new Error('Cette adresse appartient au réseau du serveur : refusée.');
  return url.href;
}

module.exports = { checkUrl, isPrivateAddress };

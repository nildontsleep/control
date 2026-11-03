#!/usr/bin/env node
'use strict';

// Signature des archives de mise à jour (Ed25519). L'empreinte SHA-256 publiée par GitHub protège d'un fichier abîmé,
// pas d'un dépôt ou d'un compte GitHub compromis : la signature, faite avec une clé qui ne quitte jamais les secrets
// de la CI, l'est. Les serveurs ne font confiance qu'aux archives signées par une clé de `update-key.json`.
//
//   node server/sign.js keygen                     crée une paire de clés (la privée va dans le secret WEB_SIGNING_KEY)
//   node server/sign.js sign <control-web-X.tgz>   écrit <archive>.sig (clé privée : variable WEB_SIGNING_KEY)
//   node server/sign.js verify <archive> <public>  contrôle une archive (clé publique en base64)

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');

const SPKI_PREFIX = Buffer.from('302a300506032b6570032100', 'hex'); // en-tête DER d'une clé publique Ed25519

/** Ce que la signature couvre : le nom de la version, puis l'archive (une archive ancienne ne peut pas se faire passer pour une nouvelle). */
const message = (version, archive) => Buffer.concat([Buffer.from(`control-web ${version}\n`), archive]);

function keygen() {
  const { publicKey, privateKey } = crypto.generateKeyPairSync('ed25519');
  return {
    publicKey: publicKey.export({ format: 'der', type: 'spki' }).subarray(SPKI_PREFIX.length).toString('base64'),
    privateKey: privateKey.export({ format: 'der', type: 'pkcs8' }).toString('base64'),
  };
}

function sign(version, archive, privateKey) {
  const key = crypto.createPrivateKey({ key: Buffer.from(privateKey, 'base64'), format: 'der', type: 'pkcs8' });
  return crypto.sign(null, message(version, archive), key).toString('base64');
}

/** `keys` : clés publiques (base64) ; vrai si l'une d'elles a signé exactement cette archive de cette version. */
function verify(version, archive, signature, keys) {
  let sig;
  try {
    sig = Buffer.from(String(signature).trim(), 'base64');
  } catch {
    return false;
  }
  if (sig.length !== 64) return false;
  return keys.some((k) => {
    try {
      const raw = Buffer.from(k, 'base64');
      if (raw.length !== 32) return false;
      const key = crypto.createPublicKey({ key: Buffer.concat([SPKI_PREFIX, raw]), format: 'der', type: 'spki' });
      return crypto.verify(null, message(version, archive), key, sig);
    } catch {
      return false;
    }
  });
}

/** Clés publiques de confiance (server/update-key.json). Vide : les mises à jour ne sont pas signées. */
function trustedKeys(file = path.join(__dirname, 'update-key.json')) {
  try {
    const keys = JSON.parse(fs.readFileSync(file, 'utf8')).publicKeys;
    return Array.isArray(keys) ? keys.filter((k) => typeof k === 'string' && k.trim()) : [];
  } catch {
    return [];
  }
}

const versionOf = (file) => /^control-web-(\d+\.\d+\.\d+)\.tgz$/.exec(path.basename(file))?.[1];

if (require.main === module) {
  const [command, file, extra] = process.argv.slice(2);
  try {
    if (command === 'keygen') {
      const { publicKey, privateKey } = keygen();
      console.log(`Clé publique (à mettre dans server/update-key.json, champ "publicKeys") :\n${publicKey}\n`);
      console.log(`Clé privée (à garder secrète : secret GitHub WEB_SIGNING_KEY) :\n${privateKey}`);
    } else if (command === 'sign') {
      const version = versionOf(file || '');
      if (!version) throw new Error('Nom attendu : control-web-<version>.tgz');
      if (!process.env.WEB_SIGNING_KEY) throw new Error('Variable WEB_SIGNING_KEY absente.');
      fs.writeFileSync(`${file}.sig`, `${sign(version, fs.readFileSync(file), process.env.WEB_SIGNING_KEY.trim())}\n`);
      console.log(`${path.basename(file)}.sig écrit.`);
    } else if (command === 'verify') {
      const version = versionOf(file || '');
      if (!version || !extra) throw new Error('Utilisation : verify control-web-<version>.tgz <clé publique>');
      const ok = verify(version, fs.readFileSync(file), fs.readFileSync(`${file}.sig`, 'utf8'), [extra]);
      console.log(ok ? 'Signature valide.' : 'SIGNATURE INVALIDE.');
      process.exit(ok ? 0 : 1);
    } else {
      console.log('Utilisation : node server/sign.js keygen | sign <archive> | verify <archive> <clé publique>');
      process.exit(command ? 1 : 0);
    }
  } catch (err) {
    console.error(err.message);
    process.exit(1);
  }
}

module.exports = { keygen, sign, verify, trustedKeys };

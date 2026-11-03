'use strict';

// Secrets que le serveur doit pouvoir relire (le mot de passe d'application Subsonic : la signature « token » de
// l'API a besoin du secret en clair). Chiffrés avec une clé aléatoire gardée à part dans le dossier de données
// (secret.key, lisible par le serveur seul) : la base seule ne les révèle pas.

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');

class Secrets {
  constructor(dataDir) {
    this.file = path.join(dataDir, 'secret.key');
    this.key = null;
  }

  #load() {
    if (this.key) return this.key;
    try {
      this.key = Buffer.from(fs.readFileSync(this.file, 'utf8').trim(), 'base64');
      if (this.key.length === 32) return this.key;
    } catch { /* première utilisation */ }
    this.key = crypto.randomBytes(32);
    fs.mkdirSync(path.dirname(this.file), { recursive: true });
    fs.writeFileSync(this.file, `${this.key.toString('base64')}\n`, { mode: 0o600 });
    return this.key;
  }

  seal(text) {
    const iv = crypto.randomBytes(12);
    const cipher = crypto.createCipheriv('aes-256-gcm', this.#load(), iv);
    const body = Buffer.concat([cipher.update(String(text), 'utf8'), cipher.final()]);
    return Buffer.concat([iv, cipher.getAuthTag(), body]).toString('base64');
  }

  open(sealed) {
    try {
      const blob = Buffer.from(String(sealed), 'base64');
      const decipher = crypto.createDecipheriv('aes-256-gcm', this.#load(), blob.subarray(0, 12));
      decipher.setAuthTag(blob.subarray(12, 28));
      return Buffer.concat([decipher.update(blob.subarray(28)), decipher.final()]).toString('utf8');
    } catch {
      return null;
    }
  }
}

module.exports = { Secrets };

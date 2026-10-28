#!/usr/bin/env node
'use strict';

// Construit l'archive du serveur web, `control-web-<version>.tgz` : ce que la mise à jour automatique télécharge.
// Elle contient exactement ce que l'image Docker embarque (package.json, package-lock.json, server/ sans ses tests,
// src/), sans les dépendances : elles viennent de l'installation d'origine (ou de `npm ci` quand elles changent).
// Utilisation : `node server/bundle.js [dossier de sortie]` (la CI le fait pour chaque release).

const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { pack } = require('./tar');

const ROOT = path.resolve(__dirname, '..');
const SINGLE = ['package.json', 'package-lock.json', 'THIRD-PARTY.md'];
const TREES = ['server', 'src'];
const SKIP = (rel) => rel === 'server/test' || rel.startsWith('server/test/') || /(^|\/)\.[^/]+$/.test(rel);

function collect(root = ROOT) {
  const files = [];
  const walk = (rel) => {
    for (const entry of fs.readdirSync(path.join(root, rel), { withFileTypes: true })) {
      const next = `${rel}/${entry.name}`;
      if (SKIP(next)) continue;
      if (entry.isDirectory()) walk(next);
      else if (entry.isFile()) files.push(next);
    }
  };
  for (const name of SINGLE) if (fs.existsSync(path.join(root, name))) files.push(name);
  for (const tree of TREES) walk(tree);
  return files.sort().map((name) => {
    const full = path.join(root, name);
    return { name, data: fs.readFileSync(full), mode: fs.statSync(full).mode & 0o111 ? 0o755 : 0o644 };
  });
}

/** { name, data, sha256 } : le nom de fichier de la release et son contenu. */
function build(root = ROOT) {
  const { version } = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'));
  const data = pack(collect(root));
  return { version, name: `control-web-${version}.tgz`, data, sha256: crypto.createHash('sha256').update(data).digest('hex') };
}

if (require.main === module) {
  const out = path.resolve(process.argv[2] || 'dist');
  const bundle = build();
  fs.mkdirSync(out, { recursive: true });
  fs.writeFileSync(path.join(out, bundle.name), bundle.data);
  console.log(`${bundle.name} : ${(bundle.data.length / 1024).toFixed(0)} Ko, sha256 ${bundle.sha256}`);
}

module.exports = { build, collect };

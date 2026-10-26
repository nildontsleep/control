#!/usr/bin/env node
'use strict';

// control, version web : un serveur Node qui héberge la bibliothèque et l'application, accessible depuis un navigateur.
// Lancement : `npm run web` (voir la section « Version web » du README pour la configuration).

const { loadConfig } = require('./config');
const { Hub } = require('./hub');
const { createApp } = require('./app');

async function main() {
  const config = loadConfig();
  const hub = new Hub(config);
  const server = createApp(hub, config);
  hub.start();

  server.listen(config.port, config.host, () => {
    const shown = config.host === '0.0.0.0' || config.host === '::' ? 'localhost' : config.host;
    console.log(`control (web) écoute sur http://${shown}:${config.port}`);
    console.log(`Données : ${config.dataDir}`);
    if (!hub.accounts.count()) {
      console.log(`Aucun compte pour l'instant : ouvrez http://${shown}:${config.port}/setup pour créer l'administrateur.`);
      if (!config.setupToken) console.log('Astuce : définissez CONTROL_SETUP_TOKEN pour que seul vous puissiez créer ce premier compte tant que le serveur est exposé.');
    }
  });

  let closing = false;
  const stop = () => {
    if (closing) return;
    closing = true;
    console.log('Arrêt…');
    server.close();
    hub.close();
    setTimeout(() => process.exit(0), 300).unref();
  };
  process.on('SIGINT', stop);
  process.on('SIGTERM', stop);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});

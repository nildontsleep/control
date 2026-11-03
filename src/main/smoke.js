'use strict';

// Test de démarrage (CONTROL_SMOKE=1, utilisé par la CI) : l'application démarre, l'interface se construit sans erreur,
// la fenêtre des réglages s'ouvre. Sort avec le code 0 si tout va bien, 1 sinon, en listant ce qui a échoué.

const TIMEOUT_MS = 60 * 1000;

/** Ce que la page vérifie une fois chargée (exécuté dans la fenêtre). */
const CHECK = `(async () => {
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const out = { items: document.querySelectorAll('.sb-item').length, title: document.title };
  document.querySelector('.sb-settings')?.click();
  await wait(900);
  out.settings = !!document.querySelector('.sheet');
  out.tabs = document.querySelectorAll('.sheet [role="tab"]').length;
  document.querySelector('.sheet .sheet-close')?.click();
  await wait(300);
  out.api = ['library', 'playlists', 'stats', 'update', 'downloads', 'media'].every((k) => !!window.control?.[k]);
  return out;
})()`;

function attach(win, app) {
  const problems = [];
  const finish = (code) => {
    if (code) console.error(`SMOKE ÉCHEC :\n- ${problems.join('\n- ')}`);
    else console.log('SMOKE OK');
    app.exit(code);
  };
  const wc = win.webContents;
  // Les erreurs de la console de la page (Electron ≥ 35 : un seul objet ; avant : arguments séparés).
  wc.on('console-message', (...args) => {
    const detail = args.length === 1 ? args[0] : { level: args[1], message: args[2] };
    const level = typeof detail.level === 'string' ? detail.level : ['verbose', 'info', 'warning', 'error'][detail.level] || 'info';
    if (level === 'error') problems.push(`console : ${String(detail.message).slice(0, 300)}`);
  });
  wc.on('render-process-gone', (e, details) => { problems.push(`rendu arrêté : ${details.reason}`); finish(1); });
  wc.on('did-fail-load', (e, code, description) => { problems.push(`chargement : ${description}`); finish(1); });
  wc.on('preload-error', (e, file, error) => { problems.push(`préchargement : ${error?.message || error}`); });

  setTimeout(() => { problems.push('délai dépassé'); finish(1); }, TIMEOUT_MS).unref();
  wc.once('did-finish-load', () => {
    setTimeout(async () => {
      try {
        const result = await wc.executeJavaScript(CHECK);
        console.log('SMOKE', JSON.stringify(result));
        if (result.items < 6) problems.push(`barre latérale incomplète (${result.items} entrées)`);
        if (!result.settings || result.tabs < 4) problems.push('la fenêtre des réglages ne s’ouvre pas');
        if (!result.api) problems.push('window.control incomplet');
      } catch (err) {
        problems.push(`vérification : ${err.message}`);
      }
      finish(problems.length ? 1 : 0);
    }, 4000);
  });
}

module.exports = { attach };

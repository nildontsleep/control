// Langue des pages hors application (connexion, installation) : celle du navigateur, français ou anglais.
(() => {
  'use strict';
  const fr = /^fr/i.test(navigator.language || '');
  window.webT = (french, english) => (fr ? french : english);
  document.documentElement.lang = fr ? 'fr' : 'en';
  // Les textes de la page portent leur version anglaise dans data-en : on la pose si le navigateur n'est pas en français.
  if (!fr) {
    for (const el of document.querySelectorAll('[data-en]')) el.textContent = el.dataset.en;
    for (const el of document.querySelectorAll('[data-en-placeholder]')) el.placeholder = el.dataset.enPlaceholder;
  }
})();

// Assistant du premier lancement : crée le compte administrateur et règle le serveur.
(() => {
  'use strict';
  const $ = (id) => document.getElementById(id);
  const error = $('error');

  fetch('/api/setup/status').then((r) => r.json()).then((s) => {
    if (s.initialized) location.replace('/');
    if (s.tokenRequired) $('tokenField').hidden = false;
    if (s.folders?.length) $('folder').value = s.folders[0];
    if (s.serverName) $('serverName').value = s.serverName;
  }).catch(() => {});

  $('form').addEventListener('submit', async (e) => {
    e.preventDefault();
    error.textContent = '';
    if ($('password').value !== $('password2').value) {
      error.textContent = 'Les deux mots de passe ne sont pas identiques.';
      return;
    }
    $('submit').disabled = true;
    try {
      const folder = $('folder').value.trim();
      const res = await fetch('/api/setup/init', {
        method: 'POST',
        headers: { 'content-type': 'application/json', 'x-control': '1' },
        credentials: 'same-origin',
        body: JSON.stringify({
          serverName: $('serverName').value.trim(),
          username: $('username').value.trim(),
          password: $('password').value,
          libraryFolders: folder ? [folder] : [],
          tmdbKey: $('tmdb').value.trim(),
          token: $('token').value.trim(),
        }),
      });
      const body = await res.json().catch(() => ({}));
      if (!res.ok || !body.ok) throw new Error(body?.error?.message || 'Installation impossible.');
      location.replace('/');
    } catch (err) {
      error.textContent = err.message;
      $('submit').disabled = false;
    }
  });
})();

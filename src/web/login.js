// Page de connexion (et d'inscription quand l'administrateur l'a ouverte).
(() => {
  'use strict';
  const $ = (id) => document.getElementById(id);
  const form = $('form');
  const error = $('error');
  let mode = 'login';

  const next = () => {
    const to = new URLSearchParams(location.search).get('next') || '/';
    return /^\/(?!\/)/.test(to) ? to : '/';
  };

  fetch('/api/setup/status').then((r) => r.json()).then((s) => {
    if (s.registrationEnabled) $('register').hidden = false;
  }).catch(() => {});

  $('toggle').addEventListener('click', (e) => {
    e.preventDefault();
    mode = mode === 'login' ? 'register' : 'login';
    $('submit').textContent = mode === 'login' ? 'Se connecter' : 'Créer le compte';
    $('sub').textContent = mode === 'login' ? 'Connexion' : 'Nouveau compte';
    $('toggle').textContent = mode === 'login' ? 'Créer un compte' : 'J’ai déjà un compte';
    $('password').autocomplete = mode === 'login' ? 'current-password' : 'new-password';
    error.textContent = '';
  });

  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    error.textContent = '';
    $('submit').disabled = true;
    try {
      const res = await fetch(mode === 'login' ? '/api/auth/login' : '/api/auth/register', {
        method: 'POST',
        headers: { 'content-type': 'application/json', 'x-control': '1' },
        credentials: 'same-origin',
        body: JSON.stringify({ username: $('username').value.trim(), password: $('password').value }),
      });
      const body = await res.json().catch(() => ({}));
      if (!res.ok || !body.ok) throw new Error(body?.error?.message || 'Connexion impossible.');
      location.replace(next());
    } catch (err) {
      error.textContent = err.message;
      $('submit').disabled = false;
    }
  });
})();

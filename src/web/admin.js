// Administration du serveur : réglages généraux, dossiers de musique, comptes, outils.
(() => {
  'use strict';

  const root = document.getElementById('root');
  const clientId = Math.random().toString(36).slice(2, 10);

  async function rpc(channel, ...args) {
    const res = await fetch(`/api/rpc/${encodeURIComponent(channel)}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-control': '1', 'x-client': clientId },
      credentials: 'same-origin',
      body: JSON.stringify({ args }),
    });
    if (res.status === 401) {
      location.replace('/login?next=/admin');
      throw new Error('Connexion requise.');
    }
    const body = await res.json().catch(() => null);
    if (!res.ok || !body?.ok) throw new Error(body?.error?.message || `Erreur ${res.status}`);
    return body.result;
  }

  function h(tag, props = {}, ...children) {
    const el = document.createElement(tag);
    for (const [k, v] of Object.entries(props)) {
      if (v === null || v === undefined || v === false) continue;
      if (k === 'class') el.className = v;
      else if (k === 'text') el.textContent = v;
      else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
      else if (k === 'checked' || k === 'disabled' || k === 'value') el[k] = v;
      else el.setAttribute(k, v === true ? '' : v);
    }
    for (const c of children.flat()) if (c !== null && c !== undefined && c !== false) el.append(c.nodeType ? c : document.createTextNode(String(c)));
    return el;
  }

  const notice = h('p', { class: 'error', role: 'status' });
  const flash = (message, bad = true) => {
    notice.textContent = message;
    notice.style.color = bad ? '#ff6a5f' : '#30d158';
    clearTimeout(flash.timer);
    flash.timer = setTimeout(() => { notice.textContent = ''; }, 5000);
  };
  const guard = (fn) => async (...a) => {
    try {
      await fn(...a);
    } catch (err) {
      flash(err.message);
    }
  };
  const fmtDate = (ms) => (ms ? new Date(ms).toLocaleString() : '—');

  // ─── Réglages ──────────────────────────────────────────────────────────────
  function settingsGroup(s, reload) {
    const toggle = (label, hint, key) => h('label', { class: 'toggle-row' },
      h('span', {}, label, hint ? h('small', { text: ` — ${hint}` }) : null),
      h('input', { type: 'checkbox', checked: !!s[key], onchange: guard(async (e) => { await rpc('srv:settings', { [key]: e.target.checked }); flash('Enregistré.', false); }) }));
    const name = h('input', { class: 'text-input', value: s.serverName, maxlength: '60' });
    const tmdb = h('input', { class: 'text-input', type: 'password', placeholder: s.tmdbKey ? '•••••••• (définie)' : 'Clé d’API v3', autocomplete: 'off' });
    const usersRoot = h('input', { class: 'text-input', placeholder: '/music/comptes (vide : aucun)', value: s.usersRoot || '', spellcheck: 'false' });
    return h('section', { class: 'group' },
      h('h2', { text: 'Serveur' }),
      h('p', { text: 'Ces réglages valent pour tout le monde.' }),
      h('div', { class: 'inline-form' }, name,
        h('button', { class: 'btn btn-secondary', text: 'Renommer', onclick: guard(async () => { await rpc('srv:settings', { serverName: name.value }); flash('Enregistré.', false); }) })),
      toggle('Inscriptions ouvertes', 'chacun peut créer un compte depuis la page de connexion', 'registrationEnabled'),
      toggle('Téléchargements pour tous', 'sinon réservés aux administrateurs', 'allowUserDownloads'),
      h('div', { class: 'inline-form' }, tmdb,
        h('button', { class: 'btn btn-secondary', text: 'Enregistrer la clé TMDB', onclick: guard(async () => { if (!tmdb.value.trim()) return; await rpc('srv:settings', { tmdbKey: tmdb.value.trim() }); tmdb.value = ''; flash('Clé enregistrée.', false); reload(); }) })),
      h('p', { class: 'hint', text: 'La clé TMDB (gratuite) sert aux films et séries ; chaque compte peut aussi renseigner la sienne dans ses réglages.' }),
      h('p', { text: 'Dossier des comptes : s’il est défini, chaque nouveau compte reçoit un dossier de musique à son nom à l’intérieur (par exemple /music/alice).' }),
      h('div', { class: 'inline-form' }, usersRoot,
        h('button', { class: 'btn btn-secondary', text: 'Enregistrer', onclick: guard(async () => { await rpc('srv:settings', { usersRoot: usersRoot.value.trim() }); flash('Enregistré.', false); reload(); }) })));
  }

  // ─── Dossiers ──────────────────────────────────────────────────────────────
  function foldersGroup(s, reload) {
    const picker = h('div', { class: 'picker', hidden: true });
    const path = h('input', { class: 'text-input', placeholder: '/music', spellcheck: 'false' });

    async function browse(dir) {
      const res = await rpc('srv:browse', dir);
      picker.hidden = false;
      picker.replaceChildren(...[
        h('div', { class: 'path', text: res.path }),
        res.parent ? h('button', { type: 'button', text: '↰  ..', onclick: guard(() => browse(res.parent)) }) : null,
        ...res.dirs.map((d) => h('button', { type: 'button', text: `📁  ${d}`, onclick: guard(() => browse(`${res.path.replace(/[\\/]$/, '')}/${d}`.replace(/\/\//g, '/'))) })),
        h('button', { type: 'button', class: 'btn btn-primary', text: 'Ajouter ce dossier', onclick: guard(async () => { await rpc('srv:add-folder', res.path); picker.hidden = true; flash('Dossier ajouté : analyse en cours.', false); reload(); }) }),
      ].filter(Boolean));
    }

    return h('section', { class: 'group' },
      h('h2', { text: 'Dossiers communs' }),
      h('p', { text: 'Ces dossiers sont dans la bibliothèque de tous les comptes. Pour que chacun ait la sienne, laissez cette liste vide et attribuez des dossiers à chaque compte plus bas. Les fichiers ne sont jamais modifiés par un retrait de dossier.' }),
      ...s.libraryFolders.map((f) => h('div', { class: 'row' }, h('span', { class: 'grow', text: f }),
        h('button', { class: 'btn btn-secondary btn-small', text: 'Retirer', onclick: guard(async () => { await rpc('srv:remove-folder', f); reload(); }) }))),
      !s.libraryFolders.length ? h('p', { text: 'Aucun dossier pour l’instant.' }) : null,
      h('div', { class: 'inline-form' }, path,
        h('button', { class: 'btn btn-primary', text: 'Ajouter', onclick: guard(async () => { await rpc('srv:add-folder', path.value.trim()); reload(); }) }),
        h('button', { class: 'btn btn-secondary', text: 'Parcourir…', onclick: guard(() => browse(path.value.trim() || s.libraryFolders[0] || '/')) }),
        h('button', { class: 'btn btn-secondary', text: 'Réanalyser', onclick: guard(async () => { await rpc('srv:rescan'); flash('Analyse lancée.', false); }) })),
      picker);
  }

  // ─── Détails d'un compte : dossiers de musique et partage ─────────────────
  function userDetails(user, users, settings) {
    const box = h('div', { class: 'details' }, h('p', { class: 'hint', text: 'Chargement…' }));
    const picker = h('div', { class: 'picker', hidden: true });
    const folderInput = h('input', { class: 'text-input', placeholder: '/music/' + user.username, spellcheck: 'false' });

    async function paint() {
      const detail = await rpc('srv:users:detail', user.id);
      const grants = new Map(detail.grants.map((g) => [g.viewerId, { browse: g.browse, compare: g.compare }]));
      const save = guard(async () => {
        const list = [...grants].map(([viewerId, g]) => ({ viewerId, browse: g.browse, compare: g.compare })).filter((g) => g.browse || g.compare);
        await rpc('srv:users:set-grants', user.id, list);
        flash('Partage enregistré.', false);
      });
      const check = (viewerId, key) => {
        const input = h('input', { type: 'checkbox', checked: !!grants.get(viewerId)?.[key] });
        input.addEventListener('change', () => {
          const g = { ...(grants.get(viewerId) || { browse: false, compare: false }), [key]: input.checked };
          grants.set(viewerId, g);
          save();
        });
        return input;
      };
      const browse = async (dir) => {
        const res = await rpc('srv:browse', dir);
        picker.hidden = false;
        picker.replaceChildren(...[
          h('div', { class: 'path', text: res.path }),
          res.parent ? h('button', { type: 'button', text: '↰  ..', onclick: guard(() => browse(res.parent)) }) : null,
          ...res.dirs.map((d) => h('button', { type: 'button', text: `📁  ${d}`, onclick: guard(() => browse(`${res.path.replace(/[\\/]$/, '')}/${d}`.replace(/\/\//g, '/'))) })),
          h('button', { type: 'button', class: 'btn btn-primary', text: 'Attribuer ce dossier', onclick: guard(async () => { await rpc('srv:users:add-folder', user.id, res.path); picker.hidden = true; flash('Dossier attribué : analyse en cours.', false); paint(); }) }),
        ].filter(Boolean));
      };
      const others = users.filter((u) => u.id !== user.id && !u.disabled);
      box.replaceChildren(...[
        h('h3', { text: `Dossiers de ${user.username}` }),
        ...detail.folders.map((f) => h('div', { class: 'row' }, h('span', { class: 'grow', text: f }),
          h('button', { class: 'btn btn-secondary btn-small', text: 'Retirer', onclick: guard(async () => { await rpc('srv:users:remove-folder', user.id, f); paint(); }) }))),
        !detail.folders.length ? h('p', { class: 'hint', text: 'Aucun dossier : sa bibliothèque ne contient que les dossiers communs.' }) : null,
        h('div', { class: 'inline-form' }, folderInput,
          h('button', { class: 'btn btn-primary', text: 'Attribuer', onclick: guard(async () => { await rpc('srv:users:add-folder', user.id, folderInput.value.trim()); folderInput.value = ''; paint(); }) }),
          h('button', { class: 'btn btn-secondary', text: 'Parcourir…', onclick: guard(() => browse(folderInput.value.trim() || detail.folders[0] || settings.usersRoot || '/')) }),
          settings.usersRoot ? h('button', { class: 'btn btn-secondary', text: 'Créer son dossier', onclick: guard(async () => { await rpc('srv:users:personal-folder', user.id); flash('Dossier créé et attribué.', false); paint(); }) }) : null),
        picker,
        h('p', { class: 'hint', text: `Téléchargements : ${detail.downloadDir}` }),
        h('h3', { text: `Qui peut voir la bibliothèque et les récaps de ${user.username}` }),
        h('p', { class: 'hint', text: 'Les administrateurs parcourent toutes les bibliothèques, jamais les récaps. Chaque compte règle aussi son propre partage depuis son profil.' }),
        h('div', { class: 'share-head' }, h('span', { class: 'grow' }), h('span', { text: 'Bibliothèque' }), h('span', { text: 'Récaps' })),
        ...[{ id: '*', username: 'Tout le monde' }, ...others].map((u) => h('div', { class: 'share-row' }, h('span', { class: 'grow', text: u.username }), check(u.id, 'browse'), check(u.id, 'compare'))),
      ].filter(Boolean));
    }
    paint().catch((err) => flash(err.message));
    return box;
  }

  // ─── Comptes ───────────────────────────────────────────────────────────────
  function usersGroup(users, me, reload, settings) {
    const username = h('input', { class: 'text-input', placeholder: 'Nom', autocapitalize: 'none', spellcheck: 'false' });
    const password = h('input', { class: 'text-input', type: 'password', placeholder: 'Mot de passe (8 minimum)', autocomplete: 'new-password' });
    const role = h('select', { class: 'text-input' }, h('option', { value: 'user', text: 'Utilisateur' }), h('option', { value: 'admin', text: 'Administrateur' }));
    return h('section', { class: 'group' },
      h('h2', { text: 'Comptes' }),
      h('p', { text: 'Chaque compte a sa propre bibliothèque d’écoute : réglages, statistiques, playlists, films et séries vus.' }),
      ...users.map((u) => {
        const holder = h('div', { class: 'details-holder' });
        const toggle = h('button', { class: 'btn btn-secondary btn-small', text: 'Détails', onclick: () => {
          if (holder.firstChild) { holder.replaceChildren(); toggle.textContent = 'Détails'; return; }
          holder.append(userDetails(u, users, settings));
          toggle.textContent = 'Fermer';
        } });
        return h('div', { class: 'user' },
          h('div', { class: 'row' },
            h('span', { class: 'grow' }, h('b', { text: u.username }), u.displayName ? ` (${u.displayName})` : '', ' ', h('small', { text: `dernière connexion : ${fmtDate(u.lastLogin)}` })),
            h('span', { class: `tag ${u.role === 'admin' ? 'admin' : ''}`, text: u.role === 'admin' ? 'Admin' : 'Utilisateur' }),
            u.disabled ? h('span', { class: 'tag off', text: 'Désactivé' }) : null,
            toggle,
            u.id === me ? null : [
              h('button', { class: 'btn btn-secondary btn-small', text: u.disabled ? 'Réactiver' : 'Désactiver', onclick: guard(async () => { await rpc('srv:users:update', u.id, { disabled: !u.disabled }); reload(); }) }),
              h('button', { class: 'btn btn-secondary btn-small', text: u.role === 'admin' ? 'Rétrograder' : 'Promouvoir', onclick: guard(async () => { await rpc('srv:users:update', u.id, { role: u.role === 'admin' ? 'user' : 'admin' }); reload(); }) }),
              h('button', { class: 'btn btn-secondary btn-small', text: 'Mot de passe', onclick: guard(async () => {
                const next = prompt(`Nouveau mot de passe pour ${u.username} (8 caractères minimum) :`);
                if (next) { await rpc('srv:users:password', u.id, next); flash('Mot de passe modifié, session fermée.', false); }
              }) }),
              h('button', { class: 'btn btn-secondary btn-small', text: 'Supprimer', onclick: guard(async () => {
                if (confirm(`Supprimer le compte ${u.username} et toutes ses données d’écoute ?`)) { await rpc('srv:users:delete', u.id); reload(); }
              }) }),
            ]),
          holder);
      }),
      h('div', { class: 'inline-form' }, username, password, role,
        h('button', { class: 'btn btn-primary', text: 'Créer le compte', onclick: guard(async () => { await rpc('srv:users:create', { username: username.value.trim(), password: password.value, role: role.value }); reload(); }) })));
  }

  // ─── Outils ────────────────────────────────────────────────────────────────
  function toolsGroup(status, reload) {
    const line = (name, t) => h('div', { class: 'row' }, h('span', { class: 'grow', text: name }),
      h('small', { text: t.installed ? `installé${t.version ? ` · ${t.version}` : ''}` : t.supported ? 'non installé' : 'indisponible sur ce système' }));
    return h('section', { class: 'group' },
      h('h2', { text: 'Outils de téléchargement' }),
      h('p', { text: 'yt-dlp et ffmpeg, partagés par tous les comptes, servent aux téléchargements et aux vignettes de vidéos.' }),
      line('yt-dlp', status.ytdlp), line('ffmpeg', status.ffmpeg), line('deno', status.deno),
      h('div', { class: 'inline-form' },
        h('button', { class: 'btn btn-primary', text: 'Installer', onclick: guard(async () => { flash('Installation en cours…', false); const r = await rpc('tools:install'); flash(r.ok ? 'Outils installés.' : r.error, r.ok === false); reload(); }) }),
        h('button', { class: 'btn btn-secondary', text: 'Mettre à jour', onclick: guard(async () => { await rpc('tools:update'); flash('Mise à jour terminée.', false); reload(); }) })));
  }

  async function render() {
    const [s, users, me, tools] = await Promise.all([
      rpc('srv:settings:get'),
      rpc('srv:users:list'),
      fetch('/api/auth/me', { credentials: 'same-origin' }).then((r) => r.json()),
      rpc('tools:status').catch(() => null),
    ]);
    const reload = () => render().catch((err) => flash(err.message));
    root.replaceChildren(
      h('div', { class: 'admin-head' },
        h('h1', { text: 'Administration' }),
        h('a', { class: 'btn btn-secondary', href: '/', text: 'Retour à l’application' })),
      notice,
      settingsGroup(s, reload),
      foldersGroup(s, reload),
      usersGroup(users, me.user.id, reload, s),
      tools ? toolsGroup(tools, reload) : null);
  }

  render().catch((err) => flash(err.message));
})();

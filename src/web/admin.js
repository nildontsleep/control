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
    const uploadMax = h('input', { class: 'text-input', type: 'number', min: '1', max: '20480', value: String(s.uploadMaxMb || 500), style: 'width:110px;flex:none', onchange: guard(async () => { await rpc('srv:settings', { uploadMaxMb: Number(uploadMax.value) }); flash('Enregistré.', false); }) });
    const usersRoot = h('input', { class: 'text-input', placeholder: '/music/comptes (vide : aucun)', value: s.usersRoot || '', spellcheck: 'false' });
    return h('section', { class: 'group' },
      h('h2', { text: 'Serveur' }),
      h('p', { text: 'Ces réglages valent pour tout le monde.' }),
      h('div', { class: 'inline-form' }, name,
        h('button', { class: 'btn btn-secondary', text: 'Renommer', onclick: guard(async () => { await rpc('srv:settings', { serverName: name.value }); flash('Enregistré.', false); }) })),
      toggle('Inscriptions ouvertes', 'chacun peut créer un compte depuis la page de connexion', 'registrationEnabled'),
      toggle('Téléchargements pour tous', 'sinon réservés aux administrateurs', 'allowUserDownloads'),
      toggle('Envois de musique pour tous', 'depuis le navigateur (glisser-déposer), dans le dossier Envois de chaque compte ; sinon réservés aux administrateurs', 'allowUserUploads'),
      h('div', { class: 'toggle-row' }, h('span', { text: 'Taille maximale d’un fichier envoyé (Mo)' }), uploadMax),
      toggle('API Subsonic', 'permet aux applications Subsonic / OpenSubsonic (Symfonium, DSub, Feishin…) de se connecter avec un mot de passe d’application propre à chaque compte', 'subsonicEnabled'),
      h('div', { class: 'inline-form' }, tmdb,
        h('button', { class: 'btn btn-secondary', text: 'Enregistrer la clé TMDB', onclick: guard(async () => { if (!tmdb.value.trim()) return; await rpc('srv:settings', { tmdbKey: tmdb.value.trim() }); tmdb.value = ''; flash('Clé enregistrée.', false); reload(); }) })),
      h('p', { class: 'hint', text: 'La clé TMDB (gratuite) sert aux films et séries ; chaque compte peut aussi renseigner la sienne dans ses réglages.' }),
      h('p', { text: 'Dossier des comptes : s’il est défini, chaque nouveau compte reçoit un dossier de musique à son nom à l’intérieur (par exemple /music/alice).' }),
      h('div', { class: 'inline-form' }, usersRoot,
        h('button', { class: 'btn btn-secondary', text: 'Enregistrer', onclick: guard(async () => { await rpc('srv:settings', { usersRoot: usersRoot.value.trim() }); flash('Enregistré.', false); reload(); }) })));
  }

  // ─── Mises à jour du serveur ───────────────────────────────────────────────
  const fmtBytes = (n) => (n >= 1048576 ? `${(n / 1048576).toFixed(1)} Mo` : `${Math.max(1, Math.round(n / 1024))} Ko`);
  const UPDATE_ERRORS = {
    network: 'Connexion à GitHub impossible.',
    http: 'GitHub ne répond pas correctement.',
    busy: 'GitHub limite les requêtes : réessayez dans quelques minutes.',
    private: 'Le dépôt est privé : renseignez un jeton GitHub en lecture ci-dessous.',
    token: 'Jeton GitHub refusé ou expiré.',
    untrusted: 'Adresse de téléchargement non reconnue.',
    corrupt: 'Le fichier téléchargé est corrompu (empreinte différente).',
    stage: 'L’archive téléchargée est inattendue ou n’a pas pu être installée.',
    engine: 'Cette version demande une version de Node.js plus récente.',
    deps: 'Impossible d’installer les dépendances de cette version (npm).',
    unsupervised: 'Ce serveur n’est pas lancé par le lanceur : il ne peut pas redémarrer tout seul.',
    signature: 'La signature de l’archive est absente ou invalide : la mise à jour est refusée.',
    launch: 'Le serveur ne s’est pas redémarré : relancez-le à la main, la nouvelle version est déjà choisie.',
  };
  const UPDATE_REASONS = {
    unsupervised: 'Ce serveur n’est pas lancé par le lanceur (npm run web ou l’image Docker) : à mettre à jour à la main.',
    asset: 'Cette version n’a pas d’archive pour le serveur : à mettre à jour à la main.',
    nodigest: 'L’archive de cette version n’a pas d’empreinte à vérifier : à mettre à jour à la main.',
    nosignature: 'Cette version n’est pas signée alors que ce serveur exige une signature : à mettre à jour à la main.',
  };

  /** Le serveur redémarre sur sa nouvelle version : on recharge la page dès qu'il répond de nouveau. */
  async function reloadWhenBack(from) {
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    let wentDown = false;
    await sleep(1200);
    for (let i = 0; i < 120; i++) {
      try {
        const res = await fetch('/api/health', { cache: 'no-store' });
        if (res.ok && ((await res.json()).version !== from || wentDown)) {
          location.reload();
          return;
        }
      } catch {
        wentDown = true;
      }
      await sleep(1000);
    }
  }

  function updatesGroup(initial, s) {
    let status = initial;
    let timer = null;
    const line = h('div', { class: 'upd-admin' });
    const notes = h('div', { class: 'upd-notes' });
    const warn = h('p', { class: 'error', hidden: true });
    const mode = h('select', { class: 'text-input' },
      h('option', { value: 'off', text: 'Désactivées' }),
      h('option', { value: 'notify', text: 'Prévenir seulement' }),
      h('option', { value: 'auto', text: 'Automatiques' }));
    mode.value = s.updateMode;
    mode.disabled = !!s.updateLocked;
    const hour = h('select', { class: 'text-input' }, ...Array.from({ length: 24 }, (_, i) => h('option', { value: String(i), text: `${i} h` })));
    hour.value = String(s.updateHour ?? 4);
    const hourRow = h('div', { class: 'toggle-row' }, h('span', { text: 'Redémarrer au plus tard à' }), hour);
    hourRow.hidden = s.updateMode !== 'auto';
    const modeHint = h('p', { class: 'hint' });
    const token = h('input', { class: 'text-input', type: 'password', placeholder: s.hasUpdateToken ? '•••••••• (définie)' : 'github_pat_…', autocomplete: 'off', spellcheck: 'false' });

    const saveSettings = guard(async (patch) => {
      await rpc('srv:settings', patch);
      if ('updateMode' in patch) { hourRow.hidden = patch.updateMode !== 'auto'; paintHint(); }
      await refresh();
    });
    const paintHint = () => {
      modeHint.textContent = s.updateLocked
        ? 'Mode imposé par la variable d’environnement CONTROL_UPDATE du serveur.'
        : mode.value === 'auto'
          ? 'Le serveur télécharge les mises à jour et les installe dès que plus personne n’est connecté, ou à l’heure indiquée au plus tard (une courte coupure : les pages se reconnectent seules).'
          : mode.value === 'notify'
            ? 'Le serveur vérifie toutes les 6 heures et prévient les administrateurs ; l’installation se déclenche d’un clic.'
            : 'Aucune vérification automatique : le bouton « Rechercher » reste disponible.';
    };
    mode.addEventListener('change', () => saveSettings({ updateMode: mode.value }));
    hour.addEventListener('change', () => saveSettings({ updateHour: Number(hour.value) }));

    const act = (fn) => guard(async () => { await fn(); await refresh(); });
    const run = guard(async () => {
      // Un seul clic : télécharger (en vérifiant), puis installer et redémarrer.
      poll();
      let st = status.phase === 'ready' ? status : await rpc('srv:update:download');
      if (st.phase === 'ready') {
        st = await rpc('srv:update:install');
        if (st.phase === 'installing') reloadWhenBack(status.current);
      }
      await refresh();
    });

    function paint() {
      const st = status;
      const buttons = [];
      let title;
      let sub = '';
      let bar = null;
      if (st.phase === 'checking') title = 'Recherche en cours…';
      else if (st.phase === 'available') {
        title = `La version ${st.version} est disponible`;
        sub = st.manual ? (UPDATE_REASONS[st.reason] || '') : st.size ? fmtBytes(st.size) : '';
        if (st.manual) buttons.push(h('a', { class: 'btn btn-primary', href: st.page, target: '_blank', rel: 'noopener', text: 'Voir la version' }));
        else buttons.push(h('button', { class: 'btn btn-primary', text: 'Mettre à jour et redémarrer', onclick: run }));
      } else if (st.phase === 'downloading') {
        title = `Téléchargement de la version ${st.version}…`;
        const pct = st.size ? Math.min(100, Math.round(((st.received || 0) / st.size) * 100)) : 0;
        sub = st.size ? `${fmtBytes(st.received || 0)} / ${fmtBytes(st.size)}` : '';
        bar = h('div', { class: 'upd-bar' }, h('i', { style: `width:${pct}%` }));
      } else if (st.phase === 'ready') {
        title = `La version ${st.version} est prête`;
        sub = st.verified ? 'Archive vérifiée (SHA-256).' : '';
        buttons.push(h('button', { class: 'btn btn-primary', text: 'Redémarrer maintenant', onclick: run }));
      } else if (st.phase === 'installing') title = 'Redémarrage du serveur…';
      else if (st.phase === 'error') {
        title = 'La mise à jour a échoué';
        sub = `${UPDATE_ERRORS[st.code] || 'Erreur inconnue.'}${st.detail ? ` (${st.detail})` : ''}`;
        buttons.push(h('button', { class: 'btn btn-secondary', text: 'Réessayer', onclick: act(() => rpc('srv:update:check')) }));
      } else {
        title = st.phase === 'none' ? 'control est à jour' : 'Aucune vérification pour l’instant';
        sub = st.skipped ? `La version ${st.skipped} n’a pas démarré sur ce serveur : elle n’est plus proposée automatiquement.` : '';
      }
      if (['idle', 'none', 'available', 'error'].includes(st.phase)) buttons.push(h('button', { class: 'btn btn-secondary', text: 'Rechercher', onclick: act(() => rpc('srv:update:check')) }));
      line.replaceChildren(
        h('div', { class: 'grow' }, h('strong', { text: title }), h('span', { class: 'hint', text: [`Version installée : ${st.current}`, st.checkedAt ? `vérifié à ${new Date(st.checkedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}` : '', sub].filter(Boolean).join(' · ') }), bar),
        h('div', { class: 'upd-buttons' }, ...buttons));
      notes.replaceChildren(...(st.phase === 'available' && st.notes ? [h('details', {}, h('summary', { text: 'Nouveautés' }), h('pre', { text: st.notes }))] : []));
      warn.hidden = !st.failure;
      if (st.failure) warn.textContent = `La version ${st.failure.version} n’a pas démarré sur ce serveur : il est revenu à la précédente.`;
      if (['checking', 'downloading', 'installing'].includes(st.phase)) poll();
    }

    async function refresh() {
      status = await rpc('srv:update:status');
      paint();
    }
    function poll() {
      clearTimeout(timer);
      timer = setTimeout(() => refresh().catch(() => poll()), 1000);
    }
    paintHint();
    paint();

    return h('section', { class: 'group', id: 'updates' },
      h('h2', { text: 'Mises à jour du serveur' }),
      h('p', { text: 'control se met à jour depuis ses releases GitHub : l’archive du serveur est téléchargée, vérifiée (SHA-256), installée à côté de la version en cours, puis le serveur redémarre dessus. Si la nouvelle version ne démarre pas, il revient tout seul à la précédente.' }),
      line,
      notes,
      warn,
      h('p', { class: 'hint', text: status.signed ? 'Archives vérifiées : empreinte SHA-256 et signature Ed25519.' : 'Archives vérifiées par leur empreinte SHA-256 seulement. Pour exiger aussi une signature (protège d’un dépôt GitHub compromis), voir « Signer les mises à jour » dans le README.' }),
      h('div', { class: 'toggle-row' }, h('span', { text: 'Mises à jour' }), mode),
      modeHint,
      hourRow,
      h('details', { class: 'upd-token' },
        h('summary', { text: 'Dépôt privé : jeton GitHub' }),
        h('p', { class: 'hint', text: 'Seulement si le dépôt des releases est privé : un jeton en lecture seule (Contents : Read). Il reste sur le serveur et ne revient jamais dans cette page.' }),
        h('div', { class: 'inline-form' }, token,
          h('button', { class: 'btn btn-secondary', text: 'Enregistrer', onclick: guard(async () => { if (!token.value.trim()) return; await rpc('srv:settings', { updateToken: token.value.trim() }); token.value = ''; flash('Jeton enregistré.', false); await rpc('srv:update:check'); await refresh(); }) }),
          s.hasUpdateToken ? h('button', { class: 'btn btn-secondary', text: 'Effacer', onclick: guard(async () => { await rpc('srv:settings', { updateToken: '' }); flash('Jeton effacé.', false); }) }) : null)));
  }

  // ─── Sauvegardes vers un webhook Discord ───────────────────────────────────
  function backupsGroup(initial, s, reload) {
    let status = initial;
    const line = h('div', { class: 'upd-admin' });
    const webhook = h('input', { class: 'text-input', type: 'password', placeholder: s.hasBackupWebhook ? '•••••••• (défini)' : 'https://discord.com/api/webhooks/…', autocomplete: 'off', spellcheck: 'false' });
    const passphrase = h('input', { class: 'text-input', type: 'password', placeholder: s.hasBackupPassphrase ? '•••••••• (définie)' : 'Phrase secrète (recommandé)', autocomplete: 'new-password' });
    const every = h('select', { class: 'text-input' },
      h('option', { value: 'off', text: 'Désactivée' }),
      h('option', { value: 'daily', text: 'Chaque jour' }),
      h('option', { value: 'weekly', text: 'Chaque semaine' }));
    every.value = s.backupEvery || 'off';
    const hour = h('select', { class: 'text-input' }, ...Array.from({ length: 24 }, (_, i) => h('option', { value: String(i), text: `${i} h` })));
    hour.value = String(s.backupHour ?? 3);
    const warn = h('p', { class: 'error', hidden: true });
    const save = (patch, message) => guard(async () => { await rpc('srv:settings', patch); if (message) flash(message, false); await refresh(); });

    function paint() {
      const last = status.last || {};
      let title = 'Aucune sauvegarde pour l’instant';
      let sub = status.configured ? 'Le webhook est configuré.' : 'Renseignez un webhook Discord (Paramètres du salon → Intégrations → Webhooks).';
      if (last.at && last.ok) {
        title = `Dernière sauvegarde : ${new Date(last.at).toLocaleString()}`;
        sub = `${(last.size / 1048576).toFixed(1)} Mo · ${last.accounts} compte(s)${last.parts > 1 ? ` · ${last.parts} parties` : ''}${last.encrypted ? ' · chiffrée' : ' · non chiffrée'}${last.skipped ? ` · ${last.skipped} base(s) ignorée(s) (trop volumineuses)` : ''}`;
      } else if (last.at) {
        title = `Dernière tentative en échec : ${new Date(last.at).toLocaleString()}`;
        sub = last.error || '';
      }
      const buttons = [
        h('button', { class: 'btn btn-primary', text: status.running ? 'Sauvegarde en cours…' : 'Sauvegarder maintenant', disabled: !status.configured || status.running, onclick: guard(async () => { flash('Sauvegarde en cours…', false); await rpc('srv:backup:run'); flash('Sauvegarde envoyée.', false); await refresh(); }) }),
        h('button', { class: 'btn btn-secondary', text: 'Tester le webhook', disabled: !status.configured, onclick: guard(async () => { await rpc('srv:backup:test'); flash('Message de test envoyé.', false); }) }),
      ];
      line.replaceChildren(
        h('div', { class: 'grow' }, h('strong', { text: title }), h('span', { class: 'hint', text: sub })),
        h('div', { class: 'upd-buttons' }, ...buttons));
      warn.hidden = !(status.configured && !status.encrypted);
      warn.textContent = 'Sans phrase secrète, l’archive envoyée à Discord contient les empreintes de mots de passe de tous les comptes : n’utilisez qu’un salon privé, ou choisissez une phrase secrète.';
    }
    async function refresh() {
      status = await rpc('srv:backup:status');
      paint();
    }
    every.addEventListener('change', () => save({ backupEvery: every.value }, 'Planification enregistrée.')());
    hour.addEventListener('change', () => save({ backupHour: Number(hour.value) }, 'Heure enregistrée.')());
    paint();

    return h('section', { class: 'group', id: 'backups' },
      h('h2', { text: 'Sauvegardes' }),
      h('p', { text: 'La base des comptes et, pour chaque compte, ses écoutes, playlists, tags et réglages sont archivés puis envoyés à un webhook Discord (découpés en parties de 9 Mo au plus). Jamais copiés : sessions, jetons et clés d’API, cookies YouTube. Les deux dernières sauvegardes restent aussi sur le disque du serveur.' }),
      line,
      warn,
      h('div', { class: 'inline-form' }, webhook,
        h('button', { class: 'btn btn-secondary', text: 'Enregistrer le webhook', onclick: guard(async () => { if (!webhook.value.trim()) return; await rpc('srv:settings', { backupWebhook: webhook.value.trim() }); webhook.value = ''; flash('Webhook enregistré.', false); reload(); }) }),
        s.hasBackupWebhook ? h('button', { class: 'btn btn-secondary', text: 'Effacer', onclick: guard(async () => { await rpc('srv:settings', { backupWebhook: '', backupEvery: 'off' }); flash('Webhook effacé, sauvegardes désactivées.', false); reload(); }) }) : null),
      h('div', { class: 'toggle-row' }, h('span', { text: 'Sauvegarde planifiée' }), every),
      h('div', { class: 'toggle-row' }, h('span', { text: 'À partir de' }), hour),
      h('details', { class: 'upd-token', open: !s.hasBackupPassphrase },
        h('summary', { text: 'Phrase secrète (chiffrement AES-256 de l’archive)' }),
        h('p', { class: 'hint', text: 'Gardez-la ailleurs que sur le serveur : sans elle, une sauvegarde chiffrée est illisible. Pour déchiffrer : CONTROL_BACKUP_PASSPHRASE=… node server/backup.js decrypt <fichier>.' }),
        h('div', { class: 'inline-form' }, passphrase,
          h('button', { class: 'btn btn-secondary', text: 'Enregistrer', onclick: guard(async () => { if (!passphrase.value) return; await rpc('srv:settings', { backupPassphrase: passphrase.value }); passphrase.value = ''; flash('Phrase secrète enregistrée.', false); reload(); }) }),
          s.hasBackupPassphrase ? h('button', { class: 'btn btn-secondary', text: 'Effacer', onclick: guard(async () => { await rpc('srv:settings', { backupPassphrase: '' }); flash('Chiffrement désactivé.', false); reload(); }) }) : null)));
  }

  // ─── Application Last.fm (scrobbling des comptes) ──────────────────────────
  function lastfmGroup(s, reload) {
    const key = h('input', { class: 'text-input', placeholder: 'Clé d’API', value: s.lastfmKey || '', autocomplete: 'off', spellcheck: 'false' });
    const secret = h('input', { class: 'text-input', type: 'password', placeholder: s.hasLastfmSecret ? '•••••••• (défini)' : 'Secret partagé', autocomplete: 'off' });
    return h('section', { class: 'group', id: 'lastfm' },
      h('h2', { text: 'Last.fm (scrobbling)' }),
      h('p', { text: 'Pour que les comptes puissent envoyer leurs écoutes à Last.fm, créez une application gratuite sur last.fm/api/account/create et renseignez ici sa clé d’API et son secret partagé. Chaque compte autorise ensuite control sur le site de Last.fm (Réglages → Général → Scrobbling) ; ListenBrainz n’a besoin de rien ici.' }),
      h('div', { class: 'inline-form' }, key, secret,
        h('button', { class: 'btn btn-secondary', text: 'Enregistrer', onclick: guard(async () => {
          const patch = { lastfmKey: key.value.trim() };
          if (secret.value) patch.lastfmSecret = secret.value;
          await rpc('srv:settings', patch);
          flash('Application Last.fm enregistrée.', false);
          reload();
        }) }),
        s.hasLastfmSecret ? h('button', { class: 'btn btn-secondary', text: 'Effacer', onclick: guard(async () => { await rpc('srv:settings', { lastfmKey: '', lastfmSecret: '' }); flash('Clés effacées.', false); reload(); }) }) : null));
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
    const [s, users, me, tools, updates, backups] = await Promise.all([
      rpc('srv:settings:get'),
      rpc('srv:users:list'),
      fetch('/api/auth/me', { credentials: 'same-origin' }).then((r) => r.json()),
      rpc('tools:status').catch(() => null),
      rpc('srv:update:status').catch(() => null),
      rpc('srv:backup:status').catch(() => null),
    ]);
    const reload = () => render().catch((err) => flash(err.message));
    root.replaceChildren(...[
      h('div', { class: 'admin-head' },
        h('h1', { text: 'Administration' }),
        h('a', { class: 'btn btn-secondary', href: '/', text: 'Retour à l’application' })),
      notice,
      settingsGroup(s, reload),
      updates ? updatesGroup(updates, s) : null,
      backups ? backupsGroup(backups, s, reload) : null,
      lastfmGroup(s, reload),
      foldersGroup(s, reload),
      usersGroup(users, me.user.id, reload, s),
      tools ? toolsGroup(tools, reload) : null,
    ].filter(Boolean));
    if (location.hash === '#updates') document.getElementById('updates')?.scrollIntoView();
  }

  render().catch((err) => flash(err.message));
})();

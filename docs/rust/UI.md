# Interface Rune : repères pour contribuer

- Code : `crates/control-app/src`. État : `state.rs` (`Ui`, `Route`, `Sheet`), lecteur `player.rs`, services
  `backend.rs` (`Backend` : store, library, tags, lyrics, tools, downloads, media, scrobbler, discord, loudness,
  updater). Vue racine : `ui/mod.rs`. Briques : `ui/common.rs` (pochettes, boutons, en-têtes, styles `ty`),
  `ui/grid.rs` (grilles virtualisées), `ui/tracklist.rs` (listes de morceaux), `views/mod.rs` (`segmented`,
  `empty_library`), `sheets/mod.rs` (`panel()` : feuille modale à hauteur bornée), `settings_ui.rs` (lignes de
  réglages `group`/`line`/`toggle`), `theme.rs` (`Palette`), `i18n.rs` (`t`, `tn`, `tv`; dictionnaires
  `i18n/{fr,en}.json`, mêmes clés que l'original ; ajouter une clé dans les deux fichiers).
- Travail lent (réseau, disque) : `ui.spawn(|backend| …, |ui, résultat, tx| …)`, jamais sur le thread de l'interface.
- Rune : sources et docs dans le checkout cargo (`~/.cargo/git/checkouts/rune-*/eff95e8/`), en particulier
  `crates/rune/README.md` (API des vues), `crates/rune-kit/README.md` (composants). Des exemples complets sont
  dans `examples/`.
- Rendu sans fenêtre pour vérifier son travail :
  `CONTROL_DATA=<dossier> target/debug/control --snap out.png --scene <scène> --size 1360x860 --at 4 --scale 1`
  (scènes dans `main.rs`, `SCENES`). Un `settings.json` avec `{"settings":{"libraryFolders":["<musique>"]}}` dans
  ce dossier remplit la bibliothèque (lancer deux fois : le premier passage analyse les fichiers).

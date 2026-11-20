# Portage de control en Rust : règles communes

Ce dépôt réécrit control (Electron + Node, `src/` et `server/`) en Rust, avec une interface Rune
(https://github.com/anomalyco/rune). L'ancien code reste dans `src/` et `server/` comme référence.

## Crates

- `crates/control-core` : tout ce qui ne dépend ni d'une fenêtre ni d'un navigateur (réglages, base SQLite,
  bibliothèque, tags, paroles, statistiques, téléchargements, films et séries, scrobbling, Discord, mises à jour).
- `crates/control-audio` : moteur de lecture (décodage symphonia, sortie cpal, DSP).
- `crates/control-app` : application de bureau (binaire `control`), interface Rune.
- `crates/control-server` : serveur web (binaire `control-server`).

## Conventions

- Compatibilité des données : mêmes fichiers, mêmes formats JSON, même schéma SQLite (`db.rs`) que l'original.
  Un dossier de données de la version Electron doit se rouvrir tel quel.
- API bloquantes, appelées depuis des threads de travail ; jamais d'E/S sur le thread de l'interface.
- Pas de `unwrap()`/`expect()` hors tests (sauf invariants évidents, comme une regex constante, avec `expect`).
  Les erreurs remontent en `anyhow::Result` ou en types d'erreur propres ; les messages destinés à
  l'utilisateur sont en français, comme dans l'original.
- Commentaires et docs en français, sobres, qui disent pourquoi.
- Événements vers l'interface : `crate::model::Emit` (`Event::Json(canal, valeur)` avec les noms de canaux
  d'origine, par exemple `dl:update`, `tools:progress`).
- Base partagée : `crate::library::SharedDb` (`Arc<parking_lot::Mutex<rusqlite::Connection>>`). Ne garder le
  verrou que le temps d'une requête ou d'une transaction, jamais pendant une E/S réseau ou disque.
- HTTP : `ureq` 3 (bloquant). Les tests ne touchent jamais le réseau : isoler l'analyse des réponses dans des
  fonctions pures testées avec des réponses enregistrées dans le test.
- Tests à côté du code (`#[cfg(test)] mod tests;` avec `module/tests.rs`, ou un module en bas du fichier).
- `cargo test -p <crate>` et `cargo clippy -p <crate> --all-targets` sans erreur ni avertissement.
- Fichiers de moins de ~800 lignes : découper par capacité (`module/…`).

# Composants tiers

control embarque les ressources suivantes.

## Lucide (icônes)

https://lucide.dev — utilisées dans `crates/control-app/src/icons.rs` et `src/renderer/js/icons.js` (interface web).

```
ISC License

Copyright (c) 2026 Lucide Icons and Contributors

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

---

The following Lucide icons are derived from the Feather project:

activity, airplay, alert-circle, alert-octagon, alert-triangle, aperture, arrow-down-circle, arrow-down-left, arrow-down-right, arrow-down, arrow-left-circle, arrow-left, arrow-right-circle, arrow-right, arrow-up-circle, arrow-up-left, arrow-up-right, arrow-up, at-sign, award, calendar, cast, check, chevron-down, chevron-left, chevron-right, chevron-up, chevrons-down, chevrons-left, chevrons-right, chevrons-up, circle, clipboard, clock, code, columns, command, compass, corner-down-left, corner-down-right, corner-left-down, corner-left-up, corner-right-down, corner-right-up, corner-up-left, corner-up-right, crosshair, database, divide-circle, divide-square, dollar-sign, download, external-link, fast-forward, feather, frown, hash, headphones, help-circle, info, italic, key, layers, layout, life-buoy, link-2, link, loader, lock, log-in, log-out, maximize, meh, minimize, minimize-2, minus-circle, minus-square, minus, monitor, moon, more-horizontal, more-vertical, move, music, navigation-2, navigation, octagon, pause-circle, percent, plus-circle, plus-square, plus, power, radio, rss, search, server, share-2, share, shopping-bag, sidebar, smartphone, smile, square, table-2, tablet, target, terminal, trash-2, trash, triangle, tv, type, upload, x-circle, x-octagon, x-square, x, zoom-in, zoom-out

The MIT License (MIT) (for the icons listed above)

Copyright (c) 2013-present Cole Bemis

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Inter (police)

https://rsms.me/inter — embarquée par Rune dans l'application de bureau, et fichiers dans `src/renderer/assets/fonts/` pour l'interface web, sous licence SIL Open Font License 1.1 (voir `src/renderer/assets/fonts/OFL.txt`).

## Bibliothèques Rust

control est construit avec des bibliothèques libres, compilées dans l'exécutable. Les principales :

- **Rune** (interface, rendu GPU, texte) — licence MIT, © Anomaly (https://github.com/anomalyco/rune), avec wgpu, winit, Parley, Taffy, AccessKit (MIT ou Apache-2.0).
- **symphonia** (décodage audio) et **opuscule** — licence **MPL-2.0** : leur code source est disponible sur crates.io et dans leurs dépôts (https://github.com/pdeljanov/Symphonia) ; control les utilise sans les modifier.
- **cpal** (sortie audio) — Apache-2.0 ; **rubato** (rééchantillonnage) — MIT.
- **lofty** (lecture et écriture des tags) — MIT ou Apache-2.0.
- **rusqlite** et **SQLite** (base de données) — MIT ; SQLite est dans le domaine public.
- **image** (pochettes), **ebur128** (niveau sonore), **ureq** et **rustls** (réseau), **notify** (surveillance des dossiers), **zbus** (MPRIS), **trash** (corbeille) — MIT, Apache-2.0 ou MIT/Apache-2.0 ; **ring**/**rustls-webpki** — ISC et apparentées.
- Serveur web : **tokio**, **axum** (MIT), **scrypt**, **aes-gcm** (MIT ou Apache-2.0), **ed25519-dalek** (BSD-3-Clause).

La liste complète, avec la licence de chaque bibliothèque, s'obtient avec `cargo metadata` (ou `cargo about`, `cargo deny list`). Aucune bibliothèque sous GPL n'est utilisée.

## Outils téléchargés à la demande

**yt-dlp** (Unlicense), **ffmpeg** (LGPL ou GPL selon la version téléchargée) et **deno** (MIT) ne sont pas embarqués : l'application les télécharge depuis leurs pages officielles quand tu installes les outils de téléchargement, et les lance comme des programmes séparés.

## Services en ligne

control interroge, à ta demande, des services publics : [LRCLIB](https://lrclib.net) (paroles), Apple Music et Deezer (recherche de pochettes), [TMDB](https://www.themoviedb.org) et [AniList](https://anilist.co) (catalogues). Aucune donnée personnelle n'est envoyée en dehors du titre, de l'artiste ou de la recherche demandés.

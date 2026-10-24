# Images pour Discord (Rich Presence)

Ces cinq images sont à téléverser dans l'application Discord de control. Les noms (« clés ») doivent être **exactement** ceux-ci, en minuscules :

| Fichier | Clé à saisir | Où elle apparaît |
| --- | --- | --- |
| `control.png` | `control` | Grande image par défaut (quand aucune pochette n'est trouvée, ou si les pochettes sont désactivées) |
| `play.png` | `play` | Petite pastille : musique en lecture |
| `pause.png` | `pause` | Petite pastille : en pause |
| `watching.png` | `watching` | Petite pastille : film, série ou animé |
| `video.png` | `video` | Petite pastille : vidéo de la bibliothèque |

Toutes font 1024 × 1024 px (Discord demande 512 px minimum).

## Pas à pas

1. Ouvrir <https://discord.com/developers/applications> → **New Application**. Le nom donné ici s'affiche chez les autres : « Écoute **control** » — choisir `control`.
2. **General Information** : mettre `control.png` comme *App Icon*, puis copier l'**Application ID**.
3. **Rich Presence → Art Assets** : téléverser les cinq fichiers ci-dessus et vérifier que chaque clé est bien celle du tableau (la clé est le nom du fichier sans `.png`). Les images peuvent mettre quelques minutes à apparaître.
4. Rien d'autre à activer dans le portail : pas de bot, pas d'OAuth2, pas d'URL de redirection. control parle directement au Discord installé sur l'ordinateur.
5. Dans Discord (application de bureau, pas le navigateur) : *Paramètres utilisateur → Confidentialité de l'activité* → laisser activés le partage de l'activité détectée et l'affichage de l'activité en cours comme statut (les libellés varient un peu selon la version).
6. L'Application ID est déjà inscrit dans control (`src/main/discord.js`, constante `APP_ID`) : rien à coller. Un autre ID peut être saisi dans **Réglages → Général → Discord** (puis **Tester**) pour utiliser sa propre application.

## Ce qui est affiché

- **Musique** : « Écoute control », titre, artiste, pochette **du fichier** (celle des tags, envoyée à un hébergeur temporaire — voir le README principal), barre de progression.
- **Vidéo de la bibliothèque** : « Regarde control », titre, image de la vidéo (ou sa pochette intégrée), durée écoulée.
- **Films, séries, animés** : « Regarde control », titre, saison et épisode, affiche du film (TMDB / AniList), barre de progression.
- **Pause** : la pastille passe à `pause` et la barre disparaît ; au bout de 10 minutes de pause, la présence est effacée.
- **Masquer les titres** : seuls « Écoute de la musique » / « Regarde un film »… sont envoyés.

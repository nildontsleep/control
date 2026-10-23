// Textes de l’interface (français par défaut, anglais en option).

const fr = {
  'common.ok': 'OK', 'common.cancel': 'Annuler', 'common.close': 'Fermer', 'common.create': 'Créer', 'common.delete': 'Supprimer',
  'common.rename': 'Renommer', 'common.save': 'Enregistrer', 'common.play': 'Lecture', 'common.shuffle': 'Aléatoire', 'common.more': 'Plus',
  'common.remove': 'Retirer', 'common.change': 'Modifier…',

  'win.close': 'Fermer', 'win.minimize': 'Réduire', 'win.zoom': 'Agrandir',

  'nav.library': 'Bibliothèque', 'nav.songs': 'Morceaux', 'nav.albums': 'Albums', 'nav.artists': 'Artistes', 'nav.videos': 'Vidéos',
  'nav.get': 'Télécharger', 'nav.downloads': 'Téléchargements', 'nav.playlists': 'Playlists', 'nav.settings': 'Réglages',

  'count.tracks_one': '{n} morceau', 'count.tracks_other': '{n} morceaux',
  'count.albums_one': '{n} album', 'count.albums_other': '{n} albums',
  'count.artists_one': '{n} artiste', 'count.artists_other': '{n} artistes',
  'count.videos_one': '{n} vidéo', 'count.videos_other': '{n} vidéos',
  'artist.count_one': '{n} morceau', 'artist.count_other': '{n} morceaux',

  'col.title': 'Titre', 'col.artist': 'Artiste', 'col.album': 'Album', 'col.plays': 'Lectures', 'col.time': 'Durée',
  'sort.by': 'Trier par', 'sort.title': 'Titre', 'sort.artist': 'Artiste', 'sort.year': 'Année', 'sort.addedAt': 'Date d’ajout',
  'sort.playCount': 'Les plus écoutés', 'sort.duration': 'Durée', 'sort.lastPlayed': 'Dernière écoute', 'sort.random': 'Aléatoire',

  'album.kind': 'Album', 'album.various': 'Artistes divers', 'album.unknown': 'Artiste inconnu', 'album.discs': '{n} disques',
  'artist.kind': 'Artiste', 'artist.songs': 'Morceaux',

  'empty.libraryTitle': 'Votre musique, ici.',
  'empty.libraryText': 'Ajoutez un dossier pour remplir la bibliothèque, ou glissez des fichiers et des dossiers dans la fenêtre.',
  'empty.addMusic': 'Ajouter mon dossier Musique', 'empty.addFolder': 'Choisir un dossier…', 'empty.download': 'Télécharger depuis un lien',
  'empty.videosTitle': 'Aucune vidéo', 'empty.videosText': 'Les vidéos (MP4, MKV, WebM…) de vos dossiers apparaîtront ici. Vous pouvez aussi en télécharger.',

  'menu.play': 'Lecture', 'menu.playSelection': 'Lire la sélection ({n})', 'menu.playNext': 'Lire ensuite', 'menu.addQueue': 'Ajouter à la file d’attente',
  'menu.addToPlaylist': 'Ajouter à une playlist', 'menu.newPlaylist': 'Nouvelle playlist…', 'menu.removeFromPlaylist': 'Retirer de la playlist',
  'menu.goArtist': 'Aller à l’artiste', 'menu.goAlbum': 'Aller à l’album', 'menu.openSource': 'Ouvrir la page d’origine',
  'menu.reveal': 'Afficher dans le dossier', 'menu.removeLibrary': 'Supprimer de la bibliothèque…',

  'del.title': 'Supprimer ce morceau ?', 'del.titleMany': 'Supprimer {n} morceaux ?',
  'del.message': 'Ils seront retirés de la bibliothèque et des playlists.', 'del.confirm': 'Supprimer',
  'del.alsoFiles': 'Mettre aussi les fichiers à la corbeille',

  'pl.new': 'Nouvelle playlist', 'pl.newSmart': 'Nouvelle playlist intelligente', 'pl.namePlaceholder': 'Nom de la playlist', 'pl.name': 'Nom',
  'pl.add': 'Ajouter une playlist', 'pl.import': 'Importer une playlist M3U…', 'pl.export': 'Exporter en M3U…', 'pl.rename': 'Renommer',
  'pl.editRules': 'Modifier les règles', 'pl.kind': 'Playlist', 'pl.smart': 'Intelligente', 'pl.empty': 'Playlist vide',
  'pl.emptyTitle': 'Cette playlist est vide', 'pl.emptyText': 'Glissez des morceaux ici depuis une autre vue, ou utilisez « Ajouter à une playlist » dans leur menu.',
  'pl.emptySmart': 'Aucun morceau ne correspond encore aux règles.', 'pl.deleteTitle': 'Supprimer « {name} » ?',
  'pl.deleteText': 'La playlist sera supprimée. Vos morceaux restent dans la bibliothèque.',
  'pl.sidebarHint': 'Aucune playlist. Cliquez sur + pour en créer une.',

  'smart.presets': 'Modèles', 'smart.rules': 'Règles', 'smart.matchAll': 'Toutes les règles', 'smart.matchAny': 'Au moins une règle',
  'smart.addRule': 'Ajouter une règle', 'smart.days': 'jours', 'smart.minutes': 'min', 'smart.sortBy': 'Trier par', 'smart.limit': 'Limite',
  'smart.limitHint': '0 = pas de limite', 'smart.matches_one': '{n} morceau correspond', 'smart.matches_other': '{n} morceaux correspondent',
  'preset.mostPlayed': 'Les plus écoutés', 'preset.recent': 'Ajoutés récemment', 'preset.neverPlayed': 'Jamais écoutés',
  'preset.recentlyPlayed': 'Écoutés récemment', 'preset.genre': 'Par genre',
  'field.playCount': 'Lectures', 'field.lastPlayed': 'Dernière écoute', 'field.addedAt': 'Date d’ajout', 'field.genre': 'Genre', 'field.artist': 'Artiste',
  'field.album': 'Album', 'field.title': 'Titre', 'field.year': 'Année', 'field.duration': 'Durée (min)',
  'op.is': 'est', 'op.isNot': 'n’est pas', 'op.gt': 'supérieur à', 'op.lt': 'inférieur à', 'op.contains': 'contient',
  'op.notContains': 'ne contient pas', 'op.within': 'dans les derniers', 'op.notWithin': 'pas dans les derniers',

  'search.placeholder': 'Rechercher', 'search.title': 'Recherche', 'search.resultsFor': 'Résultats pour « {q} »',
  'search.startTitle': 'Rechercher dans votre bibliothèque', 'search.startText': 'Morceaux, albums, artistes, vidéos et playlists.',
  'search.noneTitle': 'Aucun résultat', 'search.noneText': 'Rien ne correspond à « {q} ».',

  'player.shuffle': 'Aléatoire', 'player.prev': 'Précédent', 'player.next': 'Suivant', 'player.repeat': 'Répéter', 'player.play': 'Lecture',
  'player.pause': 'Pause', 'player.seek': 'Position', 'player.mute': 'Muet', 'player.volume': 'Volume', 'player.queue': 'File d’attente',
  'player.nothing': 'Aucune lecture', 'player.nothingSub': 'Choisissez un morceau',

  'np.title': 'À l’écoute', 'np.open': 'Ouvrir « À l’écoute »', 'np.close': 'Réduire', 'np.fullscreen': 'Plein écran',
  'queue.title': 'File d’attente', 'queue.clear': 'Vider les suivants', 'queue.empty': 'La file d’attente est vide.', 'queue.remove': 'Retirer de la file',
  'eq.title': 'Égaliseur', 'eq.hint': 'Double-cliquez sur un curseur pour le remettre à zéro.', 'eq.custom': 'Personnalisé',
  'eq.flat': 'Neutre', 'eq.bass': 'Basses renforcées', 'eq.treble': 'Aigus renforcés', 'eq.vocal': 'Voix', 'eq.rock': 'Rock', 'eq.electronic': 'Électro',
  'eq.classical': 'Classique', 'eq.acoustic': 'Acoustique', 'eq.jazz': 'Jazz', 'eq.loudness': 'Loudness',

  'nav.dropTitle': 'Déposez pour ajouter', 'drop.title': 'Déposez pour ajouter à la bibliothèque', 'drop.text': 'Fichiers audio, vidéos ou dossiers entiers.',

  'dl.subtitle': 'Collez un lien YouTube, SoundCloud, Bandcamp…', 'dl.placeholder': 'Collez un lien (vidéo ou playlist)', 'dl.paste': 'Coller',
  'dl.invalidUrl': 'Ce n’est pas un lien valide (il doit commencer par https://).', 'dl.needTools': 'Installez d’abord les outils ci-dessous.',
  'dl.analyzing': 'Analyse du lien…', 'dl.analyzeFailed': 'Impossible d’analyser ce lien.', 'dl.videoKind': 'Vidéo',
  'dl.playlistKind': 'Playlist · {n} titres', 'dl.audio': 'Audio', 'dl.video': 'Vidéo', 'dl.createPlaylist': 'Créer une playlist',
  'dl.download': 'Télécharger', 'dl.downloadAll': 'Tout télécharger ({n})', 'dl.added': 'Ajouté à la file de téléchargement',
  'dl.openFolder': 'Ouvrir le dossier', 'dl.emptyQueue': 'Aucun téléchargement pour le moment.', 'dl.inProgress': 'En cours', 'dl.finished': 'Terminés',
  'dl.clear': 'Effacer la liste', 'dl.itemOf': 'Titre {i} sur {n}', 'dl.eta': 'encore {t}', 'dl.starting': 'Démarrage…', 'dl.processing': 'Conversion et ajout des tags…',
  'dl.queued': 'En attente', 'dl.canceled': 'Annulé', 'dl.failed': 'Échec du téléchargement', 'dl.doneTracks_one': '{n} fichier ajouté', 'dl.doneTracks_other': '{n} fichiers ajoutés',
  'dl.retry': 'Réessayer', 'dl.openPlaylist': 'Ouvrir la playlist', 'dl.notYetInLibrary': 'Les fichiers arrivent dans la bibliothèque, réessayez dans un instant.',
  'dl.doneToast': 'Téléchargé : {title}',

  'tools.needTitle': 'Outils de téléchargement requis', 'tools.needText': 'yt-dlp et ffmpeg seront téléchargés une seule fois dans le dossier de l’application.',
  'tools.install': 'Installer', 'tools.installing': 'Installation de {tool}…', 'tools.extracting': 'Décompression…', 'tools.preparing': 'Préparation…',
  'tools.ready': 'Outils prêts · yt-dlp {v}', 'tools.update': 'Mettre à jour yt-dlp', 'tools.updating': 'Mise à jour de yt-dlp…', 'tools.updated': 'yt-dlp mis à jour ({v})',
  'tools.upToDate': 'yt-dlp est à jour', 'tools.updateFailed': 'Mise à jour impossible', 'tools.installed': 'Outils installés', 'tools.failed': 'L’installation a échoué',
  'tools.addDeno': 'Ajouter deno (YouTube)', 'tools.installedShort': 'installé', 'tools.missing': 'non installé',
  'tools.denoNote': 'Nécessaire pour certaines vidéos YouTube',

  'scan.discover': 'Recherche… {n} fichiers', 'scan.read': 'Analyse {done} / {total}',

  'set.tab.general': 'Général', 'set.tab.library': 'Bibliothèque', 'set.tab.downloads': 'Téléchargements', 'set.tab.about': 'À propos',
  'set.appearance': 'Apparence', 'set.theme': 'Thème', 'set.accent': 'Couleur d’accentuation', 'set.ambient': 'Fond coloré', 'set.ambientHint': 'La pochette teinte la fenêtre',
  'set.compact': 'Listes compactes', 'set.compactHint': 'Lignes plus serrées', 'set.animations': 'Animations', 'set.language': 'Langue',
  'set.languageHint': 'Un changement de langue redémarre l’interface.',
  'set.folders': 'Dossiers surveillés', 'set.foldersHint': 'Les nouveaux fichiers sont détectés automatiquement.', 'set.noFolders': 'Aucun dossier pour l’instant.',
  'set.addFolder': 'Ajouter un dossier…', 'set.scanNow': 'Analyser maintenant', 'set.scanning': 'Analyse en cours…', 'set.inLibrary': 'Dans la bibliothèque',
  'set.dlFolder': 'Dossier des téléchargements', 'set.dlBehavior': 'Comportement', 'set.concurrency': 'Téléchargements simultanés',
  'set.autoUpdate': 'Mettre à jour yt-dlp automatiquement', 'set.autoUpdateHint': 'Une fois par jour, au lancement', 'set.tools': 'Outils',
  'set.shortcuts': 'Raccourcis clavier', 'set.data': 'Données', 'set.reset': 'Réinitialiser les réglages', 'set.resetConfirm': 'Cliquez encore pour confirmer',

  'about.tagline': 'Musique, vidéos et téléchargements.', 'about.version': 'Version {v}', 'about.portable': 'Édition portable', 'about.installed': 'Édition installée',
  'about.dataDir': 'Ouvrir le dossier des données', 'about.credits': 'Icônes Lucide · Police Inter · Electron · yt-dlp · ffmpeg',

  'theme.auto': 'Auto', 'theme.light': 'Clair', 'theme.dark': 'Sombre',
  'accent.blue': 'Bleu', 'accent.purple': 'Violet', 'accent.pink': 'Rose', 'accent.red': 'Rouge', 'accent.orange': 'Orange', 'accent.yellow': 'Jaune',
  'accent.green': 'Vert', 'accent.graphite': 'Graphite', 'accent.custom': 'Personnalisée',
  'lang.auto': 'Automatique', 'lang.fr': 'Français', 'lang.en': 'English',

  'kb.playPause': 'Lecture / pause', 'kb.next': 'Morceau suivant', 'kb.prev': 'Morceau précédent', 'kb.seekForward': 'Avancer de 10 s', 'kb.seekBack': 'Reculer de 10 s',
  'kb.volUp': 'Volume +', 'kb.volDown': 'Volume −', 'kb.shuffle': 'Aléatoire', 'kb.repeat': 'Répéter', 'kb.mute': 'Muet', 'kb.queue': 'File d’attente',
  'kb.eq': 'Égaliseur', 'kb.nowPlaying': 'Ouvrir « À l’écoute »', 'kb.search': 'Rechercher', 'kb.back': 'Page précédente', 'kb.settings': 'Réglages',
  'kb.fullscreen': 'Plein écran', 'kb.escape': 'Fermer',

  'toast.added_one': 'Ajouté à « {name} »', 'toast.added_other': '{n} morceaux ajoutés à « {name} »', 'toast.playNext': 'Sera lu ensuite', 'toast.queued': 'Ajouté à la file d’attente',
  'toast.playlistCreated': 'Playlist « {name} » créée', 'toast.playlistCreatedWith_one': 'Playlist « {name} » créée avec 1 morceau',
  'toast.playlistCreatedWith_other': 'Playlist « {name} » créée avec {n} morceaux', 'toast.playlistDeleted': 'Playlist supprimée',
  'toast.removed_one': 'Morceau retiré de la bibliothèque', 'toast.removed_other': '{n} morceaux retirés de la bibliothèque',
  'toast.trashed_one': 'Placé dans la corbeille', 'toast.trashed_other': '{n} fichiers placés dans la corbeille',
  'toast.exported': 'Playlist exportée ({n} morceaux)', 'toast.imported': 'Playlist importée : {n} morceaux sur {total} retrouvés', 'toast.saved': 'Enregistré',
  'toast.cannotPlay': 'Lecture impossible : {title}', 'toast.scanDone_one': '1 fichier ajouté', 'toast.scanDone_other': '{n} fichiers ajoutés',
  'toast.droppedNone': 'Aucun fichier audio ou vidéo reconnu',
};

const en = {
  'common.ok': 'OK', 'common.cancel': 'Cancel', 'common.close': 'Close', 'common.create': 'Create', 'common.delete': 'Delete',
  'common.rename': 'Rename', 'common.save': 'Save', 'common.play': 'Play', 'common.shuffle': 'Shuffle', 'common.more': 'More',
  'common.remove': 'Remove', 'common.change': 'Change…',

  'win.close': 'Close', 'win.minimize': 'Minimize', 'win.zoom': 'Zoom',

  'nav.library': 'Library', 'nav.songs': 'Songs', 'nav.albums': 'Albums', 'nav.artists': 'Artists', 'nav.videos': 'Videos',
  'nav.get': 'Get', 'nav.downloads': 'Downloads', 'nav.playlists': 'Playlists', 'nav.settings': 'Settings',

  'count.tracks_one': '{n} song', 'count.tracks_other': '{n} songs',
  'count.albums_one': '{n} album', 'count.albums_other': '{n} albums',
  'count.artists_one': '{n} artist', 'count.artists_other': '{n} artists',
  'count.videos_one': '{n} video', 'count.videos_other': '{n} videos',
  'artist.count_one': '{n} song', 'artist.count_other': '{n} songs',

  'col.title': 'Title', 'col.artist': 'Artist', 'col.album': 'Album', 'col.plays': 'Plays', 'col.time': 'Time',
  'sort.by': 'Sort by', 'sort.title': 'Title', 'sort.artist': 'Artist', 'sort.year': 'Year', 'sort.addedAt': 'Date added',
  'sort.playCount': 'Most played', 'sort.duration': 'Duration', 'sort.lastPlayed': 'Last played', 'sort.random': 'Random',

  'album.kind': 'Album', 'album.various': 'Various Artists', 'album.unknown': 'Unknown artist', 'album.discs': '{n} discs',
  'artist.kind': 'Artist', 'artist.songs': 'Songs',

  'empty.libraryTitle': 'Your music, right here.',
  'empty.libraryText': 'Add a folder to fill your library, or drag files and folders into the window.',
  'empty.addMusic': 'Add my Music folder', 'empty.addFolder': 'Choose a folder…', 'empty.download': 'Download from a link',
  'empty.videosTitle': 'No videos', 'empty.videosText': 'Videos (MP4, MKV, WebM…) from your folders will show up here. You can also download some.',

  'menu.play': 'Play', 'menu.playSelection': 'Play selection ({n})', 'menu.playNext': 'Play next', 'menu.addQueue': 'Add to queue',
  'menu.addToPlaylist': 'Add to a playlist', 'menu.newPlaylist': 'New playlist…', 'menu.removeFromPlaylist': 'Remove from playlist',
  'menu.goArtist': 'Go to artist', 'menu.goAlbum': 'Go to album', 'menu.openSource': 'Open original page',
  'menu.reveal': 'Show in folder', 'menu.removeLibrary': 'Delete from library…',

  'del.title': 'Delete this song?', 'del.titleMany': 'Delete {n} songs?',
  'del.message': 'They will be removed from the library and from playlists.', 'del.confirm': 'Delete',
  'del.alsoFiles': 'Also move the files to the Recycle Bin',

  'pl.new': 'New playlist', 'pl.newSmart': 'New smart playlist', 'pl.namePlaceholder': 'Playlist name', 'pl.name': 'Name',
  'pl.add': 'Add a playlist', 'pl.import': 'Import an M3U playlist…', 'pl.export': 'Export as M3U…', 'pl.rename': 'Rename',
  'pl.editRules': 'Edit rules', 'pl.kind': 'Playlist', 'pl.smart': 'Smart', 'pl.empty': 'Empty playlist',
  'pl.emptyTitle': 'This playlist is empty', 'pl.emptyText': 'Drag songs here from another view, or use “Add to a playlist” in their menu.',
  'pl.emptySmart': 'No song matches the rules yet.', 'pl.deleteTitle': 'Delete “{name}”?',
  'pl.deleteText': 'The playlist will be deleted. Your songs stay in the library.',
  'pl.sidebarHint': 'No playlists yet. Click + to create one.',

  'smart.presets': 'Templates', 'smart.rules': 'Rules', 'smart.matchAll': 'All rules', 'smart.matchAny': 'Any rule',
  'smart.addRule': 'Add a rule', 'smart.days': 'days', 'smart.minutes': 'min', 'smart.sortBy': 'Sort by', 'smart.limit': 'Limit',
  'smart.limitHint': '0 = no limit', 'smart.matches_one': '{n} song matches', 'smart.matches_other': '{n} songs match',
  'preset.mostPlayed': 'Most played', 'preset.recent': 'Recently added', 'preset.neverPlayed': 'Never played',
  'preset.recentlyPlayed': 'Recently played', 'preset.genre': 'By genre',
  'field.playCount': 'Plays', 'field.lastPlayed': 'Last played', 'field.addedAt': 'Date added', 'field.genre': 'Genre', 'field.artist': 'Artist',
  'field.album': 'Album', 'field.title': 'Title', 'field.year': 'Year', 'field.duration': 'Duration (min)',
  'op.is': 'is', 'op.isNot': 'is not', 'op.gt': 'greater than', 'op.lt': 'less than', 'op.contains': 'contains',
  'op.notContains': 'does not contain', 'op.within': 'in the last', 'op.notWithin': 'not in the last',

  'search.placeholder': 'Search', 'search.title': 'Search', 'search.resultsFor': 'Results for “{q}”',
  'search.startTitle': 'Search your library', 'search.startText': 'Songs, albums, artists, videos and playlists.',
  'search.noneTitle': 'No results', 'search.noneText': 'Nothing matches “{q}”.',

  'player.shuffle': 'Shuffle', 'player.prev': 'Previous', 'player.next': 'Next', 'player.repeat': 'Repeat', 'player.play': 'Play',
  'player.pause': 'Pause', 'player.seek': 'Position', 'player.mute': 'Mute', 'player.volume': 'Volume', 'player.queue': 'Queue',
  'player.nothing': 'Not playing', 'player.nothingSub': 'Pick a song',

  'np.title': 'Now Playing', 'np.open': 'Open Now Playing', 'np.close': 'Minimize', 'np.fullscreen': 'Full screen',
  'queue.title': 'Queue', 'queue.clear': 'Clear upcoming', 'queue.empty': 'The queue is empty.', 'queue.remove': 'Remove from queue',
  'eq.title': 'Equalizer', 'eq.hint': 'Double-click a slider to reset it.', 'eq.custom': 'Custom',
  'eq.flat': 'Flat', 'eq.bass': 'Bass boost', 'eq.treble': 'Treble boost', 'eq.vocal': 'Vocal', 'eq.rock': 'Rock', 'eq.electronic': 'Electronic',
  'eq.classical': 'Classical', 'eq.acoustic': 'Acoustic', 'eq.jazz': 'Jazz', 'eq.loudness': 'Loudness',

  'nav.dropTitle': 'Drop to add', 'drop.title': 'Drop to add to the library', 'drop.text': 'Audio files, videos or whole folders.',

  'dl.subtitle': 'Paste a YouTube, SoundCloud, Bandcamp… link', 'dl.placeholder': 'Paste a link (video or playlist)', 'dl.paste': 'Paste',
  'dl.invalidUrl': 'That is not a valid link (it must start with https://).', 'dl.needTools': 'Install the tools below first.',
  'dl.analyzing': 'Analyzing link…', 'dl.analyzeFailed': 'Could not analyze this link.', 'dl.videoKind': 'Video',
  'dl.playlistKind': 'Playlist · {n} items', 'dl.audio': 'Audio', 'dl.video': 'Video', 'dl.createPlaylist': 'Create a playlist',
  'dl.download': 'Download', 'dl.downloadAll': 'Download all ({n})', 'dl.added': 'Added to the download queue',
  'dl.openFolder': 'Open folder', 'dl.emptyQueue': 'No downloads yet.', 'dl.inProgress': 'In progress', 'dl.finished': 'Finished',
  'dl.clear': 'Clear list', 'dl.itemOf': 'Item {i} of {n}', 'dl.eta': '{t} left', 'dl.starting': 'Starting…', 'dl.processing': 'Converting and tagging…',
  'dl.queued': 'Waiting', 'dl.canceled': 'Canceled', 'dl.failed': 'Download failed', 'dl.doneTracks_one': '{n} file added', 'dl.doneTracks_other': '{n} files added',
  'dl.retry': 'Retry', 'dl.openPlaylist': 'Open playlist', 'dl.notYetInLibrary': 'Files are still being added to the library, try again in a moment.',
  'dl.doneToast': 'Downloaded: {title}',

  'tools.needTitle': 'Download tools required', 'tools.needText': 'yt-dlp and ffmpeg will be downloaded once into the app’s data folder.',
  'tools.install': 'Install', 'tools.installing': 'Installing {tool}…', 'tools.extracting': 'Extracting…', 'tools.preparing': 'Preparing…',
  'tools.ready': 'Tools ready · yt-dlp {v}', 'tools.update': 'Update yt-dlp', 'tools.updating': 'Updating yt-dlp…', 'tools.updated': 'yt-dlp updated ({v})',
  'tools.upToDate': 'yt-dlp is up to date', 'tools.updateFailed': 'Update failed', 'tools.installed': 'Tools installed', 'tools.failed': 'Installation failed',
  'tools.addDeno': 'Add deno (YouTube)', 'tools.installedShort': 'installed', 'tools.missing': 'not installed',
  'tools.denoNote': 'Needed for some YouTube videos',

  'scan.discover': 'Searching… {n} files', 'scan.read': 'Analyzing {done} / {total}',

  'set.tab.general': 'General', 'set.tab.library': 'Library', 'set.tab.downloads': 'Downloads', 'set.tab.about': 'About',
  'set.appearance': 'Appearance', 'set.theme': 'Theme', 'set.accent': 'Accent color', 'set.ambient': 'Colored background', 'set.ambientHint': 'Album art tints the window',
  'set.compact': 'Compact lists', 'set.compactHint': 'Tighter rows', 'set.animations': 'Animations', 'set.language': 'Language',
  'set.languageHint': 'Changing the language restarts the interface.',
  'set.folders': 'Watched folders', 'set.foldersHint': 'New files are detected automatically.', 'set.noFolders': 'No folders yet.',
  'set.addFolder': 'Add a folder…', 'set.scanNow': 'Scan now', 'set.scanning': 'Scanning…', 'set.inLibrary': 'In the library',
  'set.dlFolder': 'Downloads folder', 'set.dlBehavior': 'Behavior', 'set.concurrency': 'Simultaneous downloads',
  'set.autoUpdate': 'Update yt-dlp automatically', 'set.autoUpdateHint': 'Once a day, at launch', 'set.tools': 'Tools',
  'set.shortcuts': 'Keyboard shortcuts', 'set.data': 'Data', 'set.reset': 'Reset settings', 'set.resetConfirm': 'Click again to confirm',

  'about.tagline': 'Music, videos and downloads.', 'about.version': 'Version {v}', 'about.portable': 'Portable edition', 'about.installed': 'Installed edition',
  'about.dataDir': 'Open data folder', 'about.credits': 'Lucide icons · Inter typeface · Electron · yt-dlp · ffmpeg',

  'theme.auto': 'Auto', 'theme.light': 'Light', 'theme.dark': 'Dark',
  'accent.blue': 'Blue', 'accent.purple': 'Purple', 'accent.pink': 'Pink', 'accent.red': 'Red', 'accent.orange': 'Orange', 'accent.yellow': 'Yellow',
  'accent.green': 'Green', 'accent.graphite': 'Graphite', 'accent.custom': 'Custom',
  'lang.auto': 'Automatic', 'lang.fr': 'Français', 'lang.en': 'English',

  'kb.playPause': 'Play / pause', 'kb.next': 'Next song', 'kb.prev': 'Previous song', 'kb.seekForward': 'Forward 10 s', 'kb.seekBack': 'Back 10 s',
  'kb.volUp': 'Volume up', 'kb.volDown': 'Volume down', 'kb.shuffle': 'Shuffle', 'kb.repeat': 'Repeat', 'kb.mute': 'Mute', 'kb.queue': 'Queue',
  'kb.eq': 'Equalizer', 'kb.nowPlaying': 'Open Now Playing', 'kb.search': 'Search', 'kb.back': 'Go back', 'kb.settings': 'Settings',
  'kb.fullscreen': 'Full screen', 'kb.escape': 'Close',

  'toast.added_one': 'Added to “{name}”', 'toast.added_other': '{n} songs added to “{name}”', 'toast.playNext': 'Will play next', 'toast.queued': 'Added to the queue',
  'toast.playlistCreated': 'Playlist “{name}” created', 'toast.playlistCreatedWith_one': 'Playlist “{name}” created with 1 song',
  'toast.playlistCreatedWith_other': 'Playlist “{name}” created with {n} songs', 'toast.playlistDeleted': 'Playlist deleted',
  'toast.removed_one': 'Song removed from the library', 'toast.removed_other': '{n} songs removed from the library',
  'toast.trashed_one': 'Moved to the Recycle Bin', 'toast.trashed_other': '{n} files moved to the Recycle Bin',
  'toast.exported': 'Playlist exported ({n} songs)', 'toast.imported': 'Playlist imported: {n} of {total} songs found', 'toast.saved': 'Saved',
  'toast.cannotPlay': 'Cannot play: {title}', 'toast.scanDone_one': '1 file added', 'toast.scanDone_other': '{n} files added',
  'toast.droppedNone': 'No audio or video files recognized',
};

const DICTS = { fr, en };
let lang = 'fr';
let plural = new Intl.PluralRules('fr');

export function resolveLanguage(pref) {
  if (pref === 'fr' || pref === 'en') return pref;
  return (navigator.language || 'fr').toLowerCase().startsWith('fr') ? 'fr' : 'en';
}

export function setLanguage(pref) {
  lang = resolveLanguage(pref);
  plural = new Intl.PluralRules(lang);
  document.documentElement.lang = lang;
  return lang;
}

export const currentLanguage = () => lang;

export function t(key, vars) {
  const dict = DICTS[lang];
  let s;
  if (vars && 'n' in vars) {
    const form = plural.select(Number(vars.n));
    s = dict[`${key}_${form}`] ?? dict[`${key}_other`] ?? fr[`${key}_${form}`] ?? fr[`${key}_other`];
  }
  s ??= dict[key] ?? fr[key] ?? key;
  if (vars) for (const [k, v] of Object.entries(vars)) s = s.replaceAll(`{${k}}`, String(v));
  return s;
}

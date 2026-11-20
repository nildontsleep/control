//! État de la fenêtre : bibliothèque dérivée, navigation, sélection, recherche, menus et feuilles ouverts.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use rune::{AppHandle, Image, ImageCache, PointerPress, TextEdit, Tx, Waker};
use serde_json::{Value, json};

use control_core::catalog::{Catalog, SearchResults};
use control_core::model::{Event, PlaylistId, ScanProgress, TrackId};
use control_core::settings::Settings;

use crate::backend::Backend;
use crate::player::Player;

/// Une vue de la fenêtre principale (avec son paramètre), comme les adresses `#/…` de l'original.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Route {
    Songs,
    Albums,
    Album(String),
    Artists,
    Artist(String),
    Videos,
    Playlist(PlaylistId),
    Search,
    Downloads,
    Stats,
    Watch(crate::views::watch::WatchRoute),
}

impl Route {
    /// Nom et paramètre enregistrés dans `lastView`.
    pub fn to_value(&self) -> Value {
        let (name, param) = match self {
            Route::Songs => ("songs", Value::Null),
            Route::Albums => ("albums", Value::Null),
            Route::Album(k) => ("album", json!(k)),
            Route::Artists => ("artists", Value::Null),
            Route::Artist(k) => ("artist", json!(k)),
            Route::Videos => ("videos", Value::Null),
            Route::Playlist(id) => ("playlist", json!(id)),
            Route::Search => ("songs", Value::Null),
            Route::Downloads => ("downloads", Value::Null),
            Route::Stats => ("stats", Value::Null),
            Route::Watch(w) => match w {
                crate::views::watch::WatchRoute::Anime => ("anime", Value::Null),
                crate::views::watch::WatchRoute::MyList => ("mylist", Value::Null),
                _ => ("watch", Value::Null),
            },
        };
        json!({ "name": name, "param": param })
    }

    pub fn from_saved(name: &str, param: &Value) -> Route {
        match name {
            "albums" => Route::Albums,
            "album" => param.as_str().map(|k| Route::Album(k.into())).unwrap_or(Route::Albums),
            "artists" => Route::Artists,
            "artist" => param.as_str().map(|k| Route::Artist(k.into())).unwrap_or(Route::Artists),
            "videos" => Route::Videos,
            "playlist" => param
                .as_i64()
                .or_else(|| param.as_str().and_then(|s| s.parse().ok()))
                .map(Route::Playlist)
                .unwrap_or(Route::Songs),
            "downloads" => Route::Downloads,
            "stats" => Route::Stats,
            "watch" => Route::Watch(crate::views::watch::WatchRoute::Movies),
            "anime" => Route::Watch(crate::views::watch::WatchRoute::Anime),
            "mylist" => Route::Watch(crate::views::watch::WatchRoute::MyList),
            _ => Route::Songs,
        }
    }
}

/// Ce sur quoi porte un menu contextuel.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuTarget {
    /// Des morceaux (la sélection, ou celui sous le pointeur), avec l'entrée de playlist éventuelle.
    Tracks { ids: Vec<TrackId>, entries: Vec<i64>, playlist: Option<PlaylistId> },
    Album(String),
    Artist(String),
    Playlist(PlaylistId),
    /// Une entrée de la file d'attente (son indice).
    Queue(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenMenu {
    pub at: PointerPress,
    pub target: MenuTarget,
    /// Sous-menu « Ajouter à une playlist » ouvert.
    pub submenu: bool,
}

/// Feuilles modales.
#[derive(Debug, Clone, PartialEq)]
pub enum Sheet {
    Settings(SettingsTab),
    NewPlaylist { tracks: Vec<TrackId> },
    RenamePlaylist(PlaylistId),
    ConfirmDelete { tracks: Vec<TrackId> },
    Equalizer,
    /// Éditeur de tags (son état : `Ui::tag_editor`).
    TagEditor,
    /// Règles d'une playlist intelligente (état : `Ui::smart`).
    Smart,
    /// Nom, description et pochette d'une playlist (état : `Ui::smart`).
    PlaylistEdit,
    /// Récap (état : `Ui::recap`).
    Recap,
    /// Coller ou modifier les paroles du morceau en cours.
    LyricsEdit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Appearance,
    Library,
    Downloads,
    Streaming,
    About,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub text: String,
    pub icon: &'static str,
    pub until: f64,
}

/// Sélection dans une liste de morceaux (identifiants), avec l'ancre des clics avec Maj.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Selection {
    pub ids: HashSet<TrackId>,
    pub anchor: Option<TrackId>,
}

/// Pochettes chargées (bornées : les plus anciennes sont oubliées).
pub struct CoverCache {
    images: HashMap<(String, bool), Image>,
    order: VecDeque<(String, bool)>,
}

const COVER_CACHE_MAX: usize = 600;

impl CoverCache {
    fn new() -> CoverCache {
        CoverCache { images: HashMap::new(), order: VecDeque::new() }
    }
}

pub struct Ui {
    pub handle: Option<AppHandle<Ui>>,
    pub backend: Arc<Backend>,
    pub settings: Settings,
    pub cat: Arc<Catalog>,
    /// Augmente à chaque nouveau catalogue (version des `memo`).
    pub cat_rev: u64,
    pub ready: bool,
    rebuild: RebuildState,

    pub route: Route,
    pub back: Vec<Route>,
    pub forward: Vec<Route>,

    pub search: TextEdit,
    pub results: Option<SearchResults>,
    pub scan: Option<ScanProgress>,

    pub selection: Selection,
    pub menu: Option<OpenMenu>,
    pub sheet: Option<Sheet>,
    pub name_edit: TextEdit,
    /// Champ de texte d'un réglage en cours d'édition (clé TMDB…).
    pub field_edit: TextEdit,
    /// Case « mettre aussi les fichiers à la corbeille » de la suppression.
    pub delete_files: bool,
    pub toasts: Vec<Toast>,
    toast_seq: u64,
    pub now_playing: bool,

    pub player: Player,
    pub stats: crate::views::stats::StatsState,
    pub recap: crate::views::recap::RecapState,
    pub downloads: crate::views::downloads::DownloadsState,
    pub watch: crate::views::watch::WatchState,
    pub tag_editor: crate::sheets::tag_editor::TagEditorState,
    pub smart: crate::sheets::smart::SmartState,
    pub live: crate::ui::playerbar::LiveBits,
    pub lyrics: crate::ui::lyrics::LyricsState,
    pub lyric_line: Option<usize>,
    /// Mots allumés de la ligne courante (mode immersif).
    lit_words: usize,
    pub lyrics_edit: TextEdit,
    /// Mode immersif (plein écran : pochette et paroles).
    pub immersive: bool,
    pub video: crate::video::VideoView,
    /// ffmpeg (relu de temps en temps : il peut être installé pendant que l'application tourne).
    ffmpeg: Option<std::path::PathBuf>,
    ffmpeg_checked: f64,
    /// Présence Discord : ce qui a été envoyé, et quand recommencer.
    presence: control_core::discord::PresenceFeed,
    presence_due: f64,
    presence_last: (Option<TrackId>, bool),
    /// Vignettes de vidéos : chemin une fois extraite, `None` si l'extraction a échoué.
    thumbs: RefCell<HashMap<TrackId, Option<std::path::PathBuf>>>,
    thumbs_wanted: RefCell<Vec<TrackId>>,
    pub waker: Option<Waker>,
    images: Option<ImageCache>,
    covers: RefCell<CoverCache>,
    pub scale: f32,
    /// Derniers états publiés par les services (forme JSON d'origine).
    pub update_state: Value,
    pub discord_status: Value,
    pub scrobble_status: Value,
    pub loudness_progress: Value,
    /// Place du bouton « Plus » d'une playlist dans la fenêtre (pour y ouvrir son menu).
    pub more_rect: std::cell::Cell<[f32; 4]>,
    pub clock: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum RebuildState {
    #[default]
    Idle,
    Running,
    /// Un changement est arrivé pendant la reconstruction : on recommencera.
    Again,
}

impl Ui {
    pub fn new(backend: Arc<Backend>, handle: Option<AppHandle<Ui>>) -> Ui {
        let settings = backend.store.settings();
        crate::i18n::set_language(&settings.language);
        let route = Route::from_saved(&settings.last_view.name, &settings.last_view.param);
        let player = Player::new(&settings);
        Ui {
            handle,
            backend,
            settings,
            cat: Arc::new(Catalog::default()),
            cat_rev: 0,
            ready: false,
            rebuild: RebuildState::Idle,
            route,
            back: Vec::new(),
            forward: Vec::new(),
            search: TextEdit::new(""),
            results: None,
            scan: None,
            selection: Selection::default(),
            menu: None,
            sheet: None,
            name_edit: TextEdit::new(""),
            field_edit: TextEdit::new(""),
            delete_files: false,
            toasts: Vec::new(),
            toast_seq: 0,
            now_playing: false,
            player,
            stats: Default::default(),
            recap: Default::default(),
            downloads: Default::default(),
            watch: Default::default(),
            tag_editor: Default::default(),
            smart: Default::default(),
            live: crate::ui::playerbar::LiveBits::new(),
            lyrics: Default::default(),
            lyric_line: None,
            lit_words: 0,
            lyrics_edit: TextEdit::new(""),
            immersive: false,
            video: Default::default(),
            ffmpeg: None,
            ffmpeg_checked: f64::NEG_INFINITY,
            presence: Default::default(),
            presence_due: 0.0,
            presence_last: (None, false),
            thumbs: RefCell::new(HashMap::new()),
            thumbs_wanted: RefCell::new(Vec::new()),
            waker: None,
            images: None,
            covers: RefCell::new(CoverCache::new()),
            scale: 2.0,
            update_state: Value::Null,
            discord_status: Value::Null,
            scrobble_status: Value::Null,
            loudness_progress: Value::Null,
            more_rect: std::cell::Cell::new([400.0, 200.0, 0.0, 0.0]),
            clock: 0.0,
        }
    }

    /// Lance `work` sur un thread, puis `apply` sur le thread de l'interface avec son résultat.
    pub fn spawn<R: Send + 'static>(
        &self,
        work: impl FnOnce(&Backend) -> R + Send + 'static,
        apply: impl FnOnce(&mut Ui, R, &mut Tx) + Send + 'static,
    ) {
        let backend = self.backend.clone();
        let handle = self.handle.clone();
        let spawned = std::thread::Builder::new().name("control-job".into()).spawn(move || {
            let out = work(&backend);
            if let Some(h) = handle {
                h.update(move |s, tx| apply(s, out, tx));
            }
        });
        if let Err(err) = spawned {
            log::error!("Thread de travail impossible : {err}");
        }
    }

    /// Lance `work` sur un thread sans rien rapporter.
    pub fn spawn_quiet(&self, work: impl FnOnce(&Backend) + Send + 'static) {
        self.spawn(work, |_, (), _| {});
    }

    // ─── Bibliothèque ────────────────────────────────────────────────────────

    /// Reconstruit le catalogue sur un thread (un seul à la fois, les demandes se regroupent).
    pub fn reload_library(&mut self) {
        match self.rebuild {
            RebuildState::Idle => {}
            _ => {
                self.rebuild = RebuildState::Again;
                return;
            }
        }
        self.rebuild = RebuildState::Running;
        self.spawn(
            |b| {
                let tracks = b.library.tracks();
                let (playlists, items) = b.library.playlists();
                Catalog::build(tracks, playlists, items)
            },
            |s, cat, _| {
                s.cat = Arc::new(cat);
                s.cat_rev += 1;
                let first = !s.ready;
                s.ready = true;
                let again = s.rebuild == RebuildState::Again;
                s.rebuild = RebuildState::Idle;
                s.library_loaded(first);
                if again {
                    s.reload_library();
                }
            },
        );
    }

    /// Le catalogue vient d'arriver : la file suit, la recherche aussi.
    fn library_loaded(&mut self, first: bool) {
        let cat = self.cat.clone();
        self.player.library_changed(&cat, &self.backend);
        if first {
            self.player.restore_session(&self.settings, &cat);
            // Rendus de démonstration : `CONTROL_DEMO_PLAY=<titre>` lance ce morceau au chargement.
            if let Some(title) = std::env::var_os("CONTROL_DEMO_PLAY") {
                let title = title.to_string_lossy().into_owned();
                let ids: Vec<TrackId> = cat.audio.iter().map(|&i| cat.entries[i].track.id).collect();
                let start = cat.audio.iter().position(|&i| cat.entries[i].track.title == title).unwrap_or(0);
                let backend = self.backend.clone();
                self.player.play_list(&ids, start, Some(false), &cat, &backend);
            }
        }
        self.selection.ids.retain(|id| cat.by_id.contains_key(id));
        if !self.search.text.trim().is_empty() {
            self.results = cat.search(&self.search.text);
        }
        if let Route::Playlist(id) = self.route
            && cat.playlist(id).is_none()
        {
            self.route = Route::Songs;
        }
    }

    /// Un événement du noyau (déjà sur le thread de l'interface).
    pub fn on_event(&mut self, event: Event) {
        match event {
            Event::LibraryChanged(_) => self.reload_library(),
            Event::ScanProgress(p) => {
                let done = p.phase == "done";
                self.scan = if done { None } else { Some(p) };
            }
            Event::Json(channel, payload) => {
                if channel.starts_with("dl:") || channel.starts_with("tools:") {
                    crate::views::downloads::on_event(self, channel, &payload);
                } else if channel.starts_with("watch:") {
                    crate::views::watch::on_event(self, channel, &payload);
                } else {
                    self.on_service_event(channel, &payload);
                }
            }
        }
    }

    /// Analyse les dossiers de la bibliothèque.
    pub fn rescan(&self) {
        self.spawn_quiet(|b| {
            let roots = b.scan_roots();
            b.library.scan(&roots);
        });
    }

    /// Autres événements des services (scrobbling, Discord, mises à jour, normalisation).
    fn on_service_event(&mut self, channel: &str, payload: &Value) {
        match channel {
            "update:state" => self.update_state = payload.clone(),
            "discord:status" => self.discord_status = payload.clone(),
            "scrobble:changed" => self.scrobble_status = payload.clone(),
            "loudness:progress" => self.loudness_progress = payload.clone(),
            _ => {}
        }
    }

    // ─── Réglages ────────────────────────────────────────────────────────────

    /// Modifie des réglages (fusion profonde), les enregistre et applique leurs effets.
    pub fn set(&mut self, patch: Value) {
        self.backend.store.patch(&patch);
        self.settings = self.backend.store.settings();
        if patch.get("language").is_some() {
            crate::i18n::set_language(&self.settings.language);
            self.presence.invalidate();
        }
        if patch.get("discord").is_some() {
            self.backend.discord.configure_from(&self.settings.discord);
            self.presence.invalidate();
        }
        if patch.get("normalize").and_then(Value::as_bool) == Some(true) {
            self.backend.loudness.kick();
        }
        self.player.apply_settings(&self.settings);
        if patch.get("libraryFolders").is_some() || patch.get("downloadDir").is_some() {
            self.rescan();
        }
    }

    // ─── Navigation ──────────────────────────────────────────────────────────

    pub fn navigate(&mut self, route: Route) {
        if route == self.route {
            return;
        }
        let old = std::mem::replace(&mut self.route, route);
        self.back.push(old);
        if self.back.len() > 100 {
            self.back.remove(0);
        }
        self.forward.clear();
        self.after_navigate();
    }

    pub fn go_back(&mut self) {
        if let Some(r) = self.back.pop() {
            let cur = std::mem::replace(&mut self.route, r);
            self.forward.push(cur);
            self.after_navigate();
        }
    }

    pub fn go_forward(&mut self) {
        if let Some(r) = self.forward.pop() {
            let cur = std::mem::replace(&mut self.route, r);
            self.back.push(cur);
            self.after_navigate();
        }
    }

    fn after_navigate(&mut self) {
        self.selection = Selection::default();
        self.menu = None;
        self.now_playing = false;
        match self.route.clone() {
            Route::Stats => crate::views::stats::on_open(self),
            Route::Downloads => crate::views::downloads::on_open(self),
            Route::Watch(w) => crate::views::watch::on_open(self, &w),
            _ => {}
        }
        if self.route != Route::Search {
            let v = self.route.to_value();
            self.backend.store.patch(&json!({ "lastView": v }));
        }
    }

    /// Le texte de recherche a changé.
    pub fn search_changed(&mut self) {
        let q = self.search.text.clone();
        if q.trim().is_empty() {
            self.results = None;
            if self.route == Route::Search {
                self.go_back();
            }
            return;
        }
        self.results = self.cat.search(&q);
        if self.route != Route::Search {
            self.navigate(Route::Search);
        }
    }

    // ─── Retours visuels ─────────────────────────────────────────────────────

    pub fn toast(&mut self, text: impl Into<String>, icon: &'static str) {
        self.toast_seq += 1;
        self.toasts.push(Toast { id: self.toast_seq, text: text.into(), icon, until: self.clock + 3.2 });
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
    }

    // ─── Pochettes ───────────────────────────────────────────────────────────

    pub fn set_images(&mut self, waker: Waker) {
        self.images = Some(ImageCache::new(waker.clone()));
        self.waker = Some(waker);
    }

    /// Image d'une pochette (petite : 160 px, grande : 640 px). `None` tant qu'aucun cache n'existe.
    pub fn cover(&self, hash: Option<&str>, small: bool) -> Option<Image> {
        let hash = hash?;
        let images = self.images.as_ref()?;
        let key = (hash.to_string(), small);
        let mut cache = self.covers.borrow_mut();
        if let Some(img) = cache.images.get(&key) {
            return Some(img.clone());
        }
        let path = self.backend.covers.path(hash, small)?;
        let size = if small { 80.0 } else { 320.0 };
        let img = images.file(path, size, self.scale);
        cache.images.insert(key.clone(), img.clone());
        cache.order.push_back(key);
        while cache.order.len() > COVER_CACHE_MAX {
            if let Some(old) = cache.order.pop_front() {
                cache.images.remove(&old);
            }
        }
        Some(img)
    }

    /// Vignette d'une vidéo, demandée au besoin (extraite par ffmpeg sur un thread).
    pub fn video_thumb(&self, id: TrackId) -> Option<Image> {
        let known = self.thumbs.borrow().get(&id).cloned();
        match known {
            Some(Some(path)) => {
                let images = self.images.as_ref()?;
                let key = (format!("thumb:{}", path.display()), false);
                let mut cache = self.covers.borrow_mut();
                if let Some(img) = cache.images.get(&key) {
                    return Some(img.clone());
                }
                let img = images.file(&path, 240.0, self.scale);
                cache.images.insert(key.clone(), img.clone());
                cache.order.push_back(key);
                Some(img)
            }
            Some(None) => None,
            None => {
                let mut wanted = self.thumbs_wanted.borrow_mut();
                if !wanted.contains(&id) {
                    wanted.push(id);
                }
                None
            }
        }
    }

    /// Lance l'extraction des vignettes demandées pendant le dernier passage de la vue.
    fn fetch_thumbs(&mut self) {
        let wanted: Vec<TrackId> = std::mem::take(&mut *self.thumbs_wanted.borrow_mut());
        if wanted.is_empty() {
            return;
        }
        for id in &wanted {
            self.thumbs.borrow_mut().insert(*id, None);
        }
        self.spawn(
            move |b| {
                let ffmpeg = b.ffmpeg();
                wanted.into_iter().map(|id| (id, b.library.video_thumb(id, ffmpeg.as_deref()))).collect::<Vec<_>>()
            },
            |s, out, _| {
                let mut thumbs = s.thumbs.borrow_mut();
                for (id, path) in out {
                    thumbs.insert(id, path);
                }
            },
        );
    }

    // ─── Horloge ─────────────────────────────────────────────────────────────

    /// À chaque image : lecture, vignettes à extraire. Renvoie `true` si la vue doit être reconstruite.
    pub fn tick(&mut self, clock: f64, tx: &mut Tx) -> bool {
        self.fetch_thumbs();
        let (cat, backend) = (self.cat.clone(), self.backend.clone());
        let was = (self.player.current_id(), self.player.is_playing());
        let changed = self.player.tick(clock, &cat, &backend);
        let now = (self.player.current_id(), self.player.is_playing());
        let position = self.player.position();
        self.live.update(position, self.player.duration());
        let mut changed = changed;
        if clock - self.ffmpeg_checked > 10.0 {
            self.ffmpeg_checked = clock;
            self.ffmpeg = self.backend.ffmpeg();
        }
        // Vidéo affichée : une image toutes les 1/30 s ; sinon la position suffit à 10 images par seconde.
        let showing_video = (self.now_playing || self.immersive) && self.player.track.as_ref().is_some_and(|t| t.is_video());
        if showing_video {
            if let (Some(item), Some(track)) = (self.player.queue.current(), self.player.track.clone()) {
                changed |= self.video.update(self.ffmpeg.as_ref(), item.uid, &track, position);
            }
        } else {
            self.video.stop();
        }
        // Paroles visibles : chargées pour le morceau en cours, ligne active suivie.
        let lyrics_shown = self.immersive || (self.now_playing && self.settings.np_tab == "lyrics");
        if lyrics_shown {
            if self.lyrics.track() != self.player.current_id() {
                crate::ui::lyrics::ensure(self, false);
                changed = true;
            }
            if crate::ui::lyrics::follow(self, position) {
                changed = true;
                if let Some(i) = self.lyric_line
                    && self.now_playing
                {
                    tx.scroll_into_view(format!("lyr-{i}"));
                }
            }
            if self.immersive {
                let lit = match (self.lyrics.lrc(), self.lyric_line) {
                    (Some(lrc), Some(i)) => lrc.lit_words(i, position),
                    _ => 0,
                };
                if lit != self.lit_words {
                    self.lit_words = lit;
                    changed = true;
                }
            }
        }
        if now.1 {
            tx.wake_after(std::time::Duration::from_millis(if showing_video { 16 } else { 100 }));
        }
        self.feed_presence(clock, now);
        changed || was != now
    }

    /// Présence Discord : environ toutes les 1,5 s pendant la lecture, chaque minute en pause, et à chaque changement.
    fn feed_presence(&mut self, clock: f64, now: (Option<TrackId>, bool)) {
        let now_ms = control_core::util::now_ms();
        let mut force = false;
        if now.0 != self.presence_last.0 {
            self.presence.track_changed();
            force = true;
        }
        if now.1 != self.presence_last.1 || force {
            self.presence.playback_changed(now.1, now_ms);
            force = true;
        }
        self.presence_last = now;
        if !force && clock < self.presence_due {
            return;
        }
        self.presence_due = clock + if now.1 { 1.5 } else { 60.0 };
        let np = control_core::discord::NowPlaying {
            track: self.player.track.as_ref(),
            playing: now.1,
            position: self.player.position(),
            duration: self.player.duration(),
            streaming: None,
            paused_since: self.presence.paused_since(),
        };
        let lang = if crate::i18n::current() == crate::i18n::Lang::En { "en" } else { "fr" };
        let texts = control_core::discord::Texts::for_language(lang);
        let activity = control_core::discord::build_activity(&np, &self.settings.discord, &texts, now_ms);
        if self.presence.should_send(activity.as_ref(), now_ms) {
            self.backend.discord.set_activity(activity);
        }
    }

    // ─── Dialogues ───────────────────────────────────────────────────────────

    /// Choisit un dossier et l'ajoute à la bibliothèque.
    pub fn choose_library_folder(&mut self, tx: &mut Tx) {
        let options = rune::FileDialogOptions { title: crate::i18n::t("empty.addFolder"), ..Default::default() };
        tx.open_folder(options, |s: &mut Ui, result, _| {
            if let Ok(Some(path)) = result {
                let dir = path.to_string_lossy().into_owned();
                let mut folders = s.settings.library_folders.clone();
                if !folders.contains(&dir) {
                    folders.push(dir);
                    s.set(json!({ "libraryFolders": folders }));
                }
            }
        });
    }

    /// Exporte une playlist en M3U8.
    pub fn export_playlist(&mut self, id: PlaylistId, tx: &mut Tx) {
        let Some(pl) = self.cat.playlist(id) else { return };
        let ids: Vec<TrackId> = self
            .cat
            .playlist_rows(pl, control_core::util::now_ms())
            .iter()
            .map(|r| self.cat.entries[r.entry].track.id)
            .collect();
        let options = rune::FileDialogOptions {
            title: crate::i18n::t("pl.export"),
            file_name: Some(format!("{}.m3u8", control_core::tags::safe_name(&pl.name))),
            ..Default::default()
        }
        .filter("M3U", &["m3u8", "m3u"]);
        tx.save_file(options, move |s: &mut Ui, result, _| {
            if let Ok(Some(path)) = result {
                let ids = ids.clone();
                s.spawn(
                    move |b| b.library.export_m3u(&path, &ids),
                    |s, res, _| match res {
                        Ok(n) => s.toast(crate::i18n::tn("toast.exported", n), "check"),
                        Err(err) => s.toast(err.to_string(), "triangle-alert"),
                    },
                );
            }
        });
    }

    // ─── Sélection ───────────────────────────────────────────────────────────

    /// Clic sur une ligne : seule, ajoutée (Ctrl/Cmd) ou étendue (Maj) dans l'ordre `list`.
    pub fn select(&mut self, id: TrackId, list: &[TrackId], toggle: bool, extend: bool) {
        if extend && let Some(anchor) = self.selection.anchor {
            let a = list.iter().position(|x| *x == anchor);
            let b = list.iter().position(|x| *x == id);
            if let (Some(a), Some(b)) = (a, b) {
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                if !toggle {
                    self.selection.ids.clear();
                }
                self.selection.ids.extend(list[lo..=hi].iter().copied());
                return;
            }
        }
        if toggle {
            if !self.selection.ids.remove(&id) {
                self.selection.ids.insert(id);
            }
        } else {
            self.selection.ids.clear();
            self.selection.ids.insert(id);
        }
        self.selection.anchor = Some(id);
    }

    /// Les morceaux visés par un clic droit : la sélection si le morceau en fait partie, sinon lui seul.
    pub fn targets(&self, id: TrackId, list: &[TrackId]) -> Vec<TrackId> {
        if self.selection.ids.contains(&id) && self.selection.ids.len() > 1 {
            list.iter().copied().filter(|x| self.selection.ids.contains(x)).collect()
        } else {
            vec![id]
        }
    }
}

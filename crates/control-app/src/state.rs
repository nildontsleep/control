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
    pub live: crate::ui::playerbar::LiveBits,
    /// Vignettes de vidéos : chemin une fois extraite, `None` si l'extraction a échoué.
    thumbs: RefCell<HashMap<TrackId, Option<std::path::PathBuf>>>,
    thumbs_wanted: RefCell<Vec<TrackId>>,
    pub waker: Option<Waker>,
    images: Option<ImageCache>,
    covers: RefCell<CoverCache>,
    pub scale: f32,
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
            live: crate::ui::playerbar::LiveBits::new(),
            thumbs: RefCell::new(HashMap::new()),
            thumbs_wanted: RefCell::new(Vec::new()),
            waker: None,
            images: None,
            covers: RefCell::new(CoverCache::new()),
            scale: 2.0,
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
            Event::Json(_, _) => {}
        }
    }

    /// Analyse les dossiers de la bibliothèque.
    pub fn rescan(&self) {
        self.spawn_quiet(|b| {
            let roots = b.scan_roots();
            b.library.scan(&roots);
        });
    }

    // ─── Réglages ────────────────────────────────────────────────────────────

    /// Modifie des réglages (fusion profonde), les enregistre et applique leurs effets.
    pub fn set(&mut self, patch: Value) {
        self.backend.store.patch(&patch);
        self.settings = self.backend.store.settings();
        if patch.get("language").is_some() {
            crate::i18n::set_language(&self.settings.language);
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
                let ffmpeg = control_core::process::which("ffmpeg");
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
        self.live.update(self.player.position(), self.player.duration());
        if now.1 {
            tx.wake_after(std::time::Duration::from_millis(100));
        }
        changed || was != now
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

//! Films, séries et animés : accueils TMDB et AniList, « Tout voir », fiche d'un titre, « Ma liste » et recherche
//! dans les catalogues (port de `src/renderer/js/views/{watch,title,shelf,mylist,watching}.js` et de
//! `src/renderer/js/media/{ui,api,play}.js`).
//!
//! Tout passe par [`control_core::media::MediaService`], appelé sur des threads de travail (`Ui::spawn`) ; les
//! réponses arrivent dans [`WatchState`]. Rune n'a pas de vue web : le lecteur vidrift s'ouvre dans le navigateur
//! du système (voir [`play`]).

mod cards;
mod episodes;
mod home;
mod images;
mod mylist;
mod play;
mod shelf;
mod title;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rune::{Cx, El, ScrollSpec, TextEdit, col};
use serde_json::Value;

use control_core::media::{
    CatalogItem, Details, ErrorCode, Home, MediaError, Page, Progress, SearchResults, Season, WatchlistItem,
};

use crate::state::Ui;
use crate::theme::Palette;

pub use images::RemoteImages;

/// Pages de la partie « Regarder ».
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WatchRoute {
    /// Accueil films et séries (onglet dans [`WatchState::tab`]).
    Movies,
    /// Accueil animés.
    Anime,
    /// Ma liste.
    MyList,
    /// Fiche d'un titre : `kind` vaut `movie`, `tv` ou `anime`.
    Title { kind: String, id: String },
    /// « Tout voir » d'une rangée ou d'un genre : source du service (`tmdb:movie/popular`, `anime:genre:Action`…)
    /// et titre de la page.
    Shelf { source: String, title: String },
    /// Recherche dans les catalogues en ligne.
    Search(String),
}

impl WatchRoute {
    /// Entrée de la barre latérale à mettre en avant (`watch`, `anime` ou `mylist`).
    pub fn section(&self) -> &'static str {
        match self {
            WatchRoute::Movies | WatchRoute::Search(_) => "watch",
            WatchRoute::Anime => "anime",
            WatchRoute::MyList => "mylist",
            WatchRoute::Title { kind, .. } => {
                if kind == "anime" {
                    "anime"
                } else {
                    "watch"
                }
            }
            WatchRoute::Shelf { source, .. } => {
                if source.starts_with("anime:") {
                    "anime"
                } else {
                    "watch"
                }
            }
        }
    }
}

/// Onglets de l'accueil films et séries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    All,
    Movie,
    Tv,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::All, Tab::Movie, Tab::Tv];

    /// Nom côté service (`all`, `movie`, `tv`), aussi la clé i18n `tab.*`.
    pub fn as_str(self) -> &'static str {
        match self {
            Tab::All => "all",
            Tab::Movie => "movie",
            Tab::Tv => "tv",
        }
    }

    pub fn parse(s: &str) -> Tab {
        match s {
            "movie" => Tab::Movie,
            "tv" => Tab::Tv,
            _ => Tab::All,
        }
    }

    /// Ce type de titre a-t-il sa place dans cet onglet ?
    pub fn accepts(self, kind: &str) -> bool {
        match self {
            Tab::All => kind != "anime",
            Tab::Movie => kind == "movie",
            Tab::Tv => kind == "tv",
        }
    }
}

/// Une réponse attendue du service.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Load<T> {
    #[default]
    Loading,
    Ready(T),
    Failed(MediaError),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(v) => Some(v),
            _ => None,
        }
    }

    fn from(res: Result<T, MediaError>) -> Load<T> {
        match res {
            Ok(v) => Load::Ready(v),
            Err(e) => Load::Failed(e),
        }
    }
}

/// Fiche ouverte.
#[derive(Debug, Default)]
pub struct TitlePage {
    pub kind: String,
    pub id: String,
    pub details: Load<Details>,
    /// Progression de ce titre, la plus récente d'abord.
    pub progress: Vec<Progress>,
    /// Saison affichée (séries).
    pub season: Option<i64>,
    pub seasons: HashMap<i64, Load<Season>>,
    /// Groupe de 50 épisodes affiché (animés).
    pub group: usize,
}

/// Page « Tout voir ».
#[derive(Debug, Default)]
pub struct ShelfPage {
    pub source: String,
    pub items: Vec<CatalogItem>,
    pub page: i64,
    pub total_pages: i64,
    pub loading: bool,
    pub error: Option<MediaError>,
}

/// Dernier épisode ouvert dans le navigateur : la fiche propose alors le précédent et le suivant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    pub kind: String,
    pub id: String,
    pub season: Option<i64>,
    pub episode: Option<i64>,
}

/// État de la carte de saisie de la clé TMDB.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum KeyStatus {
    #[default]
    Idle,
    Testing,
    Invalid,
    Network,
}

pub struct WatchState {
    pub tab: Tab,
    /// Accueils par onglet (`all`, `movie`, `tv`, `anime`) : gardés pour revenir sans attendre.
    pub homes: HashMap<&'static str, Load<Home>>,
    pub continuing: Vec<Progress>,
    pub history: Vec<Progress>,
    pub list: Vec<WatchlistItem>,
    /// « Ma liste » et la progression ont été lues au moins une fois.
    pub local_loaded: bool,
    pub title: TitlePage,
    pub shelf: ShelfPage,
    /// Champ de recherche des catalogues, et la dernière recherche lancée.
    pub query: TextEdit,
    pub search: Option<(String, Load<SearchResults>)>,
    /// Diapositive affichée dans la bannière, et le minuteur qui la fait tourner.
    pub hero: usize,
    hero_timer: u64,
    pub hero_paused: bool,
    pub key_edit: TextEdit,
    pub key_status: KeyStatus,
    pub opened: Option<Opened>,
    /// Second clic attendu pour effacer l'historique.
    pub confirm_clear: bool,
    pub images: RemoteImages,
    /// Position des rangées horizontales (pour les flèches) : décalage, largeur visible, maximum.
    offsets: Rc<RefCell<HashMap<String, [f32; 3]>>>,
}

impl Default for WatchState {
    fn default() -> Self {
        WatchState {
            tab: Tab::All,
            homes: HashMap::new(),
            continuing: Vec::new(),
            history: Vec::new(),
            list: Vec::new(),
            local_loaded: false,
            title: TitlePage::default(),
            shelf: ShelfPage::default(),
            query: TextEdit::new(""),
            search: None,
            hero: 0,
            hero_timer: 0,
            hero_paused: false,
            key_edit: TextEdit::new(""),
            key_status: KeyStatus::Idle,
            opened: None,
            confirm_clear: false,
            images: RemoteImages::default(),
            offsets: Rc::new(RefCell::new(HashMap::new())),
        }
    }
}

impl WatchState {
    /// Le titre est-il dans « Ma liste » ?
    pub fn in_list(&self, kind: &str, id: &str) -> bool {
        self.list.iter().any(|e| e.kind == kind && e.id == id)
    }
}

/// Langue des catalogues (celle de l'interface).
fn lang() -> &'static str {
    match crate::i18n::current() {
        crate::i18n::Lang::En => "en",
        crate::i18n::Lang::Fr => "fr",
    }
}

// ─── Chargements ────────────────────────────────────────────────────────────────────────────────────────────────

/// Relit « Ma liste », la reprise et l'historique.
pub fn load_local(ui: &Ui) {
    ui.spawn(
        |b| {
            let m = &b.media;
            (m.watchlist_list(), m.watch_continue(), m.watch_history())
        },
        |s, (list, continuing, history), _| {
            // Base indisponible : on affiche ce qu'on a.
            if let Ok(v) = list {
                s.watch.list = v;
            }
            if let Ok(v) = continuing {
                s.watch.continuing = v;
            }
            if let Ok(v) = history {
                s.watch.history = v;
            }
            s.watch.local_loaded = true;
        },
    );
}

/// Accueil d'un onglet (`all`, `movie`, `tv` ou `anime`). La version déjà affichée reste en place pendant le
/// chargement ; une erreur ne remplace pas un accueil déjà chargé.
pub fn load_home(ui: &mut Ui, kind: &'static str) {
    let had = ui.watch.homes.get(kind).is_some_and(|h| h.ready().is_some());
    if !had {
        ui.watch.homes.insert(kind, Load::Loading);
    }
    let lang = lang();
    ui.spawn(
        move |b| b.media.home(kind, lang),
        move |s, res, _| {
            if res.is_err() && s.watch.homes.get(kind).is_some_and(|h| h.ready().is_some()) {
                return;
            }
            s.watch.homes.insert(kind, Load::from(res));
            s.watch.hero = 0;
        },
    );
}

/// Ouvre la fiche `kind`/`id` (ou la recharge).
pub fn load_title(ui: &mut Ui, kind: &str, id: &str) {
    if ui.watch.title.kind != kind || ui.watch.title.id != id {
        ui.watch.title = TitlePage { kind: kind.into(), id: id.into(), ..Default::default() };
    } else if matches!(ui.watch.title.details, Load::Failed(_)) {
        ui.watch.title.details = Load::Loading;
    }
    let (k, i) = (kind.to_string(), id.to_string());
    let lang = lang();
    ui.spawn(
        move |b| {
            let d = b.media.details(&k, &i, lang);
            let rows = b.media.watch_show(&k, &i).unwrap_or_default();
            (k, i, d, rows)
        },
        |s, (k, i, d, rows), _| {
            let page = &mut s.watch.title;
            if page.kind != k || page.id != i {
                return;
            }
            let first = page.details.ready().is_none();
            page.details = Load::from(d);
            page.progress = rows;
            if first {
                title::pick_start(s);
            }
        },
    );
}

/// Relit la progression du titre ouvert.
pub fn load_progress(ui: &Ui) {
    let (k, i) = (ui.watch.title.kind.clone(), ui.watch.title.id.clone());
    if k.is_empty() {
        return;
    }
    ui.spawn(
        move |b| (b.media.watch_show(&k, &i).unwrap_or_default(), k, i),
        |s, (rows, k, i), _| {
            if s.watch.title.kind == k && s.watch.title.id == i {
                s.watch.title.progress = rows;
            }
        },
    );
}

/// Épisodes d'une saison de la série ouverte.
pub fn load_season(ui: &mut Ui, number: i64) {
    let page = &mut ui.watch.title;
    if page.seasons.get(&number).is_some_and(|l| l.ready().is_some()) {
        return;
    }
    page.seasons.insert(number, Load::Loading);
    let id = page.id.clone();
    let lang = lang();
    ui.spawn(
        move |b| (b.media.season(&id, number, lang), id),
        move |s, (res, id), _| {
            if s.watch.title.kind == "tv" && s.watch.title.id == id {
                s.watch.title.seasons.insert(number, Load::from(res));
            }
        },
    );
}

/// Page suivante de « Tout voir ».
pub fn load_shelf_more(ui: &mut Ui) {
    let sp = &mut ui.watch.shelf;
    if sp.loading || (sp.page > 0 && sp.page >= sp.total_pages) {
        return;
    }
    sp.loading = true;
    sp.error = None;
    let (source, next) = (sp.source.clone(), sp.page + 1);
    let lang = lang();
    ui.spawn(
        move |b| (b.media.list(&source, next, lang), source),
        |s, (res, source): (Result<Page<CatalogItem>, MediaError>, String), _| {
            let sp = &mut s.watch.shelf;
            if sp.source != source {
                return;
            }
            sp.loading = false;
            match res {
                Ok(page) => {
                    sp.page = page.page.max(sp.page + 1);
                    sp.total_pages = page.total_pages.max(sp.page);
                    if page.items.is_empty() {
                        sp.total_pages = sp.page;
                    }
                    sp.items.extend(page.items);
                }
                Err(e) => sp.error = Some(e),
            }
        },
    );
}

/// Lance la recherche en ligne de `query` (deux caractères au moins, comme l'original).
pub fn load_search(ui: &mut Ui, query: &str) {
    let q = query.trim().to_string();
    if ui.watch.search.as_ref().is_some_and(|(prev, l)| *prev == q && !matches!(l, Load::Failed(_))) {
        return;
    }
    ui.watch.search = Some((q.clone(), Load::Loading));
    let lang = lang();
    ui.spawn(
        move |b| (b.media.search(&q, lang), q),
        |s, (res, q), _| {
            if s.watch.search.as_ref().is_some_and(|(prev, _)| *prev == q) {
                s.watch.search = Some((q, Load::Ready(res)));
            }
        },
    );
}

/// Fait tourner la bannière toutes les 8 s tant que l'accueil est affiché (un minuteur à la fois).
fn start_hero_timer(ui: &mut Ui) {
    ui.watch.hero_timer += 1;
    let token = ui.watch.hero_timer;
    ui.spawn(
        |_| std::thread::sleep(std::time::Duration::from_secs(8)),
        move |s, (), _| {
            if s.watch.hero_timer != token
                || !matches!(s.route, crate::state::Route::Watch(WatchRoute::Movies | WatchRoute::Anime))
            {
                return;
            }
            if !s.watch.hero_paused {
                s.watch.hero = s.watch.hero.wrapping_add(1);
            }
            start_hero_timer(s);
        },
    );
}

/// La page `route` vient de s'ouvrir.
pub fn on_open(ui: &mut Ui, route: &WatchRoute) {
    ui.watch.confirm_clear = false;
    match route {
        WatchRoute::Movies => {
            let tab = ui.watch.tab.as_str();
            load_home(ui, tab);
            load_local(ui);
            start_hero_timer(ui);
        }
        WatchRoute::Anime => {
            load_home(ui, "anime");
            load_local(ui);
            start_hero_timer(ui);
        }
        WatchRoute::MyList => load_local(ui),
        WatchRoute::Title { kind, id } => {
            load_title(ui, kind, id);
            load_local(ui);
        }
        WatchRoute::Shelf { source, .. } => {
            if ui.watch.shelf.source != *source || ui.watch.shelf.error.is_some() {
                ui.watch.shelf = ShelfPage { source: source.clone(), ..Default::default() };
                load_shelf_more(ui);
            }
            load_local(ui);
        }
        WatchRoute::Search(q) => {
            if ui.watch.query.text != *q {
                ui.watch.query = TextEdit::new(q.clone());
            }
            load_search(ui, q);
            load_local(ui);
        }
    }
}

/// Événements `watch:*` du noyau.
pub fn on_event(ui: &mut Ui, channel: &str, payload: &Value) {
    if channel != "watch:changed" {
        return;
    }
    match payload["what"].as_str() {
        Some("list") => load_local(ui),
        _ => {
            load_local(ui);
            load_progress(ui);
        }
    }
}

/// L'erreur vient-elle d'une clé TMDB absente ?
pub fn is_no_key(err: &MediaError) -> bool {
    err.code == ErrorCode::NoKey
}

/// Page défilante verticale de la partie « Regarder ».
pub(crate) fn page(key: String, content: Vec<El>) -> El {
    col()
        .key(key)
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: false })
        .scroll_edge_fade(8.0)
        .child(col().w_full().px(32.0).pb(130.0).children(content))
}

pub fn view(ui: &Ui, p: &Palette, cx: &mut Cx<Ui>, route: &WatchRoute) -> El {
    match route {
        WatchRoute::Movies => home::view(ui, p, cx, false),
        WatchRoute::Anime => home::view(ui, p, cx, true),
        WatchRoute::MyList => mylist::view(ui, p, cx),
        WatchRoute::Title { kind, id } => title::view(ui, p, cx, kind, id),
        WatchRoute::Shelf { source, title } => shelf::view(ui, p, cx, source, title),
        WatchRoute::Search(q) => shelf::search_view(ui, p, cx, q),
    }
}

/// Ouvre une page directement (scènes des rendus sans fenêtre).
pub fn open_scene(ui: &mut Ui, route: WatchRoute) {
    ui.route = crate::state::Route::Watch(route.clone());
    on_open(ui, &route);
}

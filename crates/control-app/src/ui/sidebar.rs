//! Barre latérale translucide : recherche, bibliothèque, téléchargements, playlists, réglages.

use rune::{Cursor, El, Icon, ScrollSpec, Tx, col, el, icon, row, text};
use rune_kit::{Kit, TextInput};

use control_core::model::PlaylistKind;

use crate::i18n::t;
use crate::icons;
use crate::state::{MenuTarget, OpenMenu, Route, Sheet, SettingsTab, Ui};
use crate::theme::Palette;
use crate::ui::common::{CoverKind, cover, ty};

pub const WIDTH: f32 = 252.0;

fn heading(p: &Palette, label: String) -> El {
    text(label).font(ty::CAPTION).color(p.text3).px(10.0).pt(14.0).pb(6.0)
}

fn item(p: &Palette, key: String, glyph: El, label: String, active: bool, badge: Option<String>) -> El {
    let mut r = row()
        .key(key)
        .w_full()
        .h(32.0)
        .px(10.0)
        .gap(10.0)
        .radius(p.r(8.0))
        .items_center()
        .cursor(Cursor::Pointer)
        .role("link")
        .label(label.clone())
        .hover(p.fill)
        .child(glyph)
        .child(text(label).font(ty::BODY_MEDIUM).color(p.text).truncate().grow(1.0).min_w(0.0));
    if active {
        r = r.fill(p.accent_soft);
    }
    if let Some(b) = badge {
        r = r.child(
            row()
                .h(18.0)
                .min_w(18.0)
                .px(5.0)
                .radius(9.0)
                .center()
                .fill(p.accent)
                .child(text(b).font(ty::CAPTION).color(p.on_accent)),
        );
    }
    r
}

fn nav(p: &Palette, ui: &Ui, route: Route, glyph: &'static Icon, label_key: &str, active: bool) -> El {
    let key = format!("nav-{label_key}");
    item(p, key, icon(glyph, 17.0).color(p.accent), t(label_key), active, None)
        .on_press(move |s: &mut Ui| s.navigate(route.clone()))
        .key(format!("nav-{}", label_key))
        .label(t(label_key))
        .checked(active)
        .focusable(true)
        .tab_index(0)
        .on_secondary_press(|_: &mut Ui, _, _| {})
        .opacity(if ui.ready { 1.0 } else { 0.6 })
}

pub fn sidebar(ui: &Ui, p: &Palette, k: &Kit) -> El {
    let r = &ui.route;
    let in_section = |names: &[&str]| -> bool {
        let name = match r {
            Route::Songs => "songs",
            Route::Albums | Route::Album(_) => "albums",
            Route::Artists | Route::Artist(_) => "artists",
            Route::Videos => "videos",
            Route::Stats => "stats",
            Route::Downloads => "downloads",
            Route::Watch(crate::views::watch::WatchRoute::Movies) => "watch",
            Route::Watch(crate::views::watch::WatchRoute::Anime) => "anime",
            Route::Watch(crate::views::watch::WatchRoute::MyList) => "mylist",
            Route::Playlist(_) | Route::Search => "",
        };
        names.contains(&name)
    };

    let search = TextInput::new(&ui.search)
        .placeholder(t("search.placeholder"))
        .leading_icon(&icons::SEARCH)
        .on_edit(|s: &mut Ui| &mut s.search)
        .on_change(|s: &mut Ui, _tx: &mut Tx| s.search_changed())
        .build(k)
        .key("sb-search");

    let mut list = col().w_full().gap(1.0);
    list = list.child(heading(p, t("nav.library")));
    for (route, glyph, key, sec) in [
        (Route::Songs, &icons::MUSIC_2, "nav.songs", "songs"),
        (Route::Albums, &icons::DISC_3, "nav.albums", "albums"),
        (Route::Artists, &icons::MIC_VOCAL, "nav.artists", "artists"),
        (Route::Videos, &icons::FILM, "nav.videos", "videos"),
        (Route::Stats, &icons::CHART_COLUMN, "nav.stats", "stats"),
    ] {
        list = list.child(nav(p, ui, route, glyph, key, in_section(&[sec])));
    }
    list = list.child(heading(p, t("nav.watch")));
    for (route, glyph, key, sec) in [
        (Route::Watch(crate::views::watch::WatchRoute::Movies), &icons::CLAPPERBOARD, "nav.movies", "watch"),
        (Route::Watch(crate::views::watch::WatchRoute::Anime), &icons::TV_MINIMAL, "nav.anime", "anime"),
        (Route::Watch(crate::views::watch::WatchRoute::MyList), &icons::BOOKMARK, "nav.mylist", "mylist"),
    ] {
        list = list.child(nav(p, ui, route, glyph, key, in_section(&[sec])));
    }
    list = list.child(heading(p, t("nav.get")));
    let active_downloads = ui.downloads.active();
    list = list.child(
        item(
            p,
            "nav-downloads".into(),
            icon(&icons::CLOUD_DOWNLOAD, 17.0).color(p.accent),
            t("nav.downloads"),
            in_section(&["downloads"]),
            (active_downloads > 0).then(|| active_downloads.to_string()),
        )
        .on_press(|s: &mut Ui| s.navigate(Route::Downloads)),
    );

    // Playlists, avec un bouton « + » pour en créer une.
    let add = row()
        .key("sb-new-playlist")
        .size(22.0, 22.0)
        .radius(6.0)
        .center()
        .cursor(Cursor::Pointer)
        .hover(p.fill_hover)
        .role("button")
        .label(t("pl.new"))
        .help(t("pl.new"))
        .child(icon(&icons::PLUS, 15.0).color(p.text2))
        .on_press(|s: &mut Ui| {
            s.name_edit = rune::TextEdit::new("");
            s.name_edit.focused = true;
            s.sheet = Some(Sheet::NewPlaylist { tracks: Vec::new() });
        });
    let add_smart = row()
        .key("sb-new-smart")
        .size(22.0, 22.0)
        .radius(6.0)
        .center()
        .cursor(Cursor::Pointer)
        .hover(p.fill_hover)
        .role("button")
        .label(t("pl.newSmart"))
        .help(t("pl.newSmart"))
        .child(icon(&icons::SPARKLES, 14.0).color(p.text2))
        .on_press(|s: &mut Ui| crate::sheets::smart::open_smart(s, None));
    list = list.child(
        row()
            .w_full()
            .items_center()
            .justify_between()
            .pr(6.0)
            .child(heading(p, t("nav.playlists")))
            .child(row().gap(2.0).mt(8.0).child(add_smart).child(add)),
    );
    if ui.cat.playlists.is_empty() && ui.ready {
        list = list.child(text(t("pl.sidebarHint")).font(ty::SMALL).color(p.text3).wrap().px(10.0).py(4.0));
    }
    for pl in &ui.cat.playlists {
        let id = pl.id;
        let active = *r == Route::Playlist(id);
        let glyph = if pl.cover.is_some() {
            cover(ui, pl.cover.as_deref(), &pl.name, 18.0, 4.0, CoverKind::Playlist)
        } else {
            icon(if pl.kind == PlaylistKind::Smart { &icons::SPARKLES } else { &icons::LIST_MUSIC }, 17.0).color(p.accent)
        };
        list = list.child(
            item(p, format!("sb-pl-{id}"), glyph, pl.name.clone(), active, None)
                .on_press(move |s: &mut Ui| s.navigate(Route::Playlist(id)))
                .on_secondary_press(move |s: &mut Ui, at, _| {
                    s.menu = Some(OpenMenu { at, target: MenuTarget::Playlist(id), submenu: false });
                }),
        );
    }

    let footer = col().w_full().px(10.0).py(8.0).border_sides([1.0, 0.0, 0.0, 0.0], p.separator).child(
        item(p, "nav-settings".into(), icon(&icons::SETTINGS, 17.0).color(p.accent), t("nav.settings"), false, None)
            .on_press(|s: &mut Ui| s.sheet = Some(Sheet::Settings(SettingsTab::General))),
    );

    let status = ui.scan.as_ref().map(|sc| {
        let label = if sc.phase == "discover" {
            crate::i18n::tn("scan.discover", sc.found)
        } else {
            crate::i18n::tv("scan.read", &[("done", &sc.done), ("total", &sc.total)])
        };
        row().px(20.0).pb(6.0).gap(8.0).items_center().child(icon(&icons::LOADER, 13.0).color(p.text3)).child(
            text(label).font(ty::SMALL).color(p.text3).truncate(),
        )
    });

    col()
        .key("sidebar")
        .w(WIDTH)
        .h_full()
        .fill(p.sidebar)
        .backdrop_blur(30.0)
        .border_sides([0.0, 1.0, 0.0, 0.0], p.separator)
        .child(el().h(14.0))
        .child(row().px(12.0).pb(4.0).w_full().child(search.w_full()))
        .child(
            col()
                .key("sb-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .scroll_edge_fade(10.0)
                .child(list.px(10.0).pb(12.0)),
        )
        .children(status)
        .child(footer)
}

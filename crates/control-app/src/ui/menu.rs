//! Menus contextuels : morceaux (lire, file, playlist, artiste, album, dossier, suppression), albums, artistes,
//! playlists, entrées de la file.

use rune::{El, el};
use rune_kit::{Kit, MenuItem, context_menu as kit_menu};

use control_core::artists::{artist_key, split_artists};
use control_core::catalog::album_key_of;
use control_core::model::{PlaylistId, TrackId};

use crate::i18n::{t, tn, tv};
use crate::icons;
use crate::state::{MenuTarget, OpenMenu, Route, Sheet, Ui};
use crate::theme::Palette;

fn play(ids: Vec<TrackId>) -> impl Fn(&mut Ui) + 'static {
    move |s: &mut Ui| {
        let (cat, b) = (s.cat.clone(), s.backend.clone());
        s.player.play_list(&ids, 0, Some(false), &cat, &b);
        s.menu = None;
    }
}

fn play_next(ids: Vec<TrackId>) -> impl Fn(&mut Ui) + 'static {
    move |s: &mut Ui| {
        let (cat, b) = (s.cat.clone(), s.backend.clone());
        s.player.play_next(&ids, &cat, &b);
        s.menu = None;
        s.toast(t("toast.playNext"), "list-start");
    }
}

fn enqueue(ids: Vec<TrackId>) -> impl Fn(&mut Ui) + 'static {
    move |s: &mut Ui| {
        let (cat, b) = (s.cat.clone(), s.backend.clone());
        s.player.enqueue(&ids, &cat, &b);
        s.menu = None;
        s.toast(t("toast.queued"), "list-plus");
    }
}

/// Ajoute des morceaux à une playlist (sur un thread), avec un message.
pub fn add_to_playlist(s: &mut Ui, playlist: PlaylistId, ids: Vec<TrackId>) {
    let name = s.cat.playlist(playlist).map(|p| p.name.clone()).unwrap_or_default();
    let n = ids.len();
    s.spawn_quiet(move |b| b.library.add_to_playlist(playlist, &ids));
    s.menu = None;
    s.toast(tv("toast.added", &[("n", &n), ("name", &name)]), "check");
}

/// Entrées communes à toute liste de morceaux.
fn track_items(ui: &Ui, ids: &[TrackId], entries: &[i64], playlist: Option<PlaylistId>) -> Vec<MenuItem> {
    let mut items = Vec::new();
    let label = if ids.len() > 1 { tn("menu.playSelection", ids.len()) } else { t("menu.play") };
    items.push(MenuItem::new(label).icon(&icons::PLAY).on_select(play(ids.to_vec())));
    items.push(MenuItem::new(t("menu.playNext")).icon(&icons::LIST_START).on_select(play_next(ids.to_vec())));
    items.push(MenuItem::new(t("menu.addQueue")).icon(&icons::LIST_PLUS).on_select(enqueue(ids.to_vec())));

    // « Ajouter à une playlist » : sous-menu des playlists classiques, et « Nouvelle playlist… ».
    let mut sub: Vec<rune_kit::MenuEntry> = Vec::new();
    let new_ids = ids.to_vec();
    sub.push(rune_kit::MenuEntry::item(MenuItem::new(t("menu.newPlaylist")).icon(&icons::PLUS).on_select(move |s: &mut Ui| {
        s.menu = None;
        s.name_edit = rune::TextEdit::new("");
        s.name_edit.focused = true;
        s.sheet = Some(Sheet::NewPlaylist { tracks: new_ids.clone() });
    })));
    let manual: Vec<_> = ui.cat.playlists.iter().filter(|p| p.kind == control_core::model::PlaylistKind::Manual).collect();
    if !manual.is_empty() {
        sub.push(rune_kit::MenuEntry::Separator);
    }
    for pl in manual {
        let (pid, ids) = (pl.id, ids.to_vec());
        sub.push(rune_kit::MenuEntry::item(
            MenuItem::new(pl.name.clone()).icon(&icons::LIST_MUSIC).on_select(move |s: &mut Ui| add_to_playlist(s, pid, ids.clone())),
        ));
    }
    let open_sub = ui.menu.as_ref().is_some_and(|m| m.submenu);
    items.push(
        MenuItem::new(t("menu.addToPlaylist"))
            .icon(&icons::LIST_PLUS)
            .submenu(sub, open_sub)
            .on_select(|s: &mut Ui| {
                if let Some(m) = &mut s.menu {
                    m.submenu = !m.submenu;
                }
            }),
    );

    // Aller à l'artiste (chacun) et à l'album, pour un seul morceau.
    if ids.len() == 1
        && let Some(e) = ui.cat.entry(ids[0])
    {
        let names = split_artists(&e.track.artist);
        let names = if names.is_empty() { e.artist_names.clone() } else { names };
        for name in names.iter().take(6) {
            let key = artist_key(name);
            if !ui.cat.artist_by_key.contains_key(&key) {
                continue;
            }
            let label = if names.len() > 1 { tv("menu.goArtistNamed", &[("name", name)]) } else { t("menu.goArtist") };
            items.push(MenuItem::new(label).icon(&icons::MIC_VOCAL).on_select(move |s: &mut Ui| {
                s.menu = None;
                s.navigate(Route::Artist(key.clone()));
            }));
        }
        if let Some(ak) = album_key_of(&e.track).filter(|k| ui.cat.album_by_key.contains_key(k)) {
            items.push(MenuItem::new(t("menu.goAlbum")).icon(&icons::DISC_3).on_select(move |s: &mut Ui| {
                s.menu = None;
                s.navigate(Route::Album(ak.clone()));
            }));
        }
        let path = e.track.path.clone();
        items.push(MenuItem::new(t("menu.reveal")).icon(&icons::FOLDER_OPEN).on_select(move |s: &mut Ui| {
            s.menu = None;
            crate::os::reveal(&path);
        }));
        if let Some(url) = e.track.source_url.clone() {
            items.push(MenuItem::new(t("menu.openSource")).icon(&icons::EXTERNAL_LINK).on_select(move |s: &mut Ui| {
                s.menu = None;
                crate::os::open_url(&url);
            }));
        }
    }

    if let (Some(_), false) = (playlist, entries.is_empty()) {
        let entries = entries.to_vec();
        items.push(MenuItem::new(t("menu.removeFromPlaylist")).icon(&icons::X).on_select(move |s: &mut Ui| {
            s.menu = None;
            let e = entries.clone();
            s.spawn_quiet(move |b| b.library.remove_from_playlist(&e));
        }));
    }
    let edit = ids.to_vec();
    let shortcut = if cfg!(target_os = "macos") { "⌘I" } else { "Ctrl+I" };
    items.push(MenuItem::new(t("tag.menu")).icon(&icons::TAGS).shortcut(shortcut).on_select(move |s: &mut Ui| {
        crate::sheets::tag_editor::open(s, edit.clone());
    }));
    let del = ids.to_vec();
    items.push(MenuItem::new(t("menu.removeLibrary")).icon(&icons::TRASH_2).danger(true).on_select(move |s: &mut Ui| {
        s.menu = None;
        s.sheet = Some(Sheet::ConfirmDelete { tracks: del.clone() });
    }));
    items
}

fn tracks_of_album(ui: &Ui, key: &str) -> Vec<TrackId> {
    ui.cat.album_by_key.get(key).map(|&i| ui.cat.albums[i].tracks.iter().map(|&e| ui.cat.entries[e].track.id).collect()).unwrap_or_default()
}

fn tracks_of_artist(ui: &Ui, key: &str) -> Vec<TrackId> {
    ui.cat.artist_by_key.get(key).map(|&i| ui.cat.artists[i].tracks.iter().map(|&e| ui.cat.entries[e].track.id).collect()).unwrap_or_default()
}

fn playlist_items(ui: &Ui, id: PlaylistId) -> Vec<MenuItem> {
    let Some(pl) = ui.cat.playlist(id) else { return Vec::new() };
    let ids: Vec<TrackId> = ui.cat.playlist_rows(pl, control_core::util::now_ms()).iter().map(|r| ui.cat.entries[r.entry].track.id).collect();
    let mut items = vec![
        MenuItem::new(t("menu.play")).icon(&icons::PLAY).on_select(play(ids.clone())),
        MenuItem::new(t("menu.playNext")).icon(&icons::LIST_START).on_select(play_next(ids.clone())),
        MenuItem::new(t("menu.addQueue")).icon(&icons::LIST_PLUS).on_select(enqueue(ids)),
        MenuItem::new(t("pl.customize")).icon(&icons::PALETTE).on_select(move |s: &mut Ui| {
            crate::sheets::smart::open_customize(s, id);
        }),
        MenuItem::new(t("pl.rename")).icon(&icons::SQUARE_PEN).on_select(move |s: &mut Ui| {
            s.menu = None;
            let name = s.cat.playlist(id).map(|p| p.name.clone()).unwrap_or_default();
            s.name_edit = rune::TextEdit::new(name);
            s.name_edit.focused = true;
            s.sheet = Some(Sheet::RenamePlaylist(id));
        }),
    ];
    if pl.kind == control_core::model::PlaylistKind::Smart {
        items.push(MenuItem::new(t("pl.editRules")).icon(&icons::SLIDERS_HORIZONTAL).on_select(move |s: &mut Ui| {
            crate::sheets::smart::open_smart(s, Some(id));
        }));
    }
    items.push(MenuItem::new(t("pl.export")).icon(&icons::UPLOAD).on_select_tx(move |s: &mut Ui, tx| {
        s.menu = None;
        s.export_playlist(id, tx);
    }));
    items.push(MenuItem::new(t("common.delete")).icon(&icons::TRASH_2).danger(true).on_select(move |s: &mut Ui| {
        s.menu = None;
        s.spawn(move |b| b.library.delete_playlist(id), move |s: &mut Ui, (): (), _tx: &mut rune::Tx| {
            if s.route == Route::Playlist(id) {
                s.navigate(Route::Songs);
            }
            s.toast(t("toast.playlistDeleted"), "trash-2");
        });
    }));
    items
}

fn queue_items(i: usize) -> Vec<MenuItem> {
    vec![
        MenuItem::new(t("menu.play")).icon(&icons::PLAY).on_select(move |s: &mut Ui| {
            let (cat, b) = (s.cat.clone(), s.backend.clone());
            s.player.jump_to(i, &cat, &b);
            s.menu = None;
        }),
        MenuItem::new(t("common.remove")).icon(&icons::X).on_select(move |s: &mut Ui| {
            let cat = s.cat.clone();
            s.player.remove_at(i, &cat);
            s.menu = None;
        }),
    ]
}

pub fn context_menu(ui: &Ui, _p: &Palette, k: &Kit, m: &OpenMenu) -> El {
    let items = match &m.target {
        MenuTarget::Tracks { ids, entries, playlist } => track_items(ui, ids, entries, *playlist),
        MenuTarget::Album(key) => track_items(ui, &tracks_of_album(ui, key), &[], None),
        MenuTarget::Artist(key) => track_items(ui, &tracks_of_artist(ui, key), &[], None),
        MenuTarget::Playlist(id) => playlist_items(ui, *id),
        MenuTarget::Queue(i) => queue_items(*i),
    };
    let mut menu = kit_menu("ctx", el(), Some(m.at)).min_width(220.0).on_close(|s: &mut Ui| s.menu = None);
    for it in items {
        menu = menu.item(it);
    }
    let mut layer = Vec::new();
    let trigger = menu.build(k, &mut layer);
    el().absolute(0.0, 0.0).w_full().h_full().child(trigger).children(layer)
}

//! Feuilles modales : nouvelle playlist, renommage, suppression, réglages, tags, playlists intelligentes, récap.

pub mod smart;
pub mod tag_editor;

use rune::{El, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, Checkbox, Dialog, Kit, TextInput};

use control_core::model::{PlaylistKind, TrackId};

use crate::i18n::{t, tn, tv};
use crate::state::{Route, Sheet, Ui};
use crate::theme::Palette;
use crate::ui::common::ty;

fn footer(k: &Kit, cancel: &str, confirm: &str, danger: bool, on_confirm: impl Fn(&mut Ui, &mut Tx) + 'static) -> El {
    row()
        .w_full()
        .gap(8.0)
        .justify_end()
        .mt(16.0)
        .child(Button::new(cancel.to_string()).variant(ButtonVariant::Ghost).on_press(|s: &mut Ui| s.sheet = None).build(k))
        .child(
            Button::new(confirm.to_string())
                .variant(if danger { ButtonVariant::Danger } else { ButtonVariant::Contrast })
                .on_press_tx(on_confirm)
                .build(k),
        )
}

fn create_playlist(s: &mut Ui, tracks: Vec<TrackId>) {
    let name = s.name_edit.text.trim().to_string();
    let name = if name.is_empty() { t("pl.new") } else { name };
    s.sheet = None;
    s.spawn(
        move |b| {
            let id = b.library.create_playlist(&name, PlaylistKind::Manual, None);
            if let Some(id) = id
                && !tracks.is_empty()
            {
                b.library.add_to_playlist(id, &tracks);
            }
            (id, name, tracks.len())
        },
        |s, (id, name, n), _| {
            if let Some(id) = id {
                let msg = if n > 0 {
                    tv("toast.playlistCreatedWith", &[("n", &n), ("name", &name)])
                } else {
                    tv("toast.playlistCreated", &[("name", &name)])
                };
                s.toast(msg, "check");
                s.navigate(Route::Playlist(id));
            }
        },
    );
}

pub fn sheet(ui: &Ui, p: &Palette, k: &Kit, layer: &mut Vec<El>) {
    let Some(sheet) = &ui.sheet else { return };
    match sheet {
        Sheet::NewPlaylist { tracks } => {
            let tr = tracks.clone();
            let tr2 = tracks.clone();
            let input = TextInput::new(&ui.name_edit)
                .placeholder(t("pl.namePlaceholder"))
                .on_edit(|s: &mut Ui| &mut s.name_edit)
                .on_submit(move |s: &mut Ui, _| create_playlist(s, tr.clone()))
                .build(k);
            Dialog::new("sheet-new-playlist", t("pl.new"))
                .open(true)
                .width(420.0)
                .child(input)
                .child(footer(k, &t("common.cancel"), &t("common.create"), false, move |s, _| create_playlist(s, tr2.clone())))
                .on_close(|s: &mut Ui| s.sheet = None)
                .build(k, layer);
        }
        Sheet::RenamePlaylist(id) => {
            let id = *id;
            let rename = move |s: &mut Ui| {
                let name = s.name_edit.text.trim().to_string();
                s.sheet = None;
                s.spawn_quiet(move |b| b.library.rename_playlist(id, &name));
            };
            let input = TextInput::new(&ui.name_edit)
                .placeholder(t("pl.namePlaceholder"))
                .on_edit(|s: &mut Ui| &mut s.name_edit)
                .on_submit(move |s: &mut Ui, _| rename(s))
                .build(k);
            Dialog::new("sheet-rename", t("pl.rename"))
                .open(true)
                .width(420.0)
                .child(input)
                .child(footer(k, &t("common.cancel"), &t("common.rename"), false, move |s, _| rename(s)))
                .on_close(|s: &mut Ui| s.sheet = None)
                .build(k, layer);
        }
        Sheet::ConfirmDelete { tracks } => {
            let n = tracks.len();
            let title = if n > 1 { tn("del.titleMany", n) } else { t("del.title") };
            let ids = tracks.clone();
            let also = ui.delete_files;
            Dialog::new("sheet-delete", title)
                .open(true)
                .width(440.0)
                .child(text(t("del.message")).font(ty::BODY).color(p.text2).wrap())
                .child(
                    Checkbox::new(also)
                        .label(t("del.alsoFiles"))
                        .on_toggle(|s: &mut Ui| s.delete_files = !s.delete_files)
                        .build(k)
                        .mt(12.0),
                )
                .child(footer(k, &t("common.cancel"), &t("del.confirm"), true, move |s, _| {
                    let ids = ids.clone();
                    let files = s.delete_files;
                    s.sheet = None;
                    s.selection.ids.clear();
                    let n = ids.len();
                    s.spawn(move |b| b.library.remove_tracks(&ids, files), move |s, (), _| {
                        let key = if files { "toast.trashed" } else { "toast.removed" };
                        s.toast(tn(key, n), "trash-2");
                    });
                }))
                .on_close(|s: &mut Ui| s.sheet = None)
                .build(k, layer);
        }
        Sheet::Settings(_) | Sheet::Equalizer => crate::settings_ui::sheet(ui, p, k, layer),
        Sheet::TagEditor => tag_editor::sheet(ui, p, k, layer),
        Sheet::Smart | Sheet::PlaylistEdit => smart::sheet(ui, p, k, layer),
        Sheet::Recap => crate::views::recap::sheet(ui, p, k, layer),
        Sheet::LyricsEdit => {
            let save = |s: &mut Ui| {
                let Some(id) = s.player.current_id() else { return };
                let text = s.lyrics_edit.text.clone();
                s.sheet = None;
                s.spawn(
                    move |b| b.lyrics.set(id, &text).map_err(|e| e.to_string()),
                    move |s, r, _| match r {
                        Ok(result) => {
                            let lrc = result.parsed();
                            s.lyrics = crate::ui::lyrics::LyricsState::Ready { track: id, result, lrc };
                            s.lyric_line = None;
                            s.toast(t("toast.saved"), "check");
                        }
                        Err(e) => s.toast(e, "triangle-alert"),
                    },
                );
            };
            let area = rune_kit::TextArea::new(&ui.lyrics_edit)
                .placeholder(t("lyr.placeholder"))
                .on_edit(|s: &mut Ui| &mut s.lyrics_edit)
                .build(k)
                .h(320.0);
            let body = col()
                .w_full()
                .gap(10.0)
                .child(text(t("lyr.editHint")).font(ty::SMALL).color(p.text2).wrap())
                .child(area)
                .child(footer(k, &t("common.cancel"), &t("common.save"), false, move |s, _| save(s)));
            layer.push(panel(p, "sheet-lyrics", t("lyr.editTitle"), 580.0, 560.0, None, body));
        }
    }
    let _ = col();
}

/// Feuille modale à hauteur bornée : un voile, puis un panneau centré dont le corps défile.
/// Échap ou un clic hors du panneau la ferment.
pub fn panel(p: &Palette, key: &str, title: String, width: f32, max_h: f32, header: Option<El>, body: El) -> El {
    let close = row()
        .key(format!("{key}-close"))
        .size(28.0, 28.0)
        .radius(14.0)
        .center()
        .cursor(rune::Cursor::Pointer)
        .hover(p.fill_hover)
        .role("button")
        .label(t("common.close"))
        .child(rune::icon(&crate::icons::X, 16.0).color(p.text2))
        .on_press(|s: &mut Ui| s.sheet = None);
    let mut card = col()
        .key(key.to_string())
        .w(width)
        .h_full()
        .max_h(max_h)
        .radius(p.r(16.0))
        .fill(p.panel_solid)
        .border(1.0, p.panel_border)
        .shadow(30.0, 80.0, rune::Color::BLACK.alpha(if p.dark { 0.5 } else { 0.22 }))
        .clip_rounded(p.r(16.0))
        .role("dialog")
        .label(title.clone())
        .modal(true, |s: &mut Ui| s.sheet = None)
        .child(
            row()
                .w_full()
                .h(52.0)
                .px(18.0)
                .items_center()
                .justify_between()
                .child(text(title).font(ty::HEADLINE).color(p.text))
                .child(close),
        );
    if let Some(h) = header {
        card = card.child(h);
    }
    card = card.child(
        col()
            .key(format!("{key}-body"))
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .scroll(rune::ScrollSpec { stick_bottom: false })
            .scroll_edge_fade(10.0)
            .child(col().w_full().px(20.0).pb(20.0).child(body)),
    );
    row()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .py(40.0)
        .justify_center()
        .items_center()
        .fill(rune::Color::BLACK.alpha(0.34))
        .on_background_press(|s: &mut Ui| s.sheet = None)
        .child(card)
}

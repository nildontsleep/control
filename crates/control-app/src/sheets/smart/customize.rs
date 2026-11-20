//! Personnaliser une playlist (porté de `src/renderer/js/ui/playlist-edit.js`) : nom, description et pochette
//! (une image, la pochette d'un morceau, ou la mosaïque automatique). La pochette change tout de suite, comme
//! dans l'original ; le nom et la description à l'enregistrement.

use rune::{Cursor, El, TextEdit, col, row, text};
use rune_kit::{Button, ButtonVariant, Kit, TextArea, TextInput};

use control_core::model::PlaylistId;
use control_core::util::now_ms;

use super::super::tag_editor::widgets::{card, small_button};
use crate::i18n::t;
use crate::icons;
use crate::state::{Sheet, Ui};
use crate::theme::Palette;
use crate::ui::common::{CoverKind, cover, ty};

/// Pochettes proposées au plus dans la grille.
const PICK_MAX: usize = 120;

/// Ouvre la personnalisation (nom, description, pochette).
pub fn open_customize(ui: &mut Ui, playlist: PlaylistId) {
    let Some(pl) = ui.cat.playlist(playlist).cloned() else { return };
    let st = &mut ui.smart;
    st.playlist = Some(pl.id);
    st.name = TextEdit::new(pl.name);
    st.name.focused = true;
    st.description = TextEdit::new(pl.description);
    st.status = None;
    st.picking = false;
    st.busy = false;
    ui.menu = None;
    ui.sheet = Some(Sheet::PlaylistEdit);
}

/// Change la pochette tout de suite (sur un thread) ; `None` revient à la mosaïque.
fn set_cover(s: &mut Ui, id: PlaylistId, hash: Option<String>) {
    s.smart.status = None;
    s.smart.picking = false;
    s.spawn_quiet(move |b| b.library.set_playlist_cover(id, hash.as_deref()));
}

/// Choisit une image, l'enregistre parmi les pochettes et l'applique.
fn pick_image(id: PlaylistId, tx: &mut rune::Tx) {
    let options = rune::FileDialogOptions { title: t("pl.coverPick"), ..Default::default() }
        .filter("Images", &["jpg", "jpeg", "png", "webp", "gif", "bmp"]);
    tx.open_file(options, move |s: &mut Ui, result, _| {
        let Ok(Some(path)) = result else { return };
        s.smart.busy = true;
        s.smart.status = None;
        s.spawn(
            move |b| {
                let data = std::fs::read(&path)?;
                let hash = b.library.save_cover_image(&data)?;
                b.library.set_playlist_cover(id, Some(&hash));
                Ok::<_, std::io::Error>(())
            },
            |s, res, _| {
                s.smart.busy = false;
                if let Err(err) = res {
                    log::warn!("Pochette de playlist : {err}");
                    s.smart.status = Some(t("pl.coverError"));
                }
            },
        );
    });
}

fn save(s: &mut Ui) {
    let Some(id) = s.smart.playlist else { return };
    let old = s.cat.playlist(id).map(|p| p.name.clone()).unwrap_or_default();
    let name = s.smart.name.text.trim().to_string();
    let name = if name.is_empty() { old } else { control_core::util::clip(&name, 120).to_string() };
    let description = control_core::util::clip(s.smart.description.text.trim(), 500).to_string();
    s.sheet = None;
    s.spawn(
        move |b| {
            b.library.update_playlist(id, Some(&name), Some(&description), None);
        },
        |s, (), _| s.toast(t("pl.saved"), "check"),
    );
}

/// Pochettes différentes des morceaux de la playlist, avec le titre du premier morceau qui la porte.
fn track_covers(ui: &Ui, id: PlaylistId) -> Vec<(String, String)> {
    let Some(pl) = ui.cat.playlist(id) else { return Vec::new() };
    let mut out: Vec<(String, String)> = Vec::new();
    for r in ui.cat.playlist_rows(pl, now_ms()) {
        let tr = &ui.cat.entries[r.entry].track;
        if let Some(c) = &tr.cover
            && !out.iter().any(|(h, _)| h == c)
        {
            out.push((c.clone(), format!("{} — {}", tr.title, tr.artist)));
            if out.len() == PICK_MAX {
                break;
            }
        }
    }
    out
}

fn picker(ui: &Ui, p: &Palette, k: &Kit, id: PlaylistId) -> El {
    let covers = track_covers(ui, id);
    let back = small_button(t("tag.back"), Some(&icons::ARROW_LEFT))
        .on_press(|s: &mut Ui| s.smart.picking = false)
        .build(k)
        .key("pe-back");
    let mut out = col().w_full().gap(12.0).child(
        row()
            .gap(12.0)
            .items_center()
            .child(back)
            .child(text(t("pl.coverTrackTitle")).font(ty::HEADLINE).color(p.text)),
    );
    if covers.is_empty() {
        return out.child(text(t("pl.coverNone")).font(ty::BODY).color(p.text2).py(16.0));
    }
    let mut grid = row().w_full().gap(10.0).flex_wrap();
    for (i, (hash, label)) in covers.into_iter().enumerate() {
        let h = hash.clone();
        grid = grid.child(
            rune::el()
                .key(format!("pe-pick-{i}"))
                .cursor(Cursor::Pointer)
                .role("button")
                .label(label.clone())
                .help(label)
                .child(cover(ui, Some(&hash), "", 96.0, p.r(8.0), CoverKind::Music))
                .on_press(move |s: &mut Ui| set_cover(s, id, Some(h.clone()))),
        );
    }
    out = out.child(grid);
    out
}

fn field(p: &Palette, label: String, input: El) -> El {
    col().w_full().gap(6.0).child(text(label).font(ty::CAPTION).color(p.text3)).child(input)
}

fn form(ui: &Ui, p: &Palette, k: &Kit, id: PlaylistId) -> El {
    let st = &ui.smart;
    let buttons = col()
        .gap(6.0)
        .child(
            small_button(t("pl.coverPick"), Some(&icons::IMAGE_PLUS))
                .on_press_tx(move |_: &mut Ui, tx| pick_image(id, tx))
                .build(k)
                .key("pe-cover-pick"),
        )
        .child(
            small_button(t("pl.coverTrack"), Some(&icons::IMAGES))
                .on_press(|s: &mut Ui| s.smart.picking = true)
                .build(k)
                .key("pe-cover-track"),
        )
        .child(
            small_button(t("pl.coverAuto"), Some(&icons::LAYOUT_GRID))
                .on_press(move |s: &mut Ui| set_cover(s, id, None))
                .build(k)
                .key("pe-cover-auto"),
        );
    let mut side = col().gap(8.0).child(text(t("pl.cover")).font(ty::CAPTION).color(p.text3)).child(buttons);
    if let Some(msg) = &st.status {
        side = side.child(text(msg.clone()).font(ty::SMALL).color(p.danger).wrap());
    }
    let preview = crate::views::playlist::playlist_cover(ui, p, id, 150.0)
        .shadow(10.0, 30.0, rune::Color::BLACK.alpha(0.3))
        .opacity(if st.busy { 0.6 } else { 1.0 });
    let name = TextInput::new(&st.name)
        .width(None)
        .placeholder(t("pl.namePlaceholder"))
        .on_edit(|s: &mut Ui| &mut s.smart.name)
        .on_submit(|s: &mut Ui, _| save(s))
        .build(k)
        .key("pe-name");
    let description = TextArea::new(&st.description)
        .width(None)
        .rows(3)
        .placeholder(t("pl.descPlaceholder"))
        .on_edit(|s: &mut Ui| &mut s.smart.description)
        .on_submit(|s: &mut Ui, _| save(s))
        .build(k)
        .key("pe-description");
    col()
        .w_full()
        .gap(16.0)
        .child(row().w_full().gap(18.0).items_center().child(preview).child(side))
        .child(field(p, t("pl.name"), name))
        .child(field(p, t("pl.description"), description))
}

pub fn sheet(ui: &Ui, p: &Palette, k: &Kit, layer: &mut Vec<El>) {
    let Some(id) = ui.smart.playlist.filter(|id| ui.cat.playlist(*id).is_some()) else { return };
    let body = if ui.smart.picking { picker(ui, p, k, id) } else { form(ui, p, k, id) };
    let footer = row()
        .w_full()
        .gap(8.0)
        .justify_end()
        .child(Button::new(t("common.cancel")).on_press(|s: &mut Ui| s.sheet = None).build(k).key("pe-cancel"))
        .child(Button::new(t("common.save")).variant(ButtonVariant::Contrast).on_press(save).build(k).key("pe-save"));
    layer.push(card(p, "sheet-pl-edit", t("pl.editTitle"), (520.0, 700.0), body, footer, |s| s.sheet = None));
}

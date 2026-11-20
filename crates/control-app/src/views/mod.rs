//! Vues de la fenêtre principale.

pub mod albums;
pub mod artists;
pub mod playlist;
pub mod search;
pub mod songs;
pub mod videos;

use rune::{Cursor, El, col, row, text};
use serde_json::json;

use crate::i18n::t;
use crate::icons;
use crate::state::{Route, Ui};
use crate::theme::Palette;
use crate::ui::common::{button, empty_state, ty};

/// Bibliothèque vide : ajouter le dossier Musique, choisir un dossier, ou télécharger.
pub fn empty_library(_ui: &Ui, p: &Palette) -> El {
    let add_music = button(p, "empty-music", &t("empty.addMusic"), Some(&icons::FOLDER_PLUS), true).on_press(|s: &mut Ui| {
        if let Some(dir) = crate::paths::music_dir() {
            let dir = dir.to_string_lossy().into_owned();
            let mut folders = s.settings.library_folders.clone();
            if !folders.contains(&dir) {
                folders.push(dir);
            }
            s.set(json!({ "libraryFolders": folders }));
        }
    });
    let choose = button(p, "empty-folder", &t("empty.addFolder"), Some(&icons::FOLDER_OPEN), false)
        .on_press_tx(|s: &mut Ui, tx| s.choose_library_folder(tx));
    let dl = button(p, "empty-dl", &t("empty.download"), Some(&icons::CLOUD_DOWNLOAD), false).on_press(|s: &mut Ui| s.navigate(Route::Downloads));
    col().w_full().h_full().child(empty_state(p, &icons::MUSIC_2, &t("empty.libraryTitle"), &t("empty.libraryText"), vec![add_music, choose, dl]))
}

pub fn not_found(p: &Palette) -> El {
    col().w_full().h_full().center().child(text(t("common.notFound")).font(ty::BODY).color(p.text2))
}

/// Contrôle segmenté (tri des albums, des vidéos…).
pub fn segmented(p: &Palette, key: &str, options: Vec<(String, String)>, current: &str, set: fn(&mut Ui, String)) -> El {
    let mut r = row().key(key.to_string()).h(28.0).padding(2.0).gap(2.0).radius(p.r(8.0)).fill(p.fill).role("radiogroup");
    for (value, label) in options {
        let on = value == current;
        let v = value.clone();
        let mut seg = row()
            .key(format!("{key}-{value}"))
            .h_full()
            .px(10.0)
            .radius(p.r(6.0))
            .center()
            .cursor(Cursor::Pointer)
            .role("radio")
            .checked(on)
            .child(text(label).font(ty::SMALL_MEDIUM).color(if on { p.text } else { p.text2 }))
            .on_press(move |s: &mut Ui| set(s, v.clone()));
        if on {
            seg = seg.fill(p.seg_thumb);
        } else {
            seg = seg.hover(p.fill);
        }
        r = r.child(seg);
    }
    r
}

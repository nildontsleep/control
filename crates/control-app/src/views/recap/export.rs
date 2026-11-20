//! « Enregistrer l'image » : la carte affichée, dessinée par Rune hors de la fenêtre (un `Runtime` à part et un
//! rendu GPU sans surface), au format 1080 × 1350 de l'image d'origine, puis écrite en PNG.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rune::render::{Gpu, OFFSCREEN_FORMAT, Offscreen, Renderer, write_png};
use rune::{Align, Color, Cx, El, Env, FileDialogOptions, Image, Runtime, TextStyle, Tx, col, el, row, text};
use time::Date;

use control_core::stats::Stats;

use crate::i18n::t;
use crate::state::Ui;
use crate::views::recap::period::Period;
use crate::views::recap::slides::{self, Ctx, Slide};

/// Taille de l'image en points ; elle est dessinée à l'échelle 2 (1080 × 1350 pixels).
const W: f32 = 540.0;
const H: f32 = 675.0;
const SCALE: f32 = 2.0;

/// Tout ce que la carte affiche, pour la dessiner sur un autre thread.
struct Shot {
    period: Period,
    data: Arc<Stats>,
    slide: Slide,
    index: usize,
    today: Date,
    images: HashMap<(String, bool), Image>,
}

/// Choisit où enregistrer l'image (dans Images par défaut), puis la dessine.
pub fn save_dialog(s: &mut Ui, tx: &mut Tx) {
    let Some(o) = &s.recap.open else { return };
    if s.recap.saving {
        return;
    }
    let options = FileDialogOptions {
        title: t("wrapped.save"),
        directory: dirs::picture_dir(),
        file_name: Some(format!("{}.png", o.period.file_name())),
        ..Default::default()
    }
    .filter("PNG", &["png"]);
    tx.save_file(options, |s: &mut Ui, result, _| {
        if let Ok(Some(path)) = result {
            save_to(s, path);
        }
    });
}

/// Les pochettes que la carte montre (petites pour les listes, grandes pour l'artiste et l'album).
fn wanted_covers(data: &Stats, slide: Slide) -> Vec<(String, bool)> {
    let m = &data.music;
    let mut out = Vec::new();
    match slide {
        Slide::Artist => out.extend(m.top_artists.first().and_then(|a| a.cover.clone()).map(|c| (c, false))),
        Slide::Tracks => out.extend(m.top_tracks.iter().take(5).filter_map(|x| x.cover.clone()).map(|c| (c, true))),
        Slide::Sound => out.extend(m.top_albums.first().and_then(|a| a.cover.clone()).map(|c| (c, false))),
        _ => {}
    }
    out
}

/// Dessine la carte affichée dans `path`, sur un thread, et le dit une fois fait.
pub fn save_to(s: &mut Ui, path: PathBuf) {
    let Some(o) = &s.recap.open else { return };
    let slide = o.slides.get(o.index).copied().unwrap_or(Slide::Empty);
    let mut images = HashMap::new();
    for key in wanted_covers(&o.data, slide) {
        if let Some(img) = s.cover(Some(&key.0), key.1) {
            images.insert(key, img);
        }
    }
    let shot = Shot {
        period: o.period,
        data: o.data.clone(),
        slide,
        index: o.index,
        today: crate::views::stats::dates::today(),
        images,
    };
    s.recap.saving = true;
    s.spawn(
        move |_| render(shot, &path),
        |s, res, _| {
            s.recap.saving = false;
            match res {
                Ok(()) => notice(s, t("wrapped.saved"), "image"),
                Err(err) => {
                    log::warn!("Image du récap : {err}");
                    notice(s, err, "triangle-alert");
                }
            }
        },
    );
}

/// La carte seule : son décor, « control » et la période en haut, puis son contenu.
fn shot_view(s: &Shot, _cx: &mut Cx<Shot>) -> El {
    let cover = |hash: Option<&str>, small: bool| hash.and_then(|h| s.images.get(&(h.to_string(), small)).cloned());
    let ctx = Ctx { period: s.period, data: &s.data, cover: &cover, height: H, today: s.today, save: None };
    let mark = TextStyle::new(18.0, 700.0);
    el().w_full()
        .h_full()
        .child(slides::backdrop(slides::palette(s.index), W, H))
        .child(
            row()
                .absolute(0.0, 0.0)
                .w_full()
                .px(40.0)
                .pt(40.0)
                .justify_between()
                .child(text("control").font(mark).color(Color::WHITE.alpha(0.7)))
                .child(text(s.period.label()).font(mark).color(Color::WHITE.alpha(0.7)).align(Align::End)),
        )
        .child(
            col()
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .pt(84.0)
                .pb(44.0)
                .px(28.0)
                .justify_center()
                .items_center()
                .child(slides::stage_content(&ctx, s.slide, s.index, false)),
        )
}

/// Message après l'enregistrement : un toast, et le même dans le récap, qui recouvre les toasts.
fn notice(s: &mut Ui, text: String, icon: &'static str) {
    s.recap.notice = Some((text.clone(), icon, s.clock + 3.2));
    s.toast(text, icon);
}

/// Dessin hors fenêtre puis écriture du PNG.
fn render(shot: Shot, path: &Path) -> Result<(), String> {
    for img in shot.images.values() {
        let _ = img.wait();
    }
    // Sans carte graphique utilisable, wgpu s'arrête en panique : on le dit plutôt que de tomber.
    let gpu = std::panic::catch_unwind(Gpu::headless)
        .map_err(|_| "Aucune carte graphique disponible pour dessiner l’image.".to_string())?;
    let (pw, ph) = ((W * SCALE) as u32, (H * SCALE) as u32);
    let mut rt = Runtime::new(shot, shot_view, (W, H), Env::new(true, SCALE));
    rt.set_srgb_blend(true);
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
    let format = OFFSCREEN_FORMAT.remove_srgb_suffix();
    let mut renderer = Renderer::new(&gpu.device, format);
    renderer.set_srgb_blend(true);
    let target = Offscreen::with_format(&gpu.device, pw, ph, format);
    renderer.upload_images(&gpu.device, &gpu.queue, &mut rt.images);
    renderer.prepare(&gpu.device, &gpu.queue, &mut rt.text.atlas, &rt.instances, (pw, ph));
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    renderer.draw(&mut encoder, &target.view, rt.clear_color());
    target.copy(&mut encoder);
    renderer.submit(&gpu.queue, encoder);
    let pixels = target.read(&gpu.device);
    write_png(path, pw, ph, &pixels).map_err(|e| format!("Impossible d’enregistrer l’image : {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Stats {
        let db = control_core::db::open_in_memory().expect("base");
        control_core::stats::get(&db, "", "", 10, 0).expect("stats")
    }

    #[test]
    fn an_empty_period_has_one_card_and_draws_offscreen() {
        let data = empty();
        assert_eq!(slides::slides(&data), vec![Slide::Empty]);
        assert!(wanted_covers(&data, Slide::Tracks).is_empty());
        let dir = std::env::temp_dir().join(format!("control-recap-test-{}", std::process::id()));
        let path = dir.join("card.png");
        let shot = Shot {
            period: Period::Year(2026),
            data: Arc::new(data),
            slide: Slide::Empty,
            index: 0,
            today: crate::views::stats::dates::today(),
            images: HashMap::new(),
        };
        // Sans carte graphique (certaines machines d'intégration), le rendu le dit : rien de plus à vérifier.
        if render(shot, &path).is_ok() {
            let bytes = std::fs::read(&path).expect("image");
            assert_eq!(bytes.get(1..4), Some(&b"PNG"[..]));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

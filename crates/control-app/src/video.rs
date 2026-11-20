//! Images d'une vidéo : ffmpeg les décode en RGBA sur un thread, et l'interface affiche celle qui correspond à la
//! position du son (le moteur audio est l'horloge). Un saut relance le décodage à la nouvelle position.

use std::collections::VecDeque;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use crossbeam_channel::{Receiver, bounded};
use rune::Pixels;

use control_core::model::Track;

const FPS: f64 = 30.0;
const MAX_W: i64 = 1280;

struct Frame {
    t: f64,
    rgba: Vec<u8>,
}

struct Decoder {
    uid: u64,
    start: f64,
    child: Child,
    rx: Receiver<Frame>,
    queue: VecDeque<Frame>,
    last_t: f64,
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct VideoView {
    pixels: Pixels,
    size: Option<(u32, u32)>,
    decoder: Option<Decoder>,
    shown: bool,
}

impl Default for VideoView {
    fn default() -> Self {
        VideoView { pixels: Pixels::new(), size: None, decoder: None, shown: false }
    }
}

/// Taille de sortie : au plus 1280 px de large, dimensions paires (exigées par les filtres de ffmpeg).
fn out_size(track: &Track) -> (u32, u32) {
    let (w, h) = match (track.width, track.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w, h),
        _ => (1280, 720),
    };
    let ow = w.min(MAX_W);
    let oh = ((h as f64 * ow as f64 / w as f64 / 2.0).round() as i64 * 2).max(2);
    ((ow / 2 * 2) as u32, oh as u32)
}

impl VideoView {
    /// L'image en cours, une fois la première décodée.
    pub fn pixels(&self) -> Option<&Pixels> {
        self.shown.then_some(&self.pixels)
    }

    pub fn size(&self) -> Option<(u32, u32)> {
        self.size
    }

    /// Plus de vidéo à l'écran.
    pub fn stop(&mut self) {
        self.decoder = None;
        self.shown = false;
    }

    fn start(&mut self, ffmpeg: &PathBuf, uid: u64, track: &Track, at: f64) {
        self.decoder = None;
        let (w, h) = out_size(track);
        self.size = Some((w, h));
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-v", "error", "-ss", &format!("{:.3}", at.max(0.0)), "-i"])
            .arg(&track.path)
            .args(["-an", "-sn", "-vf", &format!("fps={FPS},scale={w}:{h}"), "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        control_core::process::hide_window(&mut cmd);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(err) => {
                log::warn!("Vidéo : ffmpeg ne démarre pas : {err}");
                return;
            }
        };
        let Some(mut out) = child.stdout.take() else { return };
        let (tx, rx) = bounded::<Frame>(6);
        let frame_len = (w * h * 4) as usize;
        let spawned = std::thread::Builder::new().name("control-video".into()).spawn(move || {
            let mut k = 0u64;
            loop {
                let mut buf = vec![0u8; frame_len];
                if out.read_exact(&mut buf).is_err() {
                    return;
                }
                let t = at.max(0.0) + k as f64 / FPS;
                k += 1;
                if tx.send(Frame { t, rgba: buf }).is_err() {
                    return;
                }
            }
        });
        if let Err(err) = spawned {
            log::warn!("Vidéo : thread impossible : {err}");
            let _ = child.kill();
            return;
        }
        self.decoder = Some(Decoder { uid, start: at, child, rx, queue: VecDeque::new(), last_t: at });
    }

    /// Montre l'image de `position` pour l'entrée `uid`. Renvoie `true` si la vue doit changer (première image).
    pub fn update(&mut self, ffmpeg: Option<&PathBuf>, uid: u64, track: &Track, position: f64) -> bool {
        let Some(ffmpeg) = ffmpeg else { return false };
        let restart = match &self.decoder {
            None => true,
            Some(d) => d.uid != uid || position < d.start - 0.25 || position > d.last_t + 2.0,
        };
        if restart {
            self.start(ffmpeg, uid, track, position);
        }
        let Some(d) = &mut self.decoder else { return false };
        while d.queue.len() < 4 {
            match d.rx.try_recv() {
                Ok(f) => {
                    d.last_t = f.t;
                    d.queue.push_back(f);
                }
                Err(_) => break,
            }
        }
        // La dernière image dont l'heure est passée ; les plus anciennes sont jetées.
        let mut pick = None;
        while d.queue.front().is_some_and(|f| f.t <= position + 0.5 / FPS) {
            pick = d.queue.pop_front();
        }
        let first = !self.shown;
        if let (Some(f), Some((w, h))) = (pick, self.size) {
            self.pixels.set(w, h, f.rgba);
            self.shown = true;
        }
        first && self.shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_size_is_even_and_bounded() {
        let mut t = Track { width: Some(1920), height: Some(1080), ..Track::default() };
        assert_eq!(out_size(&t), (1280, 720));
        t.width = Some(853);
        t.height = Some(481);
        let (w, h) = out_size(&t);
        assert!(w % 2 == 0 && h % 2 == 0 && w <= 1280);
        t.width = None;
        assert_eq!(out_size(&t), (1280, 720));
    }
}

//! Vignettes distantes (analyse, résultats, file) : téléchargées une à une sur un thread, décodées par un
//! `ImageCache` propre à la page. Une adresse qui échoue n'est pas redemandée ; la vue montre alors l'icône.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use rune::{Image, ImageCache};

use crate::state::Ui;

/// Au-delà, les vignettes gardées sont oubliées (elles seront redemandées au besoin).
const MAX_KEPT: usize = 300;
/// Une vignette plus grosse est refusée (ce n'en est pas une).
const MAX_BYTES: u64 = 8 * 1024 * 1024;

enum Slot {
    Pending,
    Ready(Image),
    Failed,
}

#[derive(Default)]
pub struct Thumbs {
    slots: RefCell<HashMap<String, Slot>>,
    cache: RefCell<Option<ImageCache>>,
    queue: Arc<Mutex<VecDeque<String>>>,
    running: Arc<AtomicBool>,
}

/// L'image d'une vignette si elle est arrivée ; sinon elle est demandée et `None` en attendant.
pub fn get(ui: &Ui, url: &str) -> Option<Image> {
    let th = &ui.downloads.thumbs;
    match th.slots.borrow().get(url) {
        Some(Slot::Ready(img)) if img.error().is_none() => return Some(img.clone()),
        Some(_) => return None,
        None => {}
    }
    request(ui, url);
    None
}

/// Demande une vignette à l'avance (dès que l'analyse arrive).
pub fn prefetch(ui: &Ui, url: Option<&str>) {
    if let Some(u) = url
        && !ui.downloads.thumbs.slots.borrow().contains_key(u)
    {
        request(ui, u);
    }
}

fn request(ui: &Ui, url: &str) {
    let th = &ui.downloads.thumbs;
    let ok = url.starts_with("https://") || url.starts_with("http://");
    {
        let mut slots = th.slots.borrow_mut();
        if slots.len() >= MAX_KEPT {
            slots.retain(|_, s| matches!(s, Slot::Pending));
        }
        slots.insert(url.to_string(), if ok { Slot::Pending } else { Slot::Failed });
    }
    if !ok {
        return;
    }
    th.queue.lock().push_back(url.to_string());
    if th.running.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(handle) = ui.handle.clone() else {
        th.running.store(false, Ordering::SeqCst);
        return;
    };
    let queue = th.queue.clone();
    let running = th.running.clone();
    let dir = ui.backend.data_dir.join("tmp");
    let spawned = std::thread::Builder::new().name("control-dl-thumbs".into()).spawn(move || {
        let mut n = 0u64;
        loop {
            let next = queue.lock().pop_front();
            let Some(url) = next else {
                running.store(false, Ordering::SeqCst);
                // Une demande arrivée entre-temps relance la boucle (sinon personne ne la servirait).
                if queue.lock().is_empty() || running.swap(true, Ordering::SeqCst) {
                    return;
                }
                continue;
            };
            n += 1;
            let bytes = fetch(&url, &dir.join(format!("thumb-{}-{n}", std::process::id())));
            handle.update(move |s: &mut Ui, _| loaded(s, url, bytes));
        }
    });
    if spawned.is_err() {
        th.running.store(false, Ordering::SeqCst);
    }
}

/// Télécharge une vignette (fichier temporaire relu puis effacé).
fn fetch(url: &str, tmp: &Path) -> Option<Vec<u8>> {
    let res = control_core::tools::download(url, tmp, |_, _| {});
    let bytes = res.ok().and_then(|()| {
        let small = std::fs::metadata(tmp).map(|m| m.len() <= MAX_BYTES).unwrap_or(false);
        if small { std::fs::read(tmp).ok() } else { None }
    });
    let _ = std::fs::remove_file(tmp);
    if bytes.is_none() {
        log::debug!("Vignette indisponible : {url}");
    }
    bytes
}

fn loaded(ui: &mut Ui, url: String, bytes: Option<Vec<u8>>) {
    let th = &ui.downloads.thumbs;
    let slot = match bytes {
        Some(b) if !b.is_empty() => {
            let mut cache = th.cache.borrow_mut();
            if cache.is_none() {
                *cache = ui.waker.clone().map(ImageCache::new);
            }
            match cache.as_ref() {
                Some(c) => Slot::Ready(c.load(b)),
                None => Slot::Failed,
            }
        }
        _ => Slot::Failed,
    };
    th.slots.borrow_mut().insert(url, slot);
}

//! Affiches, fonds et photos distants (TMDB, AniList) : les octets sont téléchargés (ureq, par `control-core`) sur
//! quelques fils de travail, puis décodés par un `rune::ImageCache` créé avec le réveil de l'application. Rien ne bloque le fil de
//! l'interface : une image absente se demande, la vue garde son dégradé de repli, et l'arrivée des octets relance
//! la vue. Le cache en mémoire est borné (les plus anciennes images sont oubliées).

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use parking_lot::Mutex;
use rune::{AppHandle, Image, ImageCache};

use crate::state::Ui;

/// Images gardées en mémoire (prêtes, en attente ou en échec).
pub const MAX_IMAGES: usize = 300;
/// Téléchargements simultanés.
const WORKERS: usize = 4;
/// Au-delà, l'image est refusée (une affiche fait quelques dizaines de Ko).
const MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone)]
enum Slot {
    Pending,
    Ready(Image),
    Failed,
}

/// Partagé avec les fils de téléchargement.
#[derive(Default)]
struct Shared {
    /// Adresses à télécharger, les plus récentes d'abord (ce que la vue vient de demander est à l'écran).
    todo: Mutex<VecDeque<String>>,
    /// Octets reçus (`None` : échec), en attente de livraison au fil de l'interface.
    done: Mutex<Vec<(String, Option<Vec<u8>>)>>,
    workers: AtomicUsize,
    /// Une livraison est déjà en route vers l'interface : les suivantes s'y regroupent.
    delivering: AtomicBool,
    /// Dossier des fichiers temporaires de téléchargement.
    dir: Mutex<PathBuf>,
}

/// Cache des images distantes de la partie « Regarder ».
#[derive(Default)]
pub struct RemoteImages {
    cache: RefCell<Option<ImageCache>>,
    slots: RefCell<HashMap<String, Slot>>,
    order: RefCell<VecDeque<String>>,
    shared: Arc<Shared>,
}

/// Télécharge une image (http ou https seulement). Le téléchargement passe par `control_core::tools::download`
/// (ureq) dans un fichier temporaire du dossier de données, relu puis effacé : l'application n'a pas de client HTTP
/// à elle.
fn fetch(url: &str, dir: &Path) -> Option<Vec<u8>> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return None;
    }
    let file = dir.join(format!("{}-{}.img", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    let res = control_core::tools::download(url, &file, |_, _| {});
    let bytes = res.ok().and_then(|()| std::fs::read(&file).ok());
    let _ = std::fs::remove_file(&file);
    bytes.filter(|b| !b.is_empty() && b.len() as u64 <= MAX_BYTES)
}

fn worker(shared: Arc<Shared>, handle: AppHandle<Ui>) {
    loop {
        while let Some(url) = shared.todo.lock().pop_front() {
            let dir = shared.dir.lock().clone();
            let bytes = fetch(&url, &dir);
            if bytes.is_none() {
                log::debug!("[watch] image non chargée : {url}");
            }
            shared.done.lock().push((url, bytes));
            if !shared.delivering.swap(true, Ordering::AcqRel) {
                handle.update(|s: &mut Ui, _| s.watch.images.deliver());
            }
        }
        shared.workers.fetch_sub(1, Ordering::AcqRel);
        // Une adresse arrivée entre la file vide et le décompte : on la prend si la place est libre.
        if shared.todo.lock().is_empty() {
            break;
        }
        if shared.workers.fetch_add(1, Ordering::AcqRel) >= WORKERS {
            shared.workers.fetch_sub(1, Ordering::AcqRel);
            break;
        }
    }
}

impl RemoteImages {
    /// L'image de `url` si elle est prête ; sinon elle est demandée (une fois) et `None` est rendu.
    pub fn get(&self, ui: &Ui, url: Option<&str>) -> Option<Image> {
        let url = url.filter(|u| !u.is_empty())?;
        if let Some(slot) = self.slots.borrow().get(url) {
            return match slot {
                Slot::Ready(img) => Some(img.clone()),
                _ => None,
            };
        }
        // Sans fenêtre (tests) ni réveil, aucune image ne peut arriver : on ne demande rien.
        let (Some(handle), Some(waker)) = (ui.handle.clone(), ui.waker.clone()) else { return None };
        if self.cache.borrow().is_none() {
            *self.cache.borrow_mut() = Some(ImageCache::new(waker));
            *self.shared.dir.lock() = ui.backend.data_dir.join("tmp").join("watch-images");
        }
        self.insert(url.to_string(), Slot::Pending);
        self.shared.todo.lock().push_front(url.to_string());
        if self.shared.workers.fetch_add(1, Ordering::AcqRel) < WORKERS {
            let shared = self.shared.clone();
            if std::thread::Builder::new().name("watch-images".into()).spawn(move || worker(shared, handle)).is_err() {
                self.shared.workers.fetch_sub(1, Ordering::AcqRel);
            }
        } else {
            self.shared.workers.fetch_sub(1, Ordering::AcqRel);
        }
        None
    }

    fn insert(&self, url: String, slot: Slot) {
        let mut slots = self.slots.borrow_mut();
        let mut order = self.order.borrow_mut();
        if slots.insert(url.clone(), slot).is_none() {
            order.push_back(url);
        }
        while order.len() > MAX_IMAGES {
            if let Some(old) = order.pop_front() {
                slots.remove(&old);
            }
        }
    }

    /// Octets arrivés (sur le fil de l'interface) : décodage confié au cache de Rune.
    pub fn deliver(&self) {
        self.shared.delivering.store(false, Ordering::Release);
        let done = std::mem::take(&mut *self.shared.done.lock());
        let cache = self.cache.borrow();
        for (url, bytes) in done {
            // Oubliée entre-temps (cache plein) : elle sera redemandée si elle s'affiche encore.
            if !matches!(self.slots.borrow().get(&url), Some(Slot::Pending)) {
                continue;
            }
            let slot = match (bytes, cache.as_ref()) {
                (Some(bytes), Some(cache)) => Slot::Ready(cache.load(bytes)),
                _ => Slot::Failed,
            };
            self.slots.borrow_mut().insert(url, slot);
        }
    }

    /// Nombre d'images connues (prêtes, en attente ou en échec).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.slots.borrow().len()
    }

    #[cfg(test)]
    pub fn mark_failed_for_test(&self, url: &str) {
        self.insert(url.to_string(), Slot::Failed);
    }
}

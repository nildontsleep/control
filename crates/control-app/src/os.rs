//! Intégration au système : ouvrir un lien, afficher un fichier dans son dossier.

use std::process::{Command, Stdio};

fn spawn(mut cmd: Command) {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    control_core::process::hide_window(&mut cmd);
    if let Err(err) = cmd.spawn() {
        log::warn!("Ouverture impossible : {err}");
    }
}

/// Ouvre un lien http(s) dans le navigateur (les autres schémas sont refusés).
pub fn open_url(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    open_path(url);
}

/// Ouvre un fichier ou un dossier avec l'application par défaut.
pub fn open_path(target: &str) {
    let cmd = if cfg!(target_os = "windows") {
        let mut c = Command::new("explorer");
        c.arg(target);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(target);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(target);
        c
    };
    spawn(cmd);
}

/// Montre un fichier dans le gestionnaire de fichiers (sélectionné quand le système le permet).
pub fn reveal(path: &str) {
    if cfg!(target_os = "windows") {
        let mut c = Command::new("explorer");
        c.arg(format!("/select,{path}"));
        spawn(c);
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.args(["-R", path]);
        spawn(c);
    } else {
        // Le portail FileManager1 sélectionne le fichier ; à défaut, on ouvre son dossier.
        let uri = format!("file://{}", path.replace('%', "%25").replace(' ', "%20"));
        let ok = Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("array:string:{uri}"),
                "string:",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok && let Some(dir) = std::path::Path::new(path).parent() {
            open_path(&dir.to_string_lossy());
        }
    }
}

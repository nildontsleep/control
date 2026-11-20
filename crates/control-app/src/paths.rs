//! Dossiers de l'application : les mêmes que la version Electron, pour reprendre ses données telles quelles.

use std::path::PathBuf;

/// Dossier des données : `control data` à côté de l'exécutable en édition portable (un fichier `portable` posé à
/// côté, ou la variable `CONTROL_PORTABLE`), sinon le dossier de configuration de l'utilisateur
/// (`~/.config/control`, `%APPDATA%\control`, `~/Library/Application Support/control`). `CONTROL_DATA` l'impose.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CONTROL_DATA").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = portable_dir() {
        return dir.join("control data");
    }
    if let Ok(dir) = std::env::var("PORTABLE_EXECUTABLE_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir).join("control data");
    }
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("control")
}

fn portable_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.to_path_buf();
    let flagged = std::env::var_os("CONTROL_PORTABLE").is_some() || dir.join("portable").is_file();
    flagged.then_some(dir)
}

/// Dossier « Musique » de l'utilisateur.
pub fn music_dir() -> Option<PathBuf> {
    dirs::audio_dir().or_else(|| dirs::home_dir().map(|h| h.join("Music")))
}

/// Dossier des téléchargements par défaut : `<Musique>/control`.
pub fn default_download_dir() -> PathBuf {
    music_dir().unwrap_or_else(|| PathBuf::from(".")).join("control")
}

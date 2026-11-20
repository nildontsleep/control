//! Lancement de programmes externes (ffmpeg, yt-dlp…).

use std::process::Command;

/// Sous Windows, pas de fenêtre de console pour un programme lancé en arrière-plan.
pub fn hide_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Nom d'un exécutable sur cette plateforme (`yt-dlp` → `yt-dlp.exe` sous Windows).
pub fn exe_name(base: &str) -> String {
    if cfg!(windows) { format!("{base}.exe") } else { base.to_string() }
}

/// Cherche un exécutable dans le `PATH`.
pub fn which(base: &str) -> Option<std::path::PathBuf> {
    let name = exe_name(base);
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(&name)).find(|p| p.is_file())
}

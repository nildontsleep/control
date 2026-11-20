//! Petits outils communs du serveur.

use std::path::{Path, PathBuf};

pub use control_core::core::RpcError as HttpError;
pub use control_core::util::now_ms;

/// Une erreur HTTP : statut, code court et message pour l'utilisateur (`httpError` de l'original).
pub fn http_error(status: u16, code: &str, message: impl Into<String>) -> HttpError {
    HttpError::new(status, code, message)
}

pub type HttpResult<T> = Result<T, HttpError>;

/// Écriture atomique d'un fichier lisible par le serveur seul (0600 sous Unix).
pub fn write_private(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension(format!("{}.tmp", std::process::id()));
    {
        use std::io::Write as _;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, file)
}

/// Chemin absolu, sans toucher au disque (`path.resolve`).
pub fn resolve(p: impl AsRef<Path>) -> PathBuf {
    let p = p.as_ref();
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    PathBuf::from(control_core::library::normalize(&abs))
}

/// Comparaison de chemins : sans tenir compte de la casse sous Windows.
pub fn path_cmp_key(p: &Path) -> String {
    let s = p.to_string_lossy().into_owned();
    if cfg!(windows) { s.to_lowercase() } else { s }
}

/// Le fichier est-il dans un des dossiers de cette liste ? (Garde-fou avant de servir un fichier.)
pub fn inside(roots: &[String], file: &str) -> bool {
    let target = path_cmp_key(&resolve(file));
    roots.iter().any(|r| {
        let root = path_cmp_key(&resolve(r));
        let sep = std::path::MAIN_SEPARATOR;
        target == root || target.starts_with(&format!("{}{sep}", root.trim_end_matches(sep)))
    })
}

/// Chemin relatif au dossier qui le contient (précédé du nom de ce dossier) : ce que voient les autres comptes.
pub fn relative(roots: &[String], file: &str) -> String {
    let resolved = resolve(file);
    for r in roots {
        let root = resolve(r);
        if let Ok(rest) = resolved.strip_prefix(&root) {
            let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            return Path::new(&name).join(rest).to_string_lossy().into_owned();
        }
    }
    resolved.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Nom d'hôte de l'adresse si elle est en https, sinon vide.
pub fn https_host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .filter(|u| u.scheme() == "https")
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_and_relative() {
        let roots = vec!["/srv/music".to_string()];
        assert!(inside(&roots, "/srv/music/a/b.mp3"));
        assert!(inside(&roots, "/srv/music"));
        assert!(!inside(&roots, "/srv/musicbis/a.mp3"));
        assert!(!inside(&roots, "/srv/music/../secret.mp3"));
        assert_eq!(
            relative(&roots, "/srv/music/A/b.mp3"),
            Path::new("music").join("A").join("b.mp3").to_string_lossy()
        );
        assert_eq!(relative(&roots, "/ailleurs/c.mp3"), "c.mp3");
    }
}

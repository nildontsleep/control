//! Commandes d'outillage du binaire (ce que faisaient `server/bundle.js`, `server/sign.js` et
//! `node server/backup.js decrypt`) :
//!
//! ```text
//! control-server --bundle [dossier]                 archive de mise à jour de ce binaire (pour la release)
//! control-server --sign keygen                      paire de clés (la privée va dans le secret WEB_SIGNING_KEY)
//! control-server --sign sign <archive>              écrit <archive>.sig (clé privée : WEB_SIGNING_KEY)
//! control-server --sign verify <archive> <clé>      contrôle une archive
//! control-server --backup decrypt <fichier> [sortie] déchiffre une sauvegarde (CONTROL_BACKUP_PASSPHRASE)
//! ```

use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

use crate::tar::{TarFile, pack};

/// Ce que l'archive contient : le binaire, `manifest.json`, et les fichiers de la page (`src/renderer`, `src/web`).
pub fn collect(binary: &Path, assets: &Path, version: &str) -> anyhow::Result<Vec<TarFile>> {
    let manifest =
        serde_json::json!({ "name": "control-server", "version": version, "target": crate::updater::target() });
    let mut files = vec![
        TarFile::new("manifest.json", serde_json::to_vec_pretty(&manifest)?),
        TarFile { name: crate::appstate::binary_name(), data: std::fs::read(binary)?, mode: 0o755 },
    ];
    fn walk(root: &Path, rel: &str, out: &mut Vec<TarFile>) -> std::io::Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(root.join(rel))?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let next = format!("{rel}/{name}");
            let kind = e.file_type()?;
            if kind.is_dir() {
                walk(root, &next, out)?;
            } else if kind.is_file() {
                out.push(TarFile::new(next, std::fs::read(e.path())?));
            }
        }
        Ok(())
    }
    walk(assets, "src/renderer", &mut files)?;
    walk(assets, "src/web", &mut files)?;
    Ok(files)
}

fn bundle(out: Option<&String>) -> anyhow::Result<()> {
    let out = PathBuf::from(out.map(String::as_str).unwrap_or("dist"));
    let version = env!("CARGO_PKG_VERSION");
    let data = pack(&collect(&std::env::current_exe()?, &crate::app::assets_root(), version)?)?;
    std::fs::create_dir_all(&out)?;
    let name = crate::updater::asset_name(version);
    std::fs::write(out.join(&name), &data)?;
    println!("{name} : {} Ko, sha256 {}", data.len() / 1024, hex::encode(Sha256::digest(&data)));
    Ok(())
}

/// La version d'une archive d'après son nom (`control-server-<version>-<plateforme>.tgz`).
fn version_of(file: &str) -> Option<String> {
    let name = Path::new(file).file_name()?.to_string_lossy().into_owned();
    let rest = name.strip_prefix("control-server-")?.strip_suffix(".tgz")?;
    let version = rest.split('-').next()?;
    crate::appstate::is_version(version).then(|| version.to_string())
}

fn sign(args: &[String]) -> anyhow::Result<i32> {
    match args.first().map(String::as_str) {
        Some("keygen") => {
            let (public, private) = crate::sign::keygen();
            println!("Clé publique (à mettre dans update-key.json, champ \"publicKeys\") :\n{public}\n");
            println!("Clé privée (à garder secrète : secret GitHub WEB_SIGNING_KEY) :\n{private}");
            Ok(0)
        }
        Some("sign") => {
            let file = args.get(1).ok_or_else(|| anyhow::anyhow!("Archive manquante."))?;
            let version = version_of(file)
                .ok_or_else(|| anyhow::anyhow!("Nom attendu : control-server-<version>-<plateforme>.tgz"))?;
            let key =
                std::env::var("WEB_SIGNING_KEY").map_err(|_| anyhow::anyhow!("Variable WEB_SIGNING_KEY absente."))?;
            let sig = crate::sign::sign(&version, &std::fs::read(file)?, key.trim())
                .ok_or_else(|| anyhow::anyhow!("Clé privée illisible."))?;
            std::fs::write(format!("{file}.sig"), format!("{sig}\n"))?;
            println!("{file}.sig écrit.");
            Ok(0)
        }
        Some("verify") => {
            let (Some(file), Some(key)) = (args.get(1), args.get(2)) else {
                anyhow::bail!("Utilisation : --sign verify <archive> <clé publique>");
            };
            let version = version_of(file).ok_or_else(|| anyhow::anyhow!("Nom d'archive inattendu."))?;
            let ok = crate::sign::verify(
                &version,
                &std::fs::read(file)?,
                &std::fs::read_to_string(format!("{file}.sig"))?,
                std::slice::from_ref(key),
            );
            println!("{}", if ok { "Signature valide." } else { "SIGNATURE INVALIDE." });
            Ok(if ok { 0 } else { 1 })
        }
        _ => {
            println!("Utilisation : control-server --sign keygen | sign <archive> | verify <archive> <clé publique>");
            Ok(1)
        }
    }
}

fn backup(args: &[String]) -> anyhow::Result<i32> {
    let (Some("decrypt"), Some(file)) = (args.first().map(String::as_str), args.get(1)) else {
        println!(
            "Utilisation : CONTROL_BACKUP_PASSPHRASE=… control-server --backup decrypt <sauvegarde.tgz.enc> [sortie.tgz]\nParties découpées : cat control-backup-*.part* > control-backup.tgz.enc"
        );
        return Ok(1);
    };
    let pass = std::env::var("CONTROL_BACKUP_PASSPHRASE")
        .map_err(|_| anyhow::anyhow!("Variable CONTROL_BACKUP_PASSPHRASE absente."))?;
    let target = args.get(2).cloned().unwrap_or_else(|| file.strip_suffix(".enc").unwrap_or(file).to_string());
    std::fs::write(&target, crate::backup::decrypt(&std::fs::read(file)?, &pass)?)?;
    println!("{target} écrit (archive .tgz).");
    Ok(0)
}

/// Traite une commande d'outillage ; `None` : ce n'en est pas une (lancer le serveur).
pub fn run(args: &[String]) -> Option<i32> {
    let res = match args.first().map(String::as_str)? {
        "--bundle" => bundle(args.get(1)).map(|()| 0),
        "--sign" => sign(&args[1..]),
        "--backup" => backup(&args[1..]),
        _ => return None,
    };
    Some(res.unwrap_or_else(|err| {
        eprintln!("{err:#}");
        1
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_contains_binary_manifest_and_pages() {
        let dir = tempfile::tempdir().unwrap();
        let assets = dir.path();
        std::fs::create_dir_all(assets.join("src/renderer/js")).unwrap();
        std::fs::create_dir_all(assets.join("src/web")).unwrap();
        std::fs::write(assets.join("src/renderer/index.html"), "<html>").unwrap();
        std::fs::write(assets.join("src/renderer/js/app.js"), "x").unwrap();
        std::fs::write(assets.join("src/web/.cache"), "caché").unwrap();
        std::fs::write(assets.join("src/web/login.html"), "<html>").unwrap();
        let bin = assets.join("bin");
        std::fs::write(&bin, "binaire").unwrap();
        let files = collect(&bin, assets, "1.2.3").unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "manifest.json",
                &crate::appstate::binary_name(),
                "src/renderer/index.html",
                "src/renderer/js/app.js",
                "src/web/login.html"
            ]
        );
        assert_eq!(files[1].mode, 0o755);
        assert_eq!(version_of("dist/control-server-1.2.3-linux-x86_64.tgz").as_deref(), Some("1.2.3"));
        assert_eq!(version_of("control-web-1.2.3.tgz"), None);
    }
}

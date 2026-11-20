//! Archives des outils (zip, tar.xz, tar.gz) : seuls les exécutables voulus en sont tirés, sans passer par
//! `tar` ou `unzip` du système (absents ou différents selon les machines).
//!
//! Les entrées sont reconnues à leur seul nom de fichier et écrites sous ce nom dans le dossier de destination :
//! un chemin malveillant dans l'archive (`../../x`) ne peut rien écrire ailleurs.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    TarXz,
    TarGz,
    Tar,
}

/// Type d'archive d'après le nom du fichier.
pub fn archive_kind(name: &str) -> Option<ArchiveKind> {
    let n = name.to_ascii_lowercase();
    if n.ends_with(".zip") {
        Some(ArchiveKind::Zip)
    } else if n.ends_with(".tar.xz") || n.ends_with(".txz") {
        Some(ArchiveKind::TarXz)
    } else if n.ends_with(".tar.gz") || n.ends_with(".tgz") {
        Some(ArchiveKind::TarGz)
    } else if n.ends_with(".tar") {
        Some(ArchiveKind::Tar)
    } else {
        None
    }
}

/// Écrit `reader` dans `dest` (fichier temporaire renommé), exécutable sous Unix.
fn write_executable(reader: &mut dyn Read, dest: &Path) -> std::io::Result<()> {
    let mut tmp = dest.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    let res = (|| {
        let mut f = File::create(&tmp)?;
        std::io::copy(reader, &mut f)?;
        f.sync_all()?;
        drop(f);
        make_executable(&tmp)?;
        std::fs::rename(&tmp, dest)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

/// Droits 755 sous Unix (rien à faire ailleurs).
pub fn make_executable(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Ce qu'on cherche dans l'archive et ce qui a été trouvé.
struct Wanted<'a> {
    names: &'a [String],
    dest: &'a Path,
    found: HashMap<String, PathBuf>,
}

impl Wanted<'_> {
    /// Nom voulu (et pas encore trouvé) correspondant à ce chemin d'archive.
    fn matches(&self, entry: &Path) -> Option<String> {
        let name = entry.file_name()?.to_str()?;
        self.names.iter().find(|w| w.as_str() == name && !self.found.contains_key(*w)).cloned()
    }

    fn take(&mut self, name: String, reader: &mut dyn Read) -> anyhow::Result<()> {
        let dest = self.dest.join(&name);
        write_executable(reader, &dest).with_context(|| format!("Écriture de {} impossible", dest.display()))?;
        self.found.insert(name, dest);
        Ok(())
    }

    fn done(&self) -> bool {
        self.found.len() == self.names.len()
    }
}

/// Tire de l'archive les fichiers nommés `names` (où qu'ils soient dans l'arborescence) et les écrit dans `dest`.
/// Renvoie leur chemin, dans l'ordre de `names` ; erreur si l'un d'eux manque.
pub fn extract_wanted(
    archive: &Path,
    kind: ArchiveKind,
    names: &[String],
    dest: &Path,
) -> anyhow::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dest)?;
    let mut wanted = Wanted { names, dest, found: HashMap::new() };
    let file = File::open(archive).with_context(|| format!("{} introuvable", archive.display()))?;
    match kind {
        ArchiveKind::Zip => extract_zip(file, &mut wanted)?,
        ArchiveKind::Tar => extract_tar(BufReader::new(file), &mut wanted)?,
        ArchiveKind::TarGz => extract_tar(flate2::read::GzDecoder::new(BufReader::new(file)), &mut wanted)?,
        ArchiveKind::TarXz => extract_tar_xz(file, &mut wanted)?,
    }
    names
        .iter()
        .map(|n| wanted.found.get(n).cloned().ok_or_else(|| anyhow!("{n} introuvable dans l’archive")))
        .collect()
}

fn extract_zip(file: File, wanted: &mut Wanted<'_>) -> anyhow::Result<()> {
    let mut zip = zip::ZipArchive::new(BufReader::new(file)).context("Archive zip illisible")?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).context("Archive zip illisible")?;
        if entry.is_dir() {
            continue;
        }
        let Some(path) = entry.enclosed_name() else { continue };
        if let Some(name) = wanted.matches(&path) {
            wanted.take(name, &mut entry)?;
            if wanted.done() {
                break;
            }
        }
    }
    Ok(())
}

fn extract_tar(reader: impl Read, wanted: &mut Wanted<'_>) -> anyhow::Result<()> {
    let mut tar = tar::Archive::new(reader);
    for entry in tar.entries().context("Archive tar illisible")? {
        let mut entry = entry.context("Archive tar illisible")?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let Ok(path) = entry.path().map(|p| p.into_owned()) else { continue };
        if let Some(name) = wanted.matches(&path) {
            wanted.take(name, &mut entry)?;
            if wanted.done() {
                break;
            }
        }
    }
    Ok(())
}

/// tar.xz : lzma-rs décompresse vers un `Write` ; un tube relie la décompression (sur un thread) à la lecture du
/// tar, sans écrire l'archive décompressée sur le disque.
fn extract_tar_xz(file: File, wanted: &mut Wanted<'_>) -> anyhow::Result<()> {
    let (reader, mut writer) = std::io::pipe()?;
    let decoder = std::thread::spawn(move || {
        let mut input = BufReader::new(file);
        lzma_rs::xz_decompress(&mut input, &mut writer).map_err(|e| e.to_string())
    });
    let mut reader = reader;
    let res = extract_tar(&mut reader, wanted);
    if res.is_ok() && !wanted.done() {
        // Fin du tar atteinte sans tout trouver : laisser la décompression finir pour distinguer une archive abîmée
        // d'un fichier absent.
        let _ = std::io::copy(&mut reader, &mut std::io::sink());
    }
    drop(reader);
    // Lecteur fermé : si tout a été trouvé avant la fin, la décompression s'arrête sur un tube rompu (sans gravité).
    let decoded = decoder.join().map_err(|_| anyhow!("Décompression interrompue"))?;
    res?;
    if let Err(err) = decoded
        && !wanted.done()
    {
        return Err(anyhow!("Archive xz illisible : {err}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const FILES: &[(&str, &[u8])] =
        &[("pkg/doc/readme.txt", b"doc"), ("pkg/bin/ffmpeg", b"FFMPEG"), ("pkg/bin/ffprobe", b"FFPROBE")];

    fn tar_bytes() -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (name, data) in FILES {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, name, *data).unwrap();
        }
        b.into_inner().unwrap()
    }

    fn names() -> Vec<String> {
        vec!["ffmpeg".into(), "ffprobe".into()]
    }

    fn check(dir: &Path, got: Vec<PathBuf>) {
        assert_eq!(got, vec![dir.join("ffmpeg"), dir.join("ffprobe")]);
        assert_eq!(std::fs::read(dir.join("ffmpeg")).unwrap(), b"FFMPEG");
        assert_eq!(std::fs::read(dir.join("ffprobe")).unwrap(), b"FFPROBE");
        assert!(!dir.join("readme.txt").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.join("ffmpeg")).unwrap().permissions().mode() & 0o777, 0o755);
        }
    }

    #[test]
    fn kinds() {
        assert_eq!(archive_kind("ffmpeg-master-latest-win64-gpl.zip"), Some(ArchiveKind::Zip));
        assert_eq!(archive_kind("ffmpeg-master-latest-linux64-gpl.tar.xz"), Some(ArchiveKind::TarXz));
        assert_eq!(archive_kind("x.TGZ"), Some(ArchiveKind::TarGz));
        assert_eq!(archive_kind("yt-dlp_linux"), None);
    }

    #[test]
    fn zip_extraction() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        {
            let mut z = zip::ZipWriter::new(File::create(&archive).unwrap());
            let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            z.add_directory("pkg/", opts).unwrap();
            // Un chemin qui sort de l'archive est ignoré.
            z.start_file("../../evil/ffmpeg", opts).unwrap();
            z.write_all(b"EVIL").unwrap();
            for (name, data) in FILES {
                z.start_file(*name, opts).unwrap();
                z.write_all(data).unwrap();
            }
            z.finish().unwrap();
        }
        let out = dir.path().join("bin");
        check(&out, extract_wanted(&archive, ArchiveKind::Zip, &names(), &out).unwrap());
    }

    #[test]
    fn tar_xz_and_gz_extraction() {
        let dir = tempfile::tempdir().unwrap();
        let tar = tar_bytes();

        let xz = dir.path().join("a.tar.xz");
        let mut packed = Vec::new();
        lzma_rs::xz_compress(&mut &tar[..], &mut packed).unwrap();
        std::fs::write(&xz, packed).unwrap();
        let out = dir.path().join("xz");
        check(&out, extract_wanted(&xz, ArchiveKind::TarXz, &names(), &out).unwrap());

        let gz = dir.path().join("a.tar.gz");
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(&tar).unwrap();
        std::fs::write(&gz, enc.finish().unwrap()).unwrap();
        let out = dir.path().join("gz");
        check(&out, extract_wanted(&gz, ArchiveKind::TarGz, &names(), &out).unwrap());
    }

    #[test]
    fn missing_entry_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path().join("a.tar");
        std::fs::write(&t, tar_bytes()).unwrap();
        let err = extract_wanted(&t, ArchiveKind::Tar, &["deno".to_string()], &dir.path().join("o")).unwrap_err();
        assert_eq!(err.to_string(), "deno introuvable dans l’archive");
        let bad = dir.path().join("b.tar.xz");
        std::fs::write(&bad, b"pas une archive").unwrap();
        assert!(extract_wanted(&bad, ArchiveKind::TarXz, &names(), &dir.path().join("p")).is_err());
    }
}

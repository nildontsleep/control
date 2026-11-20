//! Archives .tgz du serveur : écriture (archive publiée avec chaque release, sauvegardes) et lecture stricte (archive
//! que le serveur télécharge). Seuls les fichiers et les dossiers sont acceptés : ni lien, ni chemin qui sortirait du
//! dossier d'extraction, ni archive démesurée. Format ustar, comme `server/tar.js`.

use std::collections::HashSet;
use std::io::Read as _;

const BLOCK: usize = 512;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_files: 5000, max_file_bytes: 64 * 1024 * 1024, max_total_bytes: 200 * 1024 * 1024 }
    }
}

/// Un fichier d'archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TarFile {
    pub name: String,
    pub data: Vec<u8>,
    pub mode: u32,
}

impl TarFile {
    pub fn new(name: impl Into<String>, data: impl Into<Vec<u8>>) -> TarFile {
        TarFile { name: name.into(), data: data.into(), mode: 0o644 }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("archive : {0}")]
pub struct TarError(pub String);

fn err(msg: impl Into<String>) -> TarError {
    TarError(msg.into())
}

fn field(buf: &[u8], start: usize, len: usize) -> String {
    let slice = &buf[start..start + len];
    let end = slice.iter().position(|&b| b == 0).unwrap_or(len);
    String::from_utf8_lossy(&slice[..end]).into_owned()
}

fn octal(buf: &[u8], start: usize, len: usize) -> Result<u64, TarError> {
    if buf[start] & 0x80 != 0 {
        return Err(err("taille non prise en charge"));
    }
    let text = field(buf, start, len);
    let text = text.trim();
    if !text.is_empty() && !text.chars().all(|c| ('0'..='7').contains(&c)) {
        return Err(err("en-tête illisible"));
    }
    Ok(if text.is_empty() { 0 } else { u64::from_str_radix(text, 8).map_err(|_| err("en-tête illisible"))? })
}

/// Le chemin d'une entrée : relatif, sans « .. », sans séparateur Windows ni caractère de contrôle.
pub fn safe_name(raw: &str) -> Result<String, TarError> {
    let mut name = raw;
    while let Some(rest) = name.strip_prefix("./") {
        name = rest;
    }
    let name = name.trim_end_matches('/');
    let refused = || err(format!("chemin refusé ({raw:?})"));
    let drive = name.len() >= 2 && name.as_bytes()[0].is_ascii_alphabetic() && name.as_bytes()[1] == b':';
    if name.is_empty() || name.starts_with('/') || name.chars().any(|c| c == '\\' || (c as u32) < 0x20) || drive {
        return Err(refused());
    }
    if name.split('/').any(|p| p.is_empty() || p == "." || p == "..") {
        return Err(refused());
    }
    Ok(name.to_string())
}

/// Lit une archive .tgz (en mémoire). Lève une erreur au moindre doute.
pub fn unpack(gz: &[u8], limits: Limits) -> Result<Vec<TarFile>, TarError> {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(gz)
        .take(limits.max_total_bytes + 64 * BLOCK as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| err(format!("illisible ({e})")))?;
    if raw.len() as u64 > limits.max_total_bytes + 64 * BLOCK as u64 {
        return Err(err("trop volumineuse"));
    }
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut total = 0u64;
    let mut at = 0usize;
    while at + BLOCK <= raw.len() {
        let head = &raw[at..at + BLOCK];
        if head.iter().all(|&b| b == 0) {
            break;
        }
        let sum: u64 = head.iter().enumerate().map(|(i, &b)| if (148..156).contains(&i) { 32 } else { b as u64 }).sum();
        if sum != octal(head, 148, 8)? {
            return Err(err("en-tête corrompu"));
        }
        let prefix = field(head, 345, 155);
        let base = field(head, 0, 100);
        let name = safe_name(&if prefix.is_empty() { base } else { format!("{prefix}/{base}") })?;
        let size = octal(head, 124, 12)?;
        let kind = if head[156] == 0 { b'0' } else { head[156] };
        at += BLOCK;
        if kind == b'5' {
            continue;
        }
        if kind != b'0' {
            return Err(err(format!("entrée non prise en charge ({name:?}, type {:?})", (kind as char).to_string())));
        }
        total += size;
        if size > limits.max_file_bytes || total > limits.max_total_bytes || files.len() >= limits.max_files {
            return Err(err("trop volumineuse"));
        }
        if at as u64 + size > raw.len() as u64 {
            return Err(err("tronquée"));
        }
        if !seen.insert(name.clone()) {
            return Err(err(format!("fichier en double ({name:?})")));
        }
        let size = size as usize;
        files.push(TarFile { name, data: raw[at..at + size].to_vec(), mode: (octal(head, 100, 8)? & 0o777) as u32 });
        at += size.div_ceil(BLOCK) * BLOCK;
    }
    // Un fichier ne peut pas être aussi un dossier (« a » et « a/b »).
    for name in &seen {
        for (i, _) in name.match_indices('/') {
            if seen.contains(&name[..i]) {
                return Err(err(format!("chemin incohérent ({name:?})")));
            }
        }
    }
    Ok(files)
}

fn put(buf: &mut [u8], text: &str, start: usize, len: usize) {
    let bytes = text.as_bytes();
    let n = bytes.len().min(len);
    buf[start..start + n].copy_from_slice(&bytes[..n]);
}

fn put_octal(buf: &mut [u8], value: u64, start: usize, len: usize) {
    put(buf, &format!("{value:0width$o}\0", width = len - 1), start, len);
}

/// Écrit l'archive tar (non compressée).
pub fn pack_raw(files: &[TarFile]) -> Result<Vec<u8>, TarError> {
    let mut out = Vec::new();
    for f in files {
        let path = safe_name(&f.name)?;
        let (mut base, mut prefix) = (path.clone(), String::new());
        if path.len() > 100 {
            // Coupure au dernier « / » qui laisse un nom de 100 octets au plus et un préfixe de 155 au plus.
            let cut = path.get(..path.len().min(256)).and_then(|s| s.rfind('/')).filter(|&c| c > 0);
            if let Some(c) = cut {
                base = path[c + 1..].to_string();
                prefix = path[..c].to_string();
            }
            if prefix.is_empty() || prefix.len() > 155 || base.len() > 100 {
                return Err(err(format!("chemin trop long ({path})")));
            }
        }
        let mut head = [0u8; BLOCK];
        put(&mut head, &base, 0, 100);
        put_octal(&mut head, (f.mode & 0o777) as u64, 100, 8);
        put_octal(&mut head, 0, 108, 8);
        put_octal(&mut head, 0, 116, 8);
        put_octal(&mut head, f.data.len() as u64, 124, 12);
        put_octal(&mut head, 0, 136, 12);
        head[148..156].fill(32);
        head[156] = b'0';
        put(&mut head, "ustar\0", 257, 6);
        put(&mut head, "00", 263, 2);
        put(&mut head, &prefix, 345, 155);
        let sum: u64 = head.iter().map(|&b| b as u64).sum();
        put(&mut head, &format!("{sum:06o}\0 "), 148, 8);
        out.extend_from_slice(&head);
        out.extend_from_slice(&f.data);
        out.resize(out.len() + (BLOCK - f.data.len() % BLOCK) % BLOCK, 0);
    }
    out.resize(out.len() + 2 * BLOCK, 0);
    Ok(out)
}

/// Écrit une archive .tgz déterministe (dates à zéro).
pub fn pack(files: &[TarFile]) -> Result<Vec<u8>, TarError> {
    use std::io::Write as _;
    let raw = pack_raw(files)?;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(&raw).map_err(|e| err(e.to_string()))?;
    enc.finish().map_err(|e| err(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gunzip(gz: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(gz).read_to_end(&mut out).unwrap();
        out
    }

    fn gzip(raw: &[u8]) -> Vec<u8> {
        use std::io::Write as _;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(raw).unwrap();
        e.finish().unwrap()
    }

    /// Modifie l'en-tête de la première entrée (puis recalcule la somme de contrôle).
    fn tamper(gz: &[u8], mutate: impl FnOnce(&mut [u8])) -> Vec<u8> {
        let mut raw = gunzip(gz);
        let head = &mut raw[..512];
        mutate(head);
        head[148..156].fill(32);
        let sum: u64 = head.iter().map(|&b| b as u64).sum();
        put(head, &format!("{sum:06o}\0 "), 148, 8);
        gzip(&raw)
    }

    #[test]
    fn roundtrip_keeps_modes_and_long_paths() {
        let long = format!("server/{}/{}.js", "a".repeat(60), "b".repeat(80));
        let mut run = TarFile::new("server/run.sh", "x".repeat(1500));
        run.mode = 0o755;
        let files = unpack(
            &pack(&[TarFile::new("package.json", "{}"), run, TarFile::new(long.clone(), "y")]).unwrap(),
            Limits::default(),
        )
        .unwrap();
        assert_eq!(
            files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            ["package.json", "server/run.sh", long.as_str()]
        );
        assert_eq!(files[1].data.len(), 1500);
        assert_eq!(files[1].mode, 0o755);
    }

    #[test]
    fn refuses_links_escapes_duplicates_and_damage() {
        let good = pack(&[TarFile::new("a.txt", "x")]).unwrap();
        let msg = |r: Result<Vec<TarFile>, TarError>| r.unwrap_err().to_string();
        assert!(msg(unpack(&tamper(&good, |h| h[156] = b'2'), Limits::default())).contains("non prise en charge"));
        assert!(
            msg(unpack(&tamper(&good, |h| put(h, "../evil\0", 0, 8)), Limits::default())).contains("chemin refusé")
        );
        assert!(
            msg(unpack(&tamper(&good, |h| put(h, "/etc/passwd\0", 0, 12)), Limits::default()))
                .contains("chemin refusé")
        );
        assert!(msg(unpack(&tamper(&good, |h| put(h, "a\\b\0", 0, 4)), Limits::default())).contains("chemin refusé"));
        let nested = pack(&[TarFile::new("a", "1"), TarFile::new("a/b", "2")]).unwrap();
        assert!(msg(unpack(&nested, Limits::default())).contains("incohérent"));
        let one = gunzip(&pack(&[TarFile::new("a", "1")]).unwrap());
        let two = gunzip(&pack(&[TarFile::new("a", "2")]).unwrap());
        let dup = gzip(&[&one[..1024], &two[..]].concat());
        assert!(msg(unpack(&dup, Limits::default())).contains("double"));
        let mut raw = gunzip(&good);
        raw[3] ^= 0xff;
        assert!(msg(unpack(&gzip(&raw), Limits::default())).contains("corrompu"));
        assert!(msg(unpack(&good, Limits { max_file_bytes: 0, ..Limits::default() })).contains("volumineuse"));
        assert!(unpack(b"pas une archive", Limits::default()).is_err());
        assert!(pack(&[TarFile::new("../x", "")]).unwrap_err().to_string().contains("chemin refusé"));
    }
}

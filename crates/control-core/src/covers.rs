//! Pochettes : identifiées par l'empreinte de leur contenu, enregistrées en deux tailles (`<hash>.jpg`, 640 px au
//! plus, et `<hash>-s.jpg`, 160 px au plus). Plusieurs comptes peuvent partager le même dossier.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageReader};
use sha1::{Digest, Sha1};

/// Taille maximale d'une image acceptée (12 Mo), comme la version d'origine.
pub const MAX_IMAGE_BYTES: usize = 12 * 1024 * 1024;

/// Empreinte courte d'un contenu (20 caractères hexadécimaux de SHA-1).
pub fn hash_of(data: &[u8]) -> String {
    let digest = Sha1::digest(data);
    hex::encode(digest)[..20].to_string()
}

/// Les octets ressemblent-ils à une image (JPEG, PNG, WebP, GIF, BMP) ?
pub fn is_image(data: &[u8]) -> bool {
    data.len() <= MAX_IMAGE_BYTES
        && (data.starts_with(&[0xFF, 0xD8, 0xFF])
            || data.starts_with(b"\x89PNG\r\n\x1a\n")
            || (data.len() > 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP")
            || data.starts_with(b"GIF8")
            || data.starts_with(b"BM"))
}

#[derive(Debug, Clone)]
pub struct Covers {
    dir: PathBuf,
}

impl Covers {
    pub fn new(dir: impl Into<PathBuf>) -> Covers {
        let dir = dir.into();
        let _ = std::fs::create_dir_all(&dir);
        Covers { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Chemin de la grande image d'une empreinte (ou de la petite).
    pub fn path(&self, hash: &str, small: bool) -> Option<PathBuf> {
        if hash.is_empty() || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        Some(self.dir.join(if small { format!("{hash}-s.jpg") } else { format!("{hash}.jpg") }))
    }

    /// Enregistre une image et renvoie son empreinte (rien n'est réécrit si elle existe déjà).
    pub fn save(&self, data: &[u8]) -> std::io::Result<String> {
        let hash = hash_of(data);
        let big = self.dir.join(format!("{hash}.jpg"));
        let small = self.dir.join(format!("{hash}-s.jpg"));
        if big.exists() && small.exists() {
            return Ok(hash);
        }
        std::fs::create_dir_all(&self.dir)?;
        match decode(data) {
            Some(img) => {
                crate::util::write_atomic(&big, &encode_jpeg(&fit(&img, 640), 88))?;
                crate::util::write_atomic(&small, &encode_jpeg(&fit(&img, 160), 82))?;
            }
            None => {
                // Format que l'on ne sait pas décoder : on garde l'original, l'affichage s'en débrouillera.
                crate::util::write_atomic(&big, data)?;
                crate::util::write_atomic(&small, data)?;
            }
        }
        Ok(hash)
    }
}

fn decode(data: &[u8]) -> Option<DynamicImage> {
    ImageReader::new(Cursor::new(data)).with_guessed_format().ok()?.decode().ok()
}

/// Réduit une image pour que son plus grand côté fasse au plus `max` pixels (jamais d'agrandissement).
pub fn fit(img: &DynamicImage, max: u32) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    if w.max(h) <= max {
        return img.clone();
    }
    let k = max as f64 / w.max(h) as f64;
    let nw = ((w as f64 * k).round() as u32).max(1);
    let nh = ((h as f64 * k).round() as u32).max(1);
    img.resize_exact(nw, nh, image::imageops::FilterType::Lanczos3)
}

/// JPEG d'une image (sans transparence : posée sur du noir).
pub fn encode_jpeg(img: &DynamicImage, quality: u8) -> Vec<u8> {
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    let mut enc = JpegEncoder::new_with_quality(&mut out, quality);
    if enc.encode_image(&rgb).is_err() {
        out.clear();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = DynamicImage::new_rgb8(w, h);
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn saves_two_sizes() {
        let dir = tempfile::tempdir().unwrap();
        let covers = Covers::new(dir.path());
        let data = png(1000, 500);
        assert!(is_image(&data));
        let hash = covers.save(&data).unwrap();
        assert_eq!(hash.len(), 20);
        let big = image::open(covers.path(&hash, false).unwrap()).unwrap();
        let small = image::open(covers.path(&hash, true).unwrap()).unwrap();
        assert_eq!((big.width(), big.height()), (640, 320));
        assert_eq!((small.width(), small.height()), (160, 80));
        assert_eq!(covers.save(&data).unwrap(), hash);
    }

    #[test]
    fn rejects_bad_hashes() {
        let covers = Covers::new(std::env::temp_dir());
        assert!(covers.path("../etc", false).is_none());
        assert!(covers.path("", true).is_none());
    }
}

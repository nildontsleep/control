//! Secrets que le serveur doit pouvoir relire (le mot de passe d'application Subsonic : la signature « token » de
//! l'API a besoin du secret en clair). Chiffrés (AES-256-GCM) avec une clé aléatoire gardée à part dans le dossier
//! de données (`secret.key`, lisible par le serveur seul) : la base seule ne les révèle pas. Format de
//! `server/secrets.js` : base64(iv 12 | étiquette 16 | données).

use std::path::PathBuf;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use parking_lot::Mutex;

pub struct Secrets {
    file: PathBuf,
    key: Mutex<Option<[u8; 32]>>,
}

impl Secrets {
    pub fn new(data_dir: &std::path::Path) -> Secrets {
        Secrets { file: data_dir.join("secret.key"), key: Mutex::new(None) }
    }

    fn load(&self) -> [u8; 32] {
        let mut slot = self.key.lock();
        if let Some(k) = *slot {
            return k;
        }
        if let Ok(text) = std::fs::read_to_string(&self.file)
            && let Ok(raw) = B64.decode(text.trim())
            && let Ok(k) = <[u8; 32]>::try_from(raw.as_slice())
        {
            *slot = Some(k);
            return k;
        }
        let k: [u8; 32] = rand::random();
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(err) = crate::util::write_private(&self.file, format!("{}\n", B64.encode(k)).as_bytes()) {
            log::error!("Clé des secrets impossible à écrire : {err}");
        }
        *slot = Some(k);
        k
    }

    pub fn seal(&self, text: &str) -> String {
        let key = self.load();
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let iv: [u8; 12] = rand::random();
        // aes-gcm rend données | étiquette ; l'original range iv | étiquette | données.
        let Ok(out) = cipher.encrypt(Nonce::from_slice(&iv), text.as_bytes()) else { return String::new() };
        let (body, tag) = out.split_at(out.len() - 16);
        let mut blob = Vec::with_capacity(28 + body.len());
        blob.extend_from_slice(&iv);
        blob.extend_from_slice(tag);
        blob.extend_from_slice(body);
        B64.encode(blob)
    }

    pub fn open(&self, sealed: &str) -> Option<String> {
        let blob = B64.decode(sealed.trim()).ok()?;
        if blob.len() < 28 {
            return None;
        }
        let key = self.load();
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let mut joined = blob[28..].to_vec();
        joined.extend_from_slice(&blob[12..28]);
        let plain = cipher.decrypt(Nonce::from_slice(&blob[..12]), joined.as_slice()).ok()?;
        String::from_utf8(plain).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_and_open() {
        let dir = tempfile::tempdir().unwrap();
        let s = Secrets::new(dir.path());
        let sealed = s.seal("mot de passe");
        assert_ne!(sealed, "mot de passe");
        assert_eq!(s.open(&sealed).as_deref(), Some("mot de passe"));
        // Même clé relue depuis le disque.
        assert_eq!(Secrets::new(dir.path()).open(&sealed).as_deref(), Some("mot de passe"));
        assert_eq!(s.open("abîmé"), None);
        let mut blob = B64.decode(&sealed).unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 1;
        assert_eq!(s.open(&B64.encode(blob)), None);
        // Secret scellé par server/secrets.js : relu tel quel.
        let node = tempfile::tempdir().unwrap();
        std::fs::write(node.path().join("secret.key"), "/vxancxe0tP5U4V58lhs9Q5v/LciamooBkFQIBOZkYc=\n").unwrap();
        let sealed_by_node = "69ap0++PCL/n9b/R33vdPr7J0Os6u6gWhSZGi0aPygz7mw+Ityk=";
        assert_eq!(Secrets::new(node.path()).open(sealed_by_node).as_deref(), Some("secret-app"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("secret.key")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}

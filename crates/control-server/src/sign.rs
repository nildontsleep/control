//! Signature des archives de mise à jour (Ed25519). L'empreinte SHA-256 publiée par GitHub protège d'un fichier
//! abîmé, pas d'un dépôt compromis : la signature, faite avec une clé qui ne quitte jamais les secrets de la CI,
//! l'est. Les serveurs ne font confiance qu'aux archives signées par une clé de `update-key.json`.
//!
//! Format de `server/sign.js` : clé publique = 32 octets bruts en base64, clé privée = PKCS#8 DER en base64,
//! signature = 64 octets en base64, sur « control-web <version>\n » suivi de l'archive.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};

/// En-tête DER PKCS#8 d'une clé privée Ed25519 (suivi des 32 octets de la graine).
const PKCS8_PREFIX: [u8; 16] =
    [0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20];

/// Ce que la signature couvre : le nom de la version, puis l'archive.
fn message(version: &str, archive: &[u8]) -> Vec<u8> {
    let mut m = format!("control-web {version}\n").into_bytes();
    m.extend_from_slice(archive);
    m
}

/// Une paire de clés : (publique, privée), en base64.
pub fn keygen() -> (String, String) {
    let key = SigningKey::from_bytes(&rand::random::<[u8; 32]>());
    let mut der = PKCS8_PREFIX.to_vec();
    der.extend_from_slice(&key.to_bytes());
    (B64.encode(key.verifying_key().to_bytes()), B64.encode(der))
}

pub fn sign(version: &str, archive: &[u8], private_key: &str) -> Option<String> {
    let der = B64.decode(private_key.trim()).ok()?;
    let seed: [u8; 32] = der.strip_prefix(&PKCS8_PREFIX[..])?.try_into().ok()?;
    Some(B64.encode(SigningKey::from_bytes(&seed).sign(&message(version, archive)).to_bytes()))
}

/// Vrai si l'une des clés publiques a signé exactement cette archive de cette version.
pub fn verify(version: &str, archive: &[u8], signature: &str, keys: &[String]) -> bool {
    let Ok(sig) = B64.decode(signature.trim()) else { return false };
    let Ok(sig) = <[u8; 64]>::try_from(sig.as_slice()) else { return false };
    let sig = Signature::from_bytes(&sig);
    let msg = message(version, archive);
    keys.iter().any(|k| {
        B64.decode(k.trim())
            .ok()
            .and_then(|raw| <[u8; 32]>::try_from(raw.as_slice()).ok())
            .and_then(|raw| VerifyingKey::from_bytes(&raw).ok())
            .is_some_and(|key| key.verify(&msg, &sig).is_ok())
    })
}

/// Clés publiques de confiance (`update-key.json`, intégré au binaire). Vide : les mises à jour ne sont pas signées.
pub fn trusted_keys() -> Vec<String> {
    parse_keys(include_str!("../update-key.json"))
}

pub fn parse_keys(json: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v["publicKeys"].as_array().cloned())
        .map(|a| {
            a.into_iter()
                .filter_map(|k| k.as_str().map(str::trim).filter(|k| !k.is_empty()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify() {
        let (public, private) = keygen();
        let (stranger, _) = keygen();
        let sig = sign("1.2.3", b"archive", &private).unwrap();
        assert!(verify("1.2.3", b"archive", &sig, std::slice::from_ref(&public)));
        assert!(!verify("1.2.4", b"archive", &sig, std::slice::from_ref(&public)), "autre version");
        assert!(!verify("1.2.3", b"archivE", &sig, std::slice::from_ref(&public)), "archive modifiée");
        assert!(!verify("1.2.3", b"archive", &sig, &[stranger]));
        assert!(!verify("1.2.3", b"archive", "AAAA", &[public]));
        assert_eq!(trusted_keys(), Vec::<String>::new());

        // Clés et signature produites par server/sign.js : mêmes formats, même signature (Ed25519 est déterministe).
        let node_public = "HO1EJ10dCAaDtRazaFCjIBw7aAY0k3OyX7qgT7Xbmuw=".to_string();
        let node_private = "MC4CAQAwBQYDK2VwBCIEIGK1I20+/xHZg+ypitpyi3qcgms82hcHSDApktKOx+Bh";
        let node_sig = "JhEfRPGiHuVnVwIprGV5FF9bIuMdUEymq3amqc9m8sUJKzPnmJQvy/ODmD8cvIE1k2O1NC1wV7uSmL53lKdkBA==";
        assert!(verify("1.2.3", b"archive", node_sig, std::slice::from_ref(&node_public)));
        assert_eq!(sign("1.2.3", b"archive", node_private).as_deref(), Some(node_sig));
        assert_eq!(parse_keys(r#"{ "publicKeys": [" abc ", "", 3] }"#), ["abc"]);
    }
}

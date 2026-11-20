//! Liens que le serveur accepte de faire télécharger par yt-dlp : http(s) uniquement, et jamais une adresse du réseau
//! interne (le serveur ne doit pas servir de relais vers 127.0.0.1, le réseau local ou les services de métadonnées
//! d'un hébergeur). C'est une première barrière : une redirection ou un changement de DNS après la vérification y
//! échappe, l'isolation réseau du conteneur reste la vraie protection. L'application de bureau n'est pas concernée.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};

use url::{Host, Url};

/// Pourquoi un lien est refusé (messages destinés à l'utilisateur).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UrlGuardError {
    #[error("Adresse invalide.")]
    Invalid,
    #[error("Seules les adresses http et https sont acceptées.")]
    Scheme,
    #[error("Cette adresse appartient au réseau du serveur : refusée.")]
    Private,
    #[error("Adresse introuvable.")]
    NotFound,
}

/// Réseaux IPv4 refusés (adresse, longueur du préfixe).
const BLOCKED_V4: &[([u8; 4], u8)] = &[
    ([0, 0, 0, 0], 8),
    ([10, 0, 0, 0], 8),
    ([100, 64, 0, 0], 10),
    ([127, 0, 0, 0], 8),
    ([169, 254, 0, 0], 16),
    ([172, 16, 0, 0], 12),
    ([192, 0, 0, 0], 24),
    ([192, 0, 2, 0], 24),
    ([192, 168, 0, 0], 16),
    ([198, 18, 0, 0], 15),
    ([198, 51, 100, 0], 24),
    ([203, 0, 113, 0], 24),
    ([224, 0, 0, 0], 4),
    ([240, 0, 0, 0], 4),
];

/// Réseaux IPv6 refusés : adresse non spécifiée, boucle locale, ULA, lien local, multicast, documentation,
/// NAT64 et 6to4 (ces deux derniers mènent à des adresses IPv4 qu'on ne voit pas).
const BLOCKED_V6: &[(u128, u8)] = &[
    (0, 128),
    (1, 128),
    (0xfc00 << 112, 7),
    (0xfe80 << 112, 10),
    (0xff00 << 112, 8),
    (0x2001_0db8 << 96, 32),
    (0x0064_ff9b << 96, 96),
    (0x2002 << 112, 16),
];

fn in_v4(ip: Ipv4Addr, net: [u8; 4], prefix: u8) -> bool {
    let mask = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix) };
    u32::from(ip) & mask == u32::from(Ipv4Addr::from(net)) & mask
}

fn in_v6(ip: Ipv6Addr, net: u128, prefix: u8) -> bool {
    let mask = if prefix == 0 { 0 } else { u128::MAX << (128 - prefix) };
    u128::from(ip) & mask == net & mask
}

/// L'adresse IP est-elle privée, locale ou réservée ? Une IPv4 encapsulée (`::ffff:a.b.c.d`) est jugée comme IPv4.
pub fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => BLOCKED_V4.iter().any(|(net, p)| in_v4(v4, *net, *p)),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_private_ip(IpAddr::V4(v4)),
            None => BLOCKED_V6.iter().any(|(net, p)| in_v6(v6, *net, *p)),
        },
    }
}

/// Comme [`is_private_ip`], pour un texte : ce qui n'est pas une adresse IP est refusé.
pub fn is_private_address(address: &str) -> bool {
    address.parse::<IpAddr>().map(is_private_ip).unwrap_or(true)
}

/// Résolution DNS du système.
pub fn resolve_system(host: &str) -> std::io::Result<Vec<IpAddr>> {
    Ok((host, 0).to_socket_addrs()?.map(|a| a.ip()).collect())
}

/// Vérifie un lien avec le DNS du système : renvoie l'adresse nettoyée, ou une erreur lisible. Bloquant.
pub fn check_url(input: &str) -> Result<String, UrlGuardError> {
    check_url_with(input, resolve_system)
}

/// Vérifie un lien en résolvant les noms avec `resolve` (les tests en donnent un faux).
pub fn check_url_with(
    input: &str,
    resolve: impl Fn(&str) -> std::io::Result<Vec<IpAddr>>,
) -> Result<String, UrlGuardError> {
    let text = input.trim();
    if text.is_empty() || text.chars().count() > 2048 {
        return Err(UrlGuardError::Invalid);
    }
    let url = Url::parse(text).map_err(|_| UrlGuardError::Invalid)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(UrlGuardError::Scheme);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(UrlGuardError::Invalid);
    }
    let ip = match url.host() {
        Some(Host::Ipv4(v4)) => Some(IpAddr::V4(v4)),
        Some(Host::Ipv6(v6)) => Some(IpAddr::V6(v6)),
        Some(Host::Domain(domain)) => match domain.parse::<IpAddr>() {
            Ok(ip) => Some(ip),
            Err(_) => {
                let addresses = resolve(domain).map_err(|_| UrlGuardError::NotFound)?;
                if addresses.is_empty() || addresses.iter().any(|a| is_private_ip(*a)) {
                    return Err(UrlGuardError::Private);
                }
                None
            }
        },
        None => return Err(UrlGuardError::Invalid),
    };
    if ip.is_some_and(is_private_ip) {
        return Err(UrlGuardError::Private);
    }
    Ok(url.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public(_: &str) -> std::io::Result<Vec<IpAddr>> {
        Ok(vec!["93.184.216.34".parse().unwrap()])
    }

    #[test]
    fn private_addresses() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fe80::1",
            "fd12:3456::1",
            "ff02::1",
            "2001:db8::1",
            "64:ff9b::7f00:1",
            "2002:7f00:1::",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "not-an-ip",
            "",
        ] {
            assert!(is_private_address(ip), "{ip}");
        }
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "172.32.0.1",
            "100.128.0.1",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
        ] {
            assert!(!is_private_address(ip), "{ip}");
        }
    }

    #[test]
    fn only_public_http_links() {
        assert_eq!(
            check_url_with("  https://www.youtube.com/watch?v=abc  ", public).unwrap(),
            "https://www.youtube.com/watch?v=abc"
        );
        assert_eq!(check_url_with("http://93.184.216.34/a", public).unwrap(), "http://93.184.216.34/a");
        let resolve = |h: &str| {
            if h == "localhost" { Ok(vec!["127.0.0.1".parse().unwrap()]) } else { public(h) }
        };
        let long = format!("https://exemple.org/{}", "a".repeat(3000));
        for (bad, want) in [
            ("file:///etc/passwd", UrlGuardError::Scheme),
            ("ftp://exemple.org/a", UrlGuardError::Scheme),
            ("javascript:alert(1)", UrlGuardError::Scheme),
            ("http://127.0.0.1:5180/api/health", UrlGuardError::Private),
            ("http://localhost/x", UrlGuardError::Private),
            ("http://2130706433/", UrlGuardError::Private),
            ("http://0x7f.1/", UrlGuardError::Private),
            ("http://[::1]/", UrlGuardError::Private),
            ("http://[::ffff:7f00:1]/", UrlGuardError::Private),
            ("http://169.254.169.254/latest/meta-data/", UrlGuardError::Private),
            ("http://user:pass@exemple.org/", UrlGuardError::Invalid),
            ("http://user@exemple.org/", UrlGuardError::Invalid),
            ("", UrlGuardError::Invalid),
            ("pas une adresse", UrlGuardError::Invalid),
            (long.as_str(), UrlGuardError::Invalid),
        ] {
            assert_eq!(check_url_with(bad, resolve), Err(want), "{bad}");
        }
    }

    #[test]
    fn names_resolving_inside_are_refused() {
        let mixed = |_: &str| Ok(vec!["93.184.216.34".parse().unwrap(), "10.0.0.5".parse().unwrap()]);
        assert_eq!(check_url_with("https://interne.exemple.org/", mixed), Err(UrlGuardError::Private));
        let none = |_: &str| Ok(vec![]);
        assert_eq!(check_url_with("https://vide.exemple.org/", none), Err(UrlGuardError::Private));
        let missing = |_: &str| Err(std::io::Error::other("ENOTFOUND"));
        assert_eq!(check_url_with("https://nexiste.pas/", missing), Err(UrlGuardError::NotFound));
        assert_eq!(UrlGuardError::NotFound.to_string(), "Adresse introuvable.");
    }
}

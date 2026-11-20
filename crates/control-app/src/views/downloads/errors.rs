//! Erreurs de yt-dlp traduites en conseils compréhensibles (comme `friendlyError` de l'original).

use crate::i18n::{t, tv};

fn any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| text.contains(n))
}

/// « age-restricted », « age restricted »… (`age.restricted` dans l'original).
fn age_restricted(text: &str) -> bool {
    text.match_indices("restricted").any(|(i, _)| {
        let before: Vec<char> = text[..i].chars().rev().take(4).collect();
        before.len() == 4 && before[1] == 'e' && before[2] == 'g' && before[3] == 'a'
    })
}

/// Message lisible pour une erreur de yt-dlp (le texte d'origine s'il n'est pas reconnu).
pub fn friendly_error(raw: &str) -> String {
    // Codes posés par le noyau : le texte est traduit ici, dans la langue de l'interface.
    if raw == "yt-dlp:NOFILES" {
        return t("dl.errNoFiles");
    }
    if let Some(code) = raw.strip_prefix("yt-dlp:EXIT:")
        && code.trim_start_matches('-').chars().all(|c| c.is_ascii_digit())
        && !code.trim_start_matches('-').is_empty()
    {
        return tv("dl.errExit", &[("code", &code)]);
    }
    let l = raw.to_lowercase();
    if (l.contains("could not copy") && l.contains("cookie database"))
        || any(&l, &["failed to decrypt", "dpapi", "cookie database"])
    {
        return t("dl.errCookies");
    }
    let bot =
        (l.contains("confirm you") && l.contains("re not a bot")) || any(&l, &["sign in to confirm", "http error 429"]);
    if bot {
        return t("dl.errBot");
    }
    if any(&l, &["confirm your age", "inappropriate for some users"]) || age_restricted(&l) {
        return t("dl.errAge");
    }
    if any(
        &l,
        &[
            "video unavailable",
            "is unavailable",
            "private video",
            "removed",
            "not available in your country",
            "blocked it",
        ],
    ) {
        return t("dl.errUnavailable");
    }
    if any(&l, &["unable to download", "getaddrinfo", "timed out", "network", "connection"]) {
        return t("dl.errNetwork");
    }
    raw.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_errors_are_explained() {
        let _guard = crate::i18n::TEST_LANG.lock().unwrap_or_else(|e| e.into_inner());
        crate::i18n::set_language("fr");
        assert_eq!(friendly_error("yt-dlp:NOFILES"), t("dl.errNoFiles"));
        assert!(friendly_error("yt-dlp:EXIT:2").contains('2'));
        assert_eq!(friendly_error("ERROR: Sign in to confirm you’re not a bot"), t("dl.errBot"));
        assert_eq!(friendly_error("ERROR: [youtube] x: Video unavailable"), t("dl.errUnavailable"));
        assert_eq!(friendly_error("This video is age-restricted"), t("dl.errAge"));
        assert_eq!(friendly_error("ERROR: Could not copy Chrome cookie database"), t("dl.errCookies"));
        assert_eq!(friendly_error("urlopen error: timed out"), t("dl.errNetwork"));
        assert_eq!(friendly_error("autre chose"), "autre chose");
        assert_eq!(friendly_error("yt-dlp:EXIT:abc"), "yt-dlp:EXIT:abc");
    }
}

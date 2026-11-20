//! Fichier cookies.txt d'un compte (version web) : le serveur n'a pas accès au navigateur de l'utilisateur, qui
//! exporte lui-même ses cookies YouTube et les envoie. Le fichier reste sur le serveur, propre à ce compte, lisible
//! par lui seul, et ne sert qu'à yt-dlp (voir [`super::DownloadsConfig::cookies_file`]).

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::LazyLock;
use std::time::UNIX_EPOCH;

use regex::Regex;
use serde::Serialize;

pub const MAX_BYTES: usize = 2 * 1024 * 1024;
const HEADER: &str = "# Netscape HTTP Cookie File";

static HEADER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^#\s*(Netscape )?HTTP Cookie File").expect("regex constante"));

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CookiesError {
    #[error("Fichier vide.")]
    Empty,
    #[error("Fichier trop volumineux (2 Mo au plus).")]
    TooLarge,
    #[error("Ce n’est pas un fichier texte.")]
    NotText,
    #[error(
        "Ligne {0} invalide : ce n’est pas un fichier cookies.txt (format Netscape, champs séparés par des tabulations)."
    )]
    BadLine(usize),
    #[error("Aucun cookie dans ce fichier.")]
    NoCookies,
    #[error("Enregistrement impossible : {0}")]
    Io(String),
}

/// Fichier vérifié et remis en forme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normalized {
    pub text: String,
    pub count: usize,
    /// Domaines, sans point initial, triés.
    pub domains: Vec<String>,
}

/// Vérifie et remet en forme un fichier au format Netscape. Chaque ligne est un commentaire, ou un cookie à
/// 7 champs séparés par des tabulations (ce que yt-dlp exige). Fins de ligne Unix, en-tête unique.
pub fn normalize(input: &str) -> Result<Normalized, CookiesError> {
    if input.trim().is_empty() {
        return Err(CookiesError::Empty);
    }
    if input.len() > MAX_BYTES {
        return Err(CookiesError::TooLarge);
    }
    if input.contains('\0') {
        return Err(CookiesError::NotText);
    }
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut kept: Vec<String> = Vec::new();
    let mut count = 0;
    let mut domains = BTreeSet::new();
    for (i, raw) in input.split("\r\n").flat_map(|l| l.split(['\r', '\n'])).enumerate() {
        // Les tabulations finales comptent : une valeur de cookie peut être vide.
        let line = raw.trim_end_matches(|c: char| c.is_whitespace() && c != '\t');
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with('#') && !line.starts_with("#HttpOnly_") {
            if !kept.is_empty() || !HEADER_RE.is_match(line) {
                kept.push(line.to_string());
            }
            continue;
        }
        let mut fields: Vec<&str> = line.split('\t').collect();
        // Tabulations en trop en fin de ligne (éditeurs, exports approximatifs) : sans conséquence.
        while fields.len() > 7 && fields.last().is_some_and(|f| f.is_empty()) {
            fields.pop();
        }
        if fields.len() != 7 {
            return Err(CookiesError::BadLine(i + 1));
        }
        count += 1;
        kept.push(fields.join("\t"));
        let domain = fields[0].strip_prefix("#HttpOnly_").unwrap_or(fields[0]);
        domains.insert(domain.strip_prefix('.').unwrap_or(domain).to_lowercase());
    }
    if count == 0 {
        return Err(CookiesError::NoCookies);
    }
    Ok(Normalized { text: format!("{HEADER}\n{}\n", kept.join("\n")), count, domains: domains.into_iter().collect() })
}

/// Résumé d'un fichier enregistré (le contenu ne repart jamais vers le navigateur).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CookiesStatus {
    pub present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domains: Option<Vec<String>>,
}

/// Vérifie puis enregistre le fichier (droits 600 sous Unix). Renvoie (nombre de cookies, domaines).
pub fn save(file: &Path, input: &str) -> Result<(usize, Vec<String>), CookiesError> {
    let n = normalize(input)?;
    let io = |e: std::io::Error| CookiesError::Io(e.to_string());
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let mut tmp = file.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = std::path::PathBuf::from(tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let res = (|| {
        use std::io::Write;
        let mut f = opts.open(&tmp)?;
        f.write_all(n.text.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, file)
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(io(e));
    }
    Ok((n.count, n.domains))
}

pub fn status(file: &Path) -> CookiesStatus {
    let absent = CookiesStatus { present: false, saved_at: None, count: None, domains: None };
    let Ok(md) = std::fs::metadata(file) else { return absent };
    let Ok(text) = std::fs::read_to_string(file) else { return absent };
    let Ok(n) = normalize(&text) else { return absent };
    let saved_at = md.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64);
    CookiesStatus {
        present: true,
        saved_at,
        count: Some(n.count),
        domains: Some(n.domains.into_iter().take(8).collect()),
    }
}

pub fn clear(file: &Path) {
    let _ = std::fs::remove_file(file);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(domain: &str, name: &str, value: &str) -> String {
        [domain, "TRUE", "/", "TRUE", "1893456000", name, value].join("\t")
    }

    fn sample() -> String {
        [
            "# Netscape HTTP Cookie File".to_string(),
            "# https://curl.haxx.se/rfc/cookie_spec.html".into(),
            String::new(),
            row(".youtube.com", "SID", "abc"),
            format!("#HttpOnly_{}", row(".google.com", "__Secure-1PSID", "def")),
            row("www.youtube.com", "PREF", "f6=40000000"),
        ]
        .join("\n")
    }

    #[test]
    fn netscape_format_is_accepted_and_normalized() {
        let ok = normalize(&sample()).unwrap();
        assert_eq!(ok.count, 3);
        assert_eq!(ok.domains, ["google.com", "www.youtube.com", "youtube.com"]);
        assert!(ok.text.starts_with("# Netscape HTTP Cookie File\n"));
        assert_eq!(ok.text.matches("Netscape HTTP Cookie File").count(), 1);
        assert!(ok.text.contains("#HttpOnly_.google.com\t"));
        assert!(ok.text.contains("# https://curl.haxx.se"));

        let loose = normalize(&format!("\u{feff}{}\r\n", row(".youtube.com", "SID", "abc"))).unwrap();
        assert!(loose.text.starts_with("# Netscape HTTP Cookie File\n"));
        assert!(!loose.text.contains('\r'));
        assert_eq!(loose.count, 1);

        // Valeur vide : la ligne finit par une tabulation, elle reste valide.
        let empty_value = normalize(&format!("{}\n", row(".youtube.com", "X", ""))).unwrap();
        assert_eq!(empty_value.count, 1);
        assert!(empty_value.text.ends_with("\tX\t\n"));
    }

    #[test]
    fn other_files_are_refused() {
        let big = row(".youtube.com", "SID", &"x".repeat(MAX_BYTES));
        let with_junk = format!("{}\nn'importe quoi", row(".youtube.com", "SID", "abc"));
        let with_nul = format!("{}\0", row(".youtube.com", "SID", "abc"));
        for (bad, want) in [
            ("", CookiesError::Empty),
            ("   \n", CookiesError::Empty),
            ("# Netscape HTTP Cookie File\n# rien\n", CookiesError::NoCookies),
            (".youtube.com TRUE / TRUE 1893456000 SID abc", CookiesError::BadLine(1)),
            (with_junk.as_str(), CookiesError::BadLine(2)),
            (with_nul.as_str(), CookiesError::NotText),
            (big.as_str(), CookiesError::TooLarge),
        ] {
            assert_eq!(normalize(bad), Err(want), "{:?}", bad.chars().take(40).collect::<String>());
        }
    }

    #[test]
    fn save_status_clear() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("u/cookies.txt");
        assert_eq!(status(&file), CookiesStatus { present: false, saved_at: None, count: None, domains: None });
        assert_eq!(serde_json::to_value(status(&file)).unwrap(), serde_json::json!({"present": false}));
        assert!(save(&file, "pas des cookies").is_err());
        let (count, domains) = save(&file, &sample()).unwrap();
        assert_eq!(count, 3);
        assert!(domains.contains(&"youtube.com".to_string()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let st = status(&file);
        assert!(st.present && st.count == Some(3) && st.saved_at.is_some());
        assert!(!serde_json::to_string(&st).unwrap().contains("abc"));
        clear(&file);
        assert!(!file.exists());
    }
}

//! Dialogue avec yt-dlp : arguments selon le format et la qualité, lecture des lignes de progression.
//!
//! yt-dlp écrit sa progression avec des gabarits à nous (`CTRLPROG`, `CTRLPOST`, `CTRLFILE`, champs séparés par
//! des tabulations) : rien à deviner dans ses messages, qui changent d'une version à l'autre.

use std::ffi::OsString;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::model::Kind;
use crate::settings::Dl;

pub const TAB: char = '\t';

/// « (Official Video) », « [Lyrics] », « (4K Remaster) » : du bruit dans le titre. Expression donnée telle quelle à
/// yt-dlp (`--replace-in-metadata`, syntaxe Python), compatible avec la crate `regex`.
pub const NOISE: &str = r"(?i)\s*[\(\[]\s*(official\s+(music\s+)?(video|audio|lyric\s+video)|lyrics?(\s+video)?|audio|video|hd|hq|4k(\s+remaster(ed)?)?|remaster(ed)?(\s+\d{4})?)\s*[\)\]]";

static NOISE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(NOISE).expect("regex constante"));
static ERROR_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^ERROR:\s*(\[[^\]]+\]\s*)?").expect("regex constante"));

/// Formats audio acceptés par `--audio-format` ; « original » (`best`) garde le flux tel quel.
const AUDIO_FORMATS: &[&str] = &["best", "aac", "alac", "flac", "m4a", "mp3", "opus", "vorbis", "wav"];

/// Format audio demandé, ramené à une valeur connue de yt-dlp (mp3 sinon).
pub fn audio_format(format: &str) -> &'static str {
    let f = format.trim().to_ascii_lowercase();
    let f = if f == "original" { "best".to_string() } else { f };
    AUDIO_FORMATS.iter().find(|x| **x == f).copied().unwrap_or("mp3")
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Les « mix » YouTube (`list=RD…`) sont infinis : on ne télécharge que la vidéo.
pub fn is_mix(url: &str) -> bool {
    url.contains("?list=RD") || url.contains("&list=RD")
}

/// Cible de recherche yt-dlp (`scsearch12:texte`), `None` pour une source inconnue ou un texte vide.
pub fn search_target(source: &str, query: &str, limit: usize) -> Option<String> {
    let prefix = match source {
        "youtube" => "ytsearch",
        "soundcloud" => "scsearch",
        _ => return None,
    };
    let text: String = query.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect();
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(format!("{prefix}{}:{text}", limit.clamp(1, 25)))
}

/// Cookies donnés à yt-dlp : le fichier imposé par le serveur (envoyé par le compte), sinon le cookies.txt choisi
/// dans les réglages, sinon les cookies d'un navigateur.
pub fn cookie_args(forced: Option<&Path>, dl: &Dl, exists: impl Fn(&Path) -> bool) -> Vec<OsString> {
    if let Some(f) = forced.filter(|f| exists(f)) {
        return vec!["--cookies".into(), f.into()];
    }
    let file = dl.cookies_file.trim();
    if !file.is_empty() && exists(Path::new(file)) {
        return vec!["--cookies".into(), file.into()];
    }
    let browser = dl.cookies.trim();
    if !browser.is_empty() {
        return vec!["--cookies-from-browser".into(), browser.into()];
    }
    Vec::new()
}

/// Options communes de l'analyse d'un lien et de la recherche (JSON sur la sortie, sans téléchargement).
fn json_args() -> Vec<OsString> {
    ["-J", "--flat-playlist", "--no-warnings", "--encoding", "utf-8", "--ignore-config"].map(OsString::from).to_vec()
}

/// Arguments de `yt-dlp -J` pour analyser un lien.
pub fn info_args(url: &str, cookies: Vec<OsString>) -> Vec<OsString> {
    let mut a = json_args();
    if is_mix(url) {
        a.push("--no-playlist".into());
    }
    a.extend(cookies);
    a.push("--".into());
    a.push(url.into());
    a
}

/// Arguments d'une recherche (`target` vient de [`search_target`]).
pub fn search_args(target: &str, cookies: Vec<OsString>) -> Vec<OsString> {
    let mut a = json_args();
    a.extend(cookies);
    a.push("--".into());
    a.push(target.into());
    a
}

/// Ce qui définit un téléchargement, pour construire ses arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadSpec<'a> {
    pub url: &'a str,
    pub is_playlist: bool,
    pub kind: Kind,
    pub format: &'a str,
    pub quality: &'a str,
    pub parse_title: bool,
}

/// Programmes compagnons et cookies.
#[derive(Debug, Clone, Default)]
pub struct Extras<'a> {
    pub cookies: Vec<OsString>,
    pub ffmpeg: Option<&'a Path>,
    pub deno: Option<&'a Path>,
}

/// Arguments d'un téléchargement : audio (mp3, m4a, opus, flac, original ; débit pour le mp3) ou vidéo mp4
/// (480p à 4K, ou la meilleure), avec métadonnées, pochette et, pour l'audio, « Artiste - Titre » séparés.
pub fn download_args(spec: &DownloadSpec<'_>, out_dir: &Path, extras: Extras<'_>) -> Vec<OsString> {
    let template = format!("{}%(title).150B.%(ext)s", if spec.is_playlist { "%(playlist_title&{}/|)s" } else { "" });
    let progress = [
        "download:CTRLPROG",
        "%(progress.status)s",
        "%(progress.downloaded_bytes)s",
        "%(progress.total_bytes)s",
        "%(progress.total_bytes_estimate)s",
        "%(progress.speed)s",
        "%(progress.eta)s",
        "%(info.playlist_index)s",
        "%(info.n_entries)s",
        "%(info.title)s",
    ]
    .join("\t");
    let post = ["postprocess:CTRLPOST", "%(progress.status)s", "%(info.title)s"].join("\t");
    let file = ["after_move:CTRLFILE", "%(filepath)s", "%(webpage_url)s"].join("\t");

    let mut a: Vec<OsString> = [
        "--ignore-config",
        "--no-colors",
        "--newline",
        "--no-mtime",
        "--windows-filenames",
        "--no-warnings",
        "--ignore-errors",
        "--encoding",
        "utf-8",
        "--progress",
        "-o",
    ]
    .map(OsString::from)
    .to_vec();
    a.push(out_dir.join(template).into_os_string());
    for s in ["--progress-template", &progress, "--progress-template", &post, "--print", &file] {
        a.push(s.into());
    }
    for s in ["--no-simulate", "--embed-metadata", "--embed-thumbnail", "--convert-thumbnails", "jpg"] {
        a.push(s.into());
    }
    a.push(if spec.is_playlist { "--yes-playlist" } else { "--no-playlist" }.into());

    match spec.kind {
        Kind::Audio => {
            let format = audio_format(spec.format);
            for s in ["-f", "bestaudio/best", "-x", "--audio-format", format] {
                a.push(s.into());
            }
            if format == "mp3" && is_digits(spec.quality) {
                a.push("--audio-quality".into());
                a.push(format!("{}K", spec.quality).into());
            }
            if spec.parse_title {
                for s in
                    ["--parse-metadata", "title:%(artist)s - %(title)s", "--replace-in-metadata", "title", NOISE, ""]
                {
                    a.push(s.into());
                }
            }
        }
        Kind::Video => {
            let cap = if is_digits(spec.quality) { format!("res:{},", spec.quality) } else { String::new() };
            a.push("-S".into());
            a.push(format!("{cap}vcodec:h264,acodec:aac").into());
            for s in ["--merge-output-format", "mp4", "--embed-chapters"] {
                a.push(s.into());
            }
        }
    }
    a.extend(extras.cookies);
    if let Some(dir) = extras.ffmpeg.and_then(Path::parent) {
        a.push("--ffmpeg-location".into());
        a.push(dir.into());
    }
    if let Some(deno) = extras.deno {
        let mut v = OsString::from("deno:");
        v.push(deno);
        a.push("--js-runtimes".into());
        a.push(v);
    }
    a.push("--".into());
    a.push(spec.url.into());
    a
}

/// Valeur absente dans un gabarit yt-dlp (`NA`, `None`, vide).
pub fn na(v: &str) -> Option<&str> {
    match v {
        "" | "NA" | "None" => None,
        v => Some(v),
    }
}

pub fn num(v: &str) -> Option<f64> {
    na(v).and_then(|s| s.trim().parse::<f64>().ok()).filter(|n| n.is_finite())
}

/// Une ligne `CTRLPROG`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Progress {
    /// `downloading`, `finished`…
    pub status: String,
    pub downloaded: Option<f64>,
    /// Taille annoncée, sinon estimée.
    pub size: Option<f64>,
    /// Octets par seconde.
    pub speed: Option<f64>,
    /// Secondes.
    pub eta: Option<f64>,
    /// Rang dans la playlist (à partir de 1).
    pub index: Option<i64>,
    pub count: Option<i64>,
    pub title: Option<String>,
}

impl Progress {
    /// Avancement global (0 à 1) : celui de l'élément, rapporté à sa place dans la playlist.
    pub fn overall(&self, fallback_count: Option<i64>) -> f64 {
        let item = match (self.downloaded, self.size) {
            (Some(done), Some(size)) if size > 0.0 => (done / size).clamp(0.0, 1.0),
            _ => 0.0,
        };
        let n = self.count.filter(|n| *n > 0).or(fallback_count);
        match (self.index.filter(|i| *i > 0), n.filter(|n| *n > 0)) {
            (Some(i), Some(n)) => (((i - 1) as f64 + item) / n as f64).clamp(0.0, 1.0),
            _ => item,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Progress(Progress),
    /// Post-traitement (conversion, pochette…) : titre de l'élément.
    Post(Option<String>),
    /// Fichier final écrit, avec l'adresse de sa page.
    File {
        path: String,
        url: Option<String>,
    },
    /// Message d'erreur, sans le préfixe `ERROR: [extracteur]`.
    Error(String),
    Other,
}

/// Lit une ligne de la sortie de yt-dlp.
pub fn parse_line(line: &str) -> Line {
    if let Some(rest) = line.strip_prefix("CTRLPROG\t") {
        let f: Vec<&str> = rest.splitn(9, TAB).collect();
        let get = |i: usize| f.get(i).copied().unwrap_or("");
        let as_int = |i: usize| num(get(i)).map(|n| n as i64);
        return Line::Progress(Progress {
            status: get(0).to_string(),
            downloaded: num(get(1)),
            size: num(get(2)).filter(|n| *n > 0.0).or_else(|| num(get(3))),
            speed: num(get(4)),
            eta: num(get(5)),
            index: as_int(6),
            count: as_int(7),
            title: na(get(8)).map(str::to_string),
        });
    }
    if line == "CTRLPOST" || line.starts_with("CTRLPOST\t") {
        let title = line.splitn(3, TAB).nth(2).and_then(na).map(str::to_string);
        return Line::Post(title);
    }
    if let Some(rest) = line.strip_prefix("CTRLFILE\t") {
        let mut f = rest.splitn(2, TAB);
        let path = f.next().unwrap_or("").to_string();
        let url = f.next().and_then(na).map(str::to_string);
        if path.is_empty() {
            return Line::Other;
        }
        return Line::File { path, url };
    }
    if line.starts_with("ERROR:") {
        return Line::Error(ERROR_PREFIX.replace(line, "").into_owned());
    }
    Line::Other
}

/// Titre sans « (Official Video) », « [Lyrics] »… : ce que fait `--replace-in-metadata` avec [`NOISE`].
pub fn clean_title(title: &str) -> String {
    NOISE_RE.replace_all(title, "").trim().to_string()
}

/// « Artiste - Titre » coupé comme le fait `--parse-metadata "title:%(artist)s - %(title)s"` : au dernier
/// « - » (l'expression de yt-dlp est gourmande), les deux côtés non vides. Le titre est ensuite nettoyé.
pub fn split_artist_title(title: &str) -> (Option<String>, String) {
    if let Some(pos) = title.rfind(" - ") {
        let (artist, rest) = (&title[..pos], &title[pos + 3..]);
        if !artist.is_empty() && !rest.is_empty() {
            return (Some(artist.to_string()), clean_title(rest));
        }
    }
    (None, clean_title(title))
}

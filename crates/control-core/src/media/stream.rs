//! Lecteur vidrift (https://vidrift.net/docs) : adresse d'intégration, serveur utilisé, reprise et épisode
//! voisin. Tout est pur : l'interface (fenêtre ou navigateur) affiche l'adresse et relaie les messages du lecteur.

use serde_json::{Value, json};

use super::types::Details;
use super::watch::Progress;

/// Les deux serveurs officiels (le premier par défaut).
pub const OFFICIAL_STREAMS: [&str; 2] = ["https://embed.vidrift.net", "https://embed.vidrift.in"];

/// En dessous de cette position (s), on ne propose pas de reprendre : autant recommencer.
pub const RESUME_MIN: f64 = 30.0;

/// Serveur du lecteur : celui imposé par l'environnement (tests), sinon le réglage s'il est officiel, sinon le
/// premier serveur officiel.
pub fn stream_base(setting: &str, override_base: Option<&str>) -> String {
    if let Some(o) = override_base.map(str::trim).filter(|o| !o.is_empty()) {
        return o.trim_end_matches('/').to_string();
    }
    if OFFICIAL_STREAMS.contains(&setting) { setting.to_string() } else { OFFICIAL_STREAMS[0].to_string() }
}

/// Origine (`schéma://hôte[:port]`) d'une adresse, pour vérifier d'où viennent les messages du lecteur.
pub fn origin(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let o = u.origin();
    o.is_tuple().then(|| o.ascii_serialization())
}

/// Origines que la page peut intégrer (CSP `frame-src`) : les deux officielles, plus un serveur de test.
pub fn frame_origins(extra: Option<&str>) -> Vec<String> {
    let mut list: Vec<String> = OFFICIAL_STREAMS.iter().map(|s| s.to_string()).collect();
    list.extend(extra.and_then(origin));
    list
}

/// Ce que le lecteur doit jouer.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EmbedTarget {
    /// `movie`, `tv` ou `anime`.
    pub kind: String,
    pub id: String,
    pub season: Option<i64>,
    pub episode: Option<i64>,
}

/// Habillage du lecteur.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EmbedOptions {
    pub title: Option<String>,
    /// Couleur d'accent, `rrggbb` ou `#rrggbb`.
    pub color: Option<String>,
    /// Image affichée avant la lecture (https seulement).
    pub poster: Option<String>,
    /// Version doublée en anglais (animés).
    pub dub: bool,
}

fn is_hex_color(c: &str) -> bool {
    let hex = c.strip_prefix('#').unwrap_or(c);
    hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Adresse d'intégration du lecteur. Un film d'animation (épisode absent) n'a pas d'épisode dans l'adresse.
pub fn embed_url(base: &str, target: &EmbedTarget, opts: &EmbedOptions) -> String {
    let path = match target.kind.as_str() {
        "movie" => format!("/embed/movie/{}", target.id),
        "tv" => format!("/embed/tv/{}/{}/{}", target.id, target.season.unwrap_or(1), target.episode.unwrap_or(1)),
        _ => match target.episode {
            Some(ep) => format!("/embed/anilist/{}/{ep}", target.id),
            None => format!("/embed/anilist/{}", target.id),
        },
    };
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    if let Some(title) = opts.title.as_deref().filter(|t| !t.is_empty()) {
        q.append_pair("title", title);
    }
    q.append_pair("brand", "control");
    if let Some(color) = opts.color.as_deref().filter(|c| is_hex_color(c)) {
        q.append_pair("brandColor", &color.replace('#', ""));
    }
    if let Some(poster) = opts.poster.as_deref().filter(|p| p.starts_with("https://")) {
        q.append_pair("poster", poster);
    }
    q.append_pair("exit", "1");
    q.append_pair("fs", "parent");
    q.append_pair("font", "Inter");
    q.append_pair("hide", "cast");
    if target.kind == "anime" && opts.dub {
        q.append_pair("dub", "1");
    }
    format!("{base}{path}?{}", q.finish())
}

/// Position où reprendre un épisode d'après sa progression enregistrée (rien s'il est fini ou à peine commencé).
pub fn resume_at(rows: &[Progress], season: Option<i64>, episode: Option<i64>) -> Option<f64> {
    let row = rows.iter().find(|r| r.season == season && r.episode == episode)?;
    (!row.finished && row.position >= RESUME_MIN).then_some(row.position)
}

/// Message à poster au lecteur une fois chargé pour qu'il reprenne à `position`.
pub fn resume_message(position: f64) -> Value {
    json!({ "type": "vidrift:resume", "currentTime": position })
}

/// Épisode voisin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct EpisodeRef {
    pub season: Option<i64>,
    pub episode: i64,
}

/// Épisode suivant (`dir` = 1) ou précédent (`dir` = -1), d'après les saisons (séries) ou la liste des épisodes
/// (animés). On passe d'une saison à l'autre ; rien pour un film ou en bout de série.
pub fn neighbour(details: &Details, season: Option<i64>, episode: Option<i64>, dir: i64) -> Option<EpisodeRef> {
    let episode = episode?;
    let next = episode + dir;
    match details {
        Details::Anime(a) => {
            let count = a.episodes.len() as i64;
            (next >= 1 && next <= count).then_some(EpisodeRef { season: None, episode: next })
        }
        Details::Tmdb(t) => {
            let seasons = &t.seasons;
            let idx = seasons.iter().position(|s| Some(s.number) == season)?;
            if next >= 1 && next <= seasons[idx].episode_count {
                return Some(EpisodeRef { season, episode: next });
            }
            let other = seasons.get(usize::try_from(idx as i64 + dir).ok()?)?;
            Some(EpisodeRef { season: Some(other.number), episode: if dir > 0 { 1 } else { other.episode_count } })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::types::{AnimeDetails, AnimeEpisode, SeasonSummary, TmdbDetails};

    #[test]
    fn base_prefers_override_then_official_setting() {
        assert_eq!(stream_base("https://embed.vidrift.in", None), "https://embed.vidrift.in");
        assert_eq!(stream_base("https://evil.example", None), "https://embed.vidrift.net");
        assert_eq!(stream_base("https://embed.vidrift.in", Some("http://127.0.0.1:9000/")), "http://127.0.0.1:9000");
        assert_eq!(
            frame_origins(Some("http://127.0.0.1:9000/x")),
            vec!["https://embed.vidrift.net", "https://embed.vidrift.in", "http://127.0.0.1:9000"]
        );
        assert_eq!(frame_origins(Some("pas une adresse")).len(), 2);
    }

    #[test]
    fn embed_urls() {
        let base = "https://embed.vidrift.net";
        let movie = EmbedTarget { kind: "movie".into(), id: "603".into(), ..Default::default() };
        let opts = EmbedOptions {
            title: Some("Matrix & co".into()),
            color: Some("#FF00aa".into()),
            poster: Some("https://image.tmdb.org/t/p/w1280/x.jpg".into()),
            dub: true,
        };
        assert_eq!(
            embed_url(base, &movie, &opts),
            "https://embed.vidrift.net/embed/movie/603?title=Matrix+%26+co&brand=control&brandColor=FF00aa\
             &poster=https%3A%2F%2Fimage.tmdb.org%2Ft%2Fp%2Fw1280%2Fx.jpg&exit=1&fs=parent&font=Inter&hide=cast"
        );
        let tv = EmbedTarget { kind: "tv".into(), id: "1399".into(), season: Some(2), episode: None };
        assert_eq!(
            embed_url(
                base,
                &tv,
                &EmbedOptions { color: Some("red".into()), poster: Some("http://x".into()), ..Default::default() }
            ),
            "https://embed.vidrift.net/embed/tv/1399/2/1?brand=control&exit=1&fs=parent&font=Inter&hide=cast"
        );
        let anime = EmbedTarget { kind: "anime".into(), id: "21".into(), season: None, episode: Some(5) };
        assert!(embed_url(base, &anime, &opts).starts_with("https://embed.vidrift.net/embed/anilist/21/5?"));
        assert!(embed_url(base, &anime, &opts).ends_with("&hide=cast&dub=1"));
        let film = EmbedTarget { episode: None, ..anime };
        assert!(
            embed_url(base, &film, &EmbedOptions::default()).starts_with("https://embed.vidrift.net/embed/anilist/21?")
        );
    }

    fn progress(season: Option<i64>, episode: Option<i64>, position: f64, finished: bool) -> Progress {
        Progress {
            kind: "tv".into(),
            id: "1".into(),
            season,
            episode,
            position,
            duration: 1000.0,
            finished,
            title: String::new(),
            poster: None,
            backdrop: None,
            year: None,
            genres: json!([]),
            first_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn resume_only_unfinished_started_episodes() {
        let rows = vec![
            progress(Some(1), Some(1), 400.0, false),
            progress(Some(1), Some(2), 20.0, false),
            progress(Some(1), Some(3), 900.0, true),
        ];
        assert_eq!(resume_at(&rows, Some(1), Some(1)), Some(400.0));
        assert_eq!(resume_at(&rows, Some(1), Some(2)), None);
        assert_eq!(resume_at(&rows, Some(1), Some(3)), None);
        assert_eq!(resume_at(&rows, Some(2), Some(1)), None);
        assert_eq!(resume_message(400.0), json!({"type": "vidrift:resume", "currentTime": 400.0}));
    }

    #[test]
    fn neighbours_cross_seasons() {
        let season = |number, episode_count| SeasonSummary { number, episode_count, ..Default::default() };
        let tv =
            Details::Tmdb(Box::new(TmdbDetails { seasons: vec![season(1, 3), season(2, 5)], ..Default::default() }));
        assert_eq!(neighbour(&tv, Some(1), Some(2), 1), Some(EpisodeRef { season: Some(1), episode: 3 }));
        assert_eq!(neighbour(&tv, Some(1), Some(3), 1), Some(EpisodeRef { season: Some(2), episode: 1 }));
        assert_eq!(neighbour(&tv, Some(2), Some(1), -1), Some(EpisodeRef { season: Some(1), episode: 3 }));
        assert_eq!(neighbour(&tv, Some(1), Some(1), -1), None);
        assert_eq!(neighbour(&tv, Some(2), Some(5), 1), None);
        assert_eq!(neighbour(&tv, Some(9), Some(1), 1), None);
        let anime = Details::Anime(Box::new(AnimeDetails {
            episodes: (1..=2).map(|number| AnimeEpisode { number, ..Default::default() }).collect(),
            ..Default::default()
        }));
        assert_eq!(neighbour(&anime, None, Some(1), 1), Some(EpisodeRef { season: None, episode: 2 }));
        assert_eq!(neighbour(&anime, None, Some(2), 1), None);
        assert_eq!(neighbour(&anime, None, None, 1), None);
    }
}

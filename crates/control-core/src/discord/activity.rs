//! Ce qu'on montre sur Discord : construit à partir de ce qui se joue (musique, vidéo locale, streaming), puis
//! traduit en activité Rich Presence. Tout est pur : l'horloge est passée en paramètre.

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::model::{Kind, Track, TrackId};
use crate::settings;

/// Une pause de plus de 10 minutes efface la présence.
pub const IDLE_CLEAR_MS: i64 = 10 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityKind {
    Listening,
    Watching,
}

impl ActivityKind {
    /// Type d'activité Discord : 2 « Écoute », 3 « Regarde ».
    pub fn discord_type(self) -> i64 {
        match self {
            ActivityKind::Listening => 2,
            ActivityKind::Watching => 3,
        }
    }
}

/// Petite pastille : clés des images de l'application Discord (voir `docs/discord-assets/README.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Icon {
    Play,
    Pause,
    Watching,
    Video,
}

impl Icon {
    pub fn key(self) -> &'static str {
        match self {
            Icon::Play => "play",
            Icon::Pause => "pause",
            Icon::Watching => "watching",
            Icon::Video => "video",
        }
    }
}

/// Image locale à rendre accessible à Discord : pochette (empreinte du dossier des pochettes) ou image d'une vidéo.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
pub enum CoverRef {
    Cover(String),
    #[serde(rename = "vthumb")]
    VideoThumb(TrackId),
}

impl CoverRef {
    /// Clé du cache des adresses (`cover:<empreinte>`, `vthumb:<id>`), comme l'original.
    pub fn key(&self) -> String {
        match self {
            CoverRef::Cover(h) => format!("cover:{h}"),
            CoverRef::VideoThumb(id) => format!("vthumb:{id}"),
        }
    }

    fn valid(&self) -> bool {
        match self {
            CoverRef::Cover(h) => h.len() == 20 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            CoverRef::VideoThumb(id) => *id > 0,
        }
    }
}

/// Ce que l'application veut afficher (déjà normalisé par [`Activity::sanitized`]).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub kind: ActivityKind,
    pub details: String,
    pub state: Option<String>,
    /// Image publique (https), par exemple l'affiche d'un film.
    pub image: Option<String>,
    pub image_text: Option<String>,
    pub icon: Option<Icon>,
    pub icon_text: Option<String>,
    pub playing: bool,
    /// Secondes.
    pub position: f64,
    /// Secondes.
    pub duration: f64,
    pub cover: Option<CoverRef>,
}

/// Texte accepté par Discord : espaces repliés, 128 caractères au plus, 2 au moins.
pub fn clip(value: &str, max: usize) -> Option<String> {
    let s = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        return None;
    }
    let mut s = if s.chars().count() > max { format!("{}…", s.chars().take(max - 1).collect::<String>()) } else { s };
    if s.chars().count() < 2 {
        s.push(' '); // Discord exige au moins 2 caractères
    }
    Some(s)
}

fn https_url(v: Option<&str>) -> Option<String> {
    v.filter(|u| u.len() <= 256 && u.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://"))).map(str::to_string)
}

impl Activity {
    /// Normalise les textes et images ; `None` sans titre.
    pub fn sanitized(self) -> Option<Activity> {
        let details = clip(&self.details, 128)?;
        let opt = |v: Option<String>| v.and_then(|s| clip(&s, 128));
        let num = |v: f64| if v.is_finite() && v >= 0.0 { v } else { 0.0 };
        Some(Activity {
            kind: self.kind,
            details,
            state: opt(self.state),
            image: https_url(self.image.as_deref()),
            image_text: opt(self.image_text),
            icon: self.icon,
            icon_text: opt(self.icon_text),
            playing: self.playing,
            position: num(self.position),
            duration: num(self.duration),
            cover: self.cover.filter(CoverRef::valid),
        })
    }
}

/// Textes de la présence, dans la langue de l'interface.
#[derive(Debug, Clone, PartialEq)]
pub struct Texts {
    pub hide_music: String,
    pub hide_video: String,
    pub hide_movie: String,
    pub hide_tv: String,
    pub hide_anime: String,
    pub watching_now: String,
    pub listening_now: String,
    pub paused: String,
    pub local_video: String,
    pub unknown_artist: String,
}

impl Texts {
    pub fn fr() -> Texts {
        Texts {
            hide_music: "Écoute de la musique".into(),
            hide_video: "Regarde une vidéo".into(),
            hide_movie: "Regarde un film".into(),
            hide_tv: "Regarde une série".into(),
            hide_anime: "Regarde un animé".into(),
            watching_now: "Regarde".into(),
            listening_now: "Écoute".into(),
            paused: "En pause".into(),
            local_video: "Vidéo locale".into(),
            unknown_artist: "Artiste inconnu".into(),
        }
    }

    pub fn en() -> Texts {
        Texts {
            hide_music: "Listening to music".into(),
            hide_video: "Watching a video".into(),
            hide_movie: "Watching a movie".into(),
            hide_tv: "Watching a show".into(),
            hide_anime: "Watching anime".into(),
            watching_now: "Watching".into(),
            listening_now: "Listening".into(),
            paused: "Paused".into(),
            local_video: "Local video".into(),
            unknown_artist: "Unknown artist".into(),
        }
    }

    /// Selon la langue (`fr`, `en`, `auto`…) : le français par défaut, comme l'interface.
    pub fn for_language(lang: &str) -> Texts {
        if lang.starts_with("en") { Texts::en() } else { Texts::fr() }
    }

    fn hide_stream(&self, kind: &str) -> &str {
        match kind {
            "movie" => &self.hide_movie,
            "tv" => &self.hide_tv,
            "anime" => &self.hide_anime,
            _ => &self.hide_video,
        }
    }
}

/// Lecture en streaming (films, séries, animés), décrite par la page de lecture.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Streaming {
    /// `movie`, `tv` ou `anime`.
    pub kind: String,
    pub title: String,
    /// Saison et épisode, par exemple.
    pub subtitle: Option<String>,
    /// Affiche (https).
    pub poster: Option<String>,
    /// Position en secondes, mesurée à l'instant `at`.
    pub position: f64,
    pub duration: f64,
    pub playing: bool,
    /// Millisecondes Unix de la mesure de `position`.
    pub at: i64,
}

/// Ce qui se joue maintenant.
#[derive(Debug, Clone, Copy, Default)]
pub struct NowPlaying<'a> {
    pub track: Option<&'a Track>,
    pub playing: bool,
    /// Secondes.
    pub position: f64,
    /// Secondes.
    pub duration: f64,
    /// Page de streaming ouverte (prioritaire sur la bibliothèque).
    pub streaming: Option<&'a Streaming>,
    /// Début de la pause en cours (millisecondes Unix), voir [`PresenceFeed`].
    pub paused_since: Option<i64>,
}

/// L'activité à montrer, ou `None` pour effacer la présence (désactivée, rien en cours, longue pause…).
pub fn build_activity(np: &NowPlaying<'_>, opts: &settings::Discord, texts: &Texts, now_ms: i64) -> Option<Activity> {
    if !opts.enabled {
        return None;
    }
    if let Some(s) = np.streaming.filter(|_| opts.streaming) {
        return from_streaming(s, opts, texts, now_ms).sanitized();
    }
    let track = np.track?;
    let video = track.kind == Kind::Video;
    if (video && !opts.video) || (!video && !opts.music) {
        return None;
    }
    if !np.playing && np.paused_since.is_some_and(|t| now_ms - t > IDLE_CLEAR_MS) {
        return None;
    }
    from_track(track, np, opts, texts).sanitized()
}

fn from_streaming(s: &Streaming, opts: &settings::Discord, texts: &Texts, now_ms: i64) -> Activity {
    let hide = opts.hide_titles;
    let position = if s.playing { s.position + (now_ms - s.at) as f64 / 1000.0 } else { s.position };
    Activity {
        kind: ActivityKind::Watching,
        details: if hide { texts.hide_stream(&s.kind).to_string() } else { s.title.clone() },
        state: if hide { None } else { s.subtitle.clone() },
        image: if hide || !opts.covers { None } else { s.poster.clone() },
        image_text: if hide { None } else { Some(s.title.clone()) },
        icon: Some(if s.playing { Icon::Watching } else { Icon::Pause }),
        icon_text: Some(if s.playing { texts.watching_now.clone() } else { texts.paused.clone() }),
        playing: s.playing,
        position,
        duration: s.duration,
        cover: None,
    }
}

fn from_track(track: &Track, np: &NowPlaying<'_>, opts: &settings::Discord, texts: &Texts) -> Activity {
    let video = track.kind == Kind::Video;
    let playing = np.playing;
    let mut a = Activity {
        kind: if video { ActivityKind::Watching } else { ActivityKind::Listening },
        details: String::new(),
        state: None,
        image: None,
        image_text: None,
        icon: Some(match (playing, video) {
            (false, _) => Icon::Pause,
            (true, true) => Icon::Video,
            (true, false) => Icon::Play,
        }),
        icon_text: Some(match (playing, video) {
            (false, _) => texts.paused.clone(),
            (true, true) => texts.watching_now.clone(),
            (true, false) => texts.listening_now.clone(),
        }),
        playing,
        position: np.position,
        duration: np.duration,
        cover: None,
    };
    if opts.hide_titles {
        a.details = if video { texts.hide_video.clone() } else { texts.hide_music.clone() };
        return a;
    }
    let artist = if track.artist.is_empty() || track.artist == "Unknown" { "" } else { track.artist.as_str() };
    a.details = track.title.clone();
    a.state = Some(if video {
        if track.artist.is_empty() { texts.local_video.clone() } else { track.artist.clone() }
    } else if artist.is_empty() {
        texts.unknown_artist.clone()
    } else {
        artist.to_string()
    });
    a.image_text = (!track.album.is_empty()).then(|| track.album.clone());
    // La pochette du fichier (tag) ou l'image de la vidéo : le client la rend accessible à Discord.
    a.cover = if !opts.covers {
        None
    } else if let Some(hash) = track.cover.as_ref().filter(|h| !h.is_empty()) {
        Some(CoverRef::Cover(hash.clone()))
    } else if video {
        Some(CoverRef::VideoThumb(track.id))
    } else {
        None
    };
    a
}

/// Activité Rich Presence envoyée par `SET_ACTIVITY`. `cover` : adresse publique de la pochette, si on l'a.
pub fn activity_json(a: &Activity, cover: Option<&str>, now_ms: i64) -> Value {
    let mut assets = Map::new();
    assets.insert("large_image".into(), json!(cover.or(a.image.as_deref()).unwrap_or("control")));
    if let Some(t) = &a.image_text {
        assets.insert("large_text".into(), json!(t));
    }
    if let Some(icon) = a.icon {
        assets.insert("small_image".into(), json!(icon.key()));
        if let Some(t) = &a.icon_text {
            assets.insert("small_text".into(), json!(t));
        }
    }
    let mut out = Map::new();
    out.insert("type".into(), json!(a.kind.discord_type()));
    out.insert("details".into(), json!(a.details));
    out.insert("assets".into(), Value::Object(assets));
    out.insert("instance".into(), json!(false));
    // « Écoute <artiste> » / « Regarde <titre> » dans la liste des membres.
    out.insert("status_display_type".into(), json!(if a.kind == ActivityKind::Listening { 1 } else { 2 }));
    if let Some(s) = &a.state {
        out.insert("state".into(), json!(s));
    }
    if a.playing {
        let start = (now_ms as f64 - a.position * 1000.0).round() as i64;
        let ts = if a.duration > a.position {
            json!({ "start": start, "end": (start as f64 + a.duration * 1000.0).round() as i64 })
        } else {
            json!({ "start": start })
        };
        out.insert("timestamps".into(), ts);
    }
    Value::Object(out)
}

/// Suivi côté interface : début de la pause en cours, et envoi seulement d'une vraie différence (hors position)
/// ou d'une position qui s'écarte de plus de 2,5 s de ce que Discord compte déjà.
///
/// L'application appelle [`PresenceFeed::track_changed`] et [`PresenceFeed::playback_changed`] sur les
/// événements du lecteur, puis, environ toutes les 1,5 s pendant la lecture et chaque minute en pause,
/// construit l'activité avec [`build_activity`] (avec `paused_since`) et la passe au client si
/// [`PresenceFeed::should_send`] le dit.
#[derive(Debug, Clone, Default)]
pub struct PresenceFeed {
    paused_since: Option<i64>,
    last_key: Option<String>,
    sent_position: f64,
    sent_at: Option<i64>,
}

impl PresenceFeed {
    pub fn track_changed(&mut self) {
        self.paused_since = None;
    }

    pub fn playback_changed(&mut self, playing: bool, now_ms: i64) {
        self.paused_since = if playing { None } else { Some(self.paused_since.unwrap_or(now_ms)) };
    }

    pub fn paused_since(&self) -> Option<i64> {
        self.paused_since
    }

    /// Les réglages ou la langue ont changé : le prochain envoi part quoi qu'il arrive.
    pub fn invalidate(&mut self) {
        self.last_key = None;
    }

    /// Faut-il transmettre cette activité au client ? (Retient ce qui a été transmis.)
    pub fn should_send(&mut self, activity: Option<&Activity>, now_ms: i64) -> bool {
        let key = match activity {
            Some(a) => {
                let mut v = serde_json::to_value(a).unwrap_or(Value::Null);
                if let Value::Object(m) = &mut v {
                    m.remove("position");
                }
                v.to_string()
            }
            None => "null".into(),
        };
        let position = activity.map(|a| a.position).unwrap_or(0.0);
        let playing = activity.is_some_and(|a| a.playing);
        let drifted = match self.sent_at {
            Some(at) if playing => {
                let expected = self.sent_position + (now_ms - at) as f64 / 1000.0;
                (expected - position).abs() > 2.5
            }
            _ => false,
        };
        if self.last_key.as_deref() == Some(key.as_str()) && !drifted {
            return false;
        }
        self.last_key = Some(key);
        self.sent_position = position;
        self.sent_at = playing.then_some(now_ms);
        true
    }
}

//! Réglages : valeurs par défaut (identiques à la version d'origine) et vue typée.
//!
//! Le fichier garde du JSON libre (une clé inconnue survit à une mise à jour) ; l'application lit la vue typée
//! [`Settings`], où chaque champ manquant ou mal formé prend sa valeur par défaut.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Valeurs par défaut de tous les réglages.
pub fn defaults() -> Value {
    json!({
        // Apparence
        "theme": "auto",
        "accent": "pink",
        "customAccent": "#7c5cff",
        "language": "auto",
        "animations": true,
        "ambient": true,
        "compact": false,
        "scene": "none",
        "sceneSpeed": 100,
        "sceneBlur": 0,
        "radius": 100,
        "uiScale": 100,
        "translucency": 100,
        "oledLevel": 25,
        "customAmbiences": [],

        // Bibliothèque
        "libraryFolders": [],
        "downloadDir": "",

        // Lecture
        "volume": 0.8,
        "muted": false,
        "shuffle": false,
        "repeat": "off",
        "rate": 1,
        "crossfade": 0,
        "automix": false,
        "normalize": false,
        "backgroundPlayback": true,
        "backgroundVideo": true,
        "normalizeTarget": -16,
        "eq": { "enabled": false, "preset": "flat", "gains": [0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
        "session": null,

        // Vues
        "sort": { "songs": { "key": "title", "dir": "asc" }, "albums": "title", "artists": "name", "videos": "addedAt" },
        "albumSize": 190,
        "lastView": { "name": "songs", "param": null },
        "statsRange": "30d",
        "npTab": "queue",
        "immersiveLyrics": true,
        "recapSeen": "",
        "recapSeenWeek": "",

        // Discord
        "discord": {
            "enabled": true, "music": true, "video": true, "streaming": true,
            "covers": true, "hideTitles": false, "clientId": ""
        },

        // Mises à jour
        "update": { "auto": true },

        // Films, séries et animés
        "media": {
            "tmdbKey": "",
            "animeTitles": "romaji",
            "streamBase": "https://embed.vidrift.net",
            "dub": false
        },

        // Téléchargements
        "dl": {
            "kind": "audio",
            "audioFormat": "mp3",
            "audioQuality": "320",
            "videoQuality": "1080",
            "concurrency": 2,
            "cookies": "",
            "cookiesFile": "",
            "searchSource": "soundcloud",
            "createPlaylist": true,
            "autoUpdate": true
        }
    })
}

/// Remplace par leur valeur par défaut les réglages dont le type ne correspond pas (un fichier abîmé ou écrit par
/// une autre version ne doit pas empêcher l'application de démarrer). Les clés inconnues sont gardées.
pub fn sanitize(value: &mut Value, reference: &Value) {
    let Value::Object(refs) = reference else { return };
    let Value::Object(map) = value else {
        *value = reference.clone();
        return;
    };
    for (k, def) in refs {
        let Some(v) = map.get_mut(k) else {
            map.insert(k.clone(), def.clone());
            continue;
        };
        let same = matches!(
            (&*v, def),
            (_, Value::Null)
                | (Value::Bool(_), Value::Bool(_))
                | (Value::Number(_), Value::Number(_))
                | (Value::String(_), Value::String(_))
                | (Value::Array(_), Value::Array(_))
                | (Value::Object(_), Value::Object(_))
        );
        if !same {
            *v = def.clone();
        } else if def.is_object() {
            sanitize(v, def);
        }
    }
}

/// Liste de textes : les éléments d'un autre type sont ignorés.
fn strings<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default())
}

/// Liste de nombres : les éléments d'un autre type valent 0.
fn numbers<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<f64>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()).unwrap_or_default())
}

macro_rules! settings_struct {
    ($(#[$m:meta])* pub struct $name:ident { $( $(#[$fm:meta])* pub $field:ident : $ty:ty = $default:expr ),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase", default)]
        pub struct $name {
            $( $(#[$fm])* pub $field: $ty, )*
        }
        impl Default for $name {
            fn default() -> Self {
                Self { $( $field: $default, )* }
            }
        }
    };
}

settings_struct! {
    /// Égaliseur 10 bandes.
    pub struct Eq {
        pub enabled: bool = false,
        pub preset: String = "flat".into(),
        #[serde(deserialize_with = "numbers")]
        pub gains: Vec<f64> = vec![0.0; 10],
    }
}

settings_struct! {
    pub struct SongSort {
        pub key: String = "title".into(),
        pub dir: String = "asc".into(),
    }
}

settings_struct! {
    pub struct Sort {
        pub songs: SongSort = SongSort::default(),
        pub albums: String = "title".into(),
        pub artists: String = "name".into(),
        pub videos: String = "addedAt".into(),
    }
}

settings_struct! {
    pub struct LastView {
        pub name: String = "songs".into(),
        pub param: Value = Value::Null,
    }
}

settings_struct! {
    pub struct Discord {
        pub enabled: bool = true,
        pub music: bool = true,
        pub video: bool = true,
        pub streaming: bool = true,
        pub covers: bool = true,
        pub hide_titles: bool = false,
        pub client_id: String = String::new(),
    }
}

settings_struct! {
    pub struct Update {
        pub auto: bool = true,
    }
}

settings_struct! {
    pub struct Media {
        pub tmdb_key: String = String::new(),
        pub anime_titles: String = "romaji".into(),
        pub stream_base: String = "https://embed.vidrift.net".into(),
        pub dub: bool = false,
    }
}

settings_struct! {
    pub struct Dl {
        pub kind: String = "audio".into(),
        pub audio_format: String = "mp3".into(),
        pub audio_quality: String = "320".into(),
        pub video_quality: String = "1080".into(),
        pub concurrency: f64 = 2.0,
        pub cookies: String = String::new(),
        pub cookies_file: String = String::new(),
        pub search_source: String = "soundcloud".into(),
        pub create_playlist: bool = true,
        pub auto_update: bool = true,
    }
}

settings_struct! {
    /// Tous les réglages, typés.
    pub struct Settings {
        pub theme: String = "auto".into(),
        pub accent: String = "pink".into(),
        pub custom_accent: String = "#7c5cff".into(),
        pub language: String = "auto".into(),
        pub animations: bool = true,
        pub ambient: bool = true,
        pub compact: bool = false,
        pub scene: String = "none".into(),
        pub scene_speed: f64 = 100.0,
        pub scene_blur: f64 = 0.0,
        pub radius: f64 = 100.0,
        pub ui_scale: f64 = 100.0,
        pub translucency: f64 = 100.0,
        pub oled_level: f64 = 25.0,
        /// Ambiances enregistrées (objets libres : leur forme appartient à l'interface).
        pub custom_ambiences: Vec<Value> = Vec::new(),

        #[serde(deserialize_with = "strings")]
        pub library_folders: Vec<String> = Vec::new(),
        pub download_dir: String = String::new(),

        pub volume: f64 = 0.8,
        pub muted: bool = false,
        pub shuffle: bool = false,
        pub repeat: String = "off".into(),
        pub rate: f64 = 1.0,
        pub crossfade: f64 = 0.0,
        pub automix: bool = false,
        pub normalize: bool = false,
        pub background_playback: bool = true,
        pub background_video: bool = true,
        pub normalize_target: f64 = -16.0,
        pub eq: Eq = Eq::default(),
        pub session: Value = Value::Null,

        pub sort: Sort = Sort::default(),
        pub album_size: f64 = 190.0,
        pub last_view: LastView = LastView::default(),
        pub stats_range: String = "30d".into(),
        pub np_tab: String = "queue".into(),
        pub immersive_lyrics: bool = true,
        pub recap_seen: String = String::new(),
        pub recap_seen_week: String = String::new(),

        pub discord: Discord = Discord::default(),
        pub update: Update = Update::default(),
        pub media: Media = Media::default(),
        pub dl: Dl = Dl::default(),
    }
}

impl Dl {
    /// Téléchargements simultanés (1 à 4).
    pub fn concurrency(&self) -> usize {
        self.concurrency.clamp(1.0, 4.0) as usize
    }
}

impl Settings {
    /// Vue typée d'une valeur JSON complète (valeurs par défaut déjà fusionnées ou non : les manques sont comblés).
    pub fn from_value(mut v: Value) -> Settings {
        sanitize(&mut v, &defaults());
        let mut s: Settings = serde_json::from_value(v).unwrap_or_default();
        if s.eq.gains.len() != 10 || s.eq.gains.iter().any(|g| !g.is_finite()) {
            s.eq.gains = vec![0.0; 10];
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_typed_defaults() {
        assert_eq!(Settings::from_value(defaults()), Settings::default());
    }

    #[test]
    fn bad_values_fall_back() {
        let s = Settings::from_value(json!({
            "volume": "loud", "eq": {"gains": "x", "enabled": true}, "dl": 3,
            "libraryFolders": ["/a", 4, null, "/b"], "customAmbiences": [1, {"x": 2}]
        }));
        assert_eq!(s.library_folders, vec!["/a".to_string(), "/b".to_string()]);
        assert_eq!(s.custom_ambiences.len(), 2);
        assert!((s.volume - 0.8).abs() < 1e-9);
        assert!(s.eq.enabled);
        assert_eq!(s.eq.gains, vec![0.0; 10]);
        assert_eq!(s.dl, Dl::default());
    }
}

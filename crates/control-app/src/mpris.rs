//! Linux : le lecteur sur le bus de session (MPRIS), pour les touches multimédia du clavier et le panneau
//! « en cours de lecture » du bureau (titre, artiste, pochette, position, lecture/pause, précédent/suivant).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use rune::AppHandle;
use zbus::blocking::connection;
use zbus::interface;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use crate::state::Ui;

/// Ce que le bureau voit du lecteur.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MprisState {
    pub playing: bool,
    pub has_track: bool,
    pub track_id: i64,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// `file://` de la pochette.
    pub art_url: Option<String>,
    /// Secondes.
    pub duration: f64,
    pub position: f64,
    pub volume: f64,
}

type Shared = Arc<Mutex<MprisState>>;

struct Root;

#[interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {}
    fn quit(&self) {}
    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> String {
        "control".into()
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        "control".into()
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

struct Player {
    state: Shared,
    handle: AppHandle<Ui>,
}

impl Player {
    fn send(&self, f: impl FnOnce(&mut Ui) + Send + 'static) {
        self.handle.update(move |s, _| f(s));
    }
}

fn track_path(id: i64) -> ObjectPath<'static> {
    ObjectPath::try_from(format!("/org/control/track/{}", id.max(0))).unwrap_or_else(|_| ObjectPath::from_static_str_unchecked("/org/mpris/MediaPlayer2/TrackList/NoTrack"))
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn play_pause(&self) {
        self.send(|s| s.player.toggle());
    }
    fn play(&self) {
        self.send(|s| {
            if !s.player.is_playing() {
                s.player.toggle();
            }
        });
    }
    fn pause(&self) {
        self.send(|s| {
            if s.player.is_playing() {
                s.player.toggle();
            }
        });
    }
    fn stop(&self) {
        self.pause();
    }
    fn next(&self) {
        self.send(|s| {
            let (c, b) = (s.cat.clone(), s.backend.clone());
            s.player.next(&c, &b);
        });
    }
    fn previous(&self) {
        self.send(|s| {
            let (c, b) = (s.cat.clone(), s.backend.clone());
            s.player.prev(&c, &b);
        });
    }
    /// Décalage en microsecondes.
    fn seek(&self, offset: i64) {
        self.send(move |s| {
            let pos = s.player.position();
            s.player.seek(pos + offset as f64 / 1e6);
        });
    }
    fn set_position(&self, _track: ObjectPath<'_>, position: i64) {
        self.send(move |s| s.player.seek(position as f64 / 1e6));
    }
    fn open_uri(&self, _uri: String) {}

    #[zbus(property)]
    fn playback_status(&self) -> String {
        let st = self.state.lock();
        if !st.has_track {
            "Stopped".into()
        } else if st.playing {
            "Playing".into()
        } else {
            "Paused".into()
        }
    }
    #[zbus(property)]
    fn loop_status(&self) -> String {
        "None".into()
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn shuffle(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let st = self.state.lock();
        let mut m: HashMap<String, OwnedValue> = HashMap::new();
        let put = |m: &mut HashMap<String, OwnedValue>, k: &str, v: Value<'_>| {
            if let Ok(v) = OwnedValue::try_from(v) {
                m.insert(k.to_string(), v);
            }
        };
        put(&mut m, "mpris:trackid", Value::from(track_path(st.track_id)));
        if st.has_track {
            put(&mut m, "mpris:length", Value::from((st.duration * 1e6) as i64));
            put(&mut m, "xesam:title", Value::from(st.title.clone()));
            put(&mut m, "xesam:album", Value::from(st.album.clone()));
            put(&mut m, "xesam:artist", Value::from(vec![st.artist.clone()]));
            if let Some(url) = &st.art_url {
                put(&mut m, "mpris:artUrl", Value::from(url.clone()));
            }
        }
        m
    }
    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.state.lock().volume
    }
    #[zbus(property)]
    fn set_volume(&self, v: f64) {
        let v = v.clamp(0.0, 1.0);
        self.send(move |s| s.set(serde_json::json!({ "volume": v, "muted": false })));
    }
    #[zbus(property)]
    fn position(&self) -> i64 {
        (self.state.lock().position * 1e6) as i64
    }
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.state.lock().has_track
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

/// Le service MPRIS ; il disparaît du bus quand la valeur est libérée.
pub struct Mpris {
    conn: connection::Connection,
    state: Shared,
}

impl Mpris {
    pub fn start(handle: AppHandle<Ui>) -> Option<Mpris> {
        let state: Shared = Arc::new(Mutex::new(MprisState::default()));
        let built = connection::Builder::session()
            .and_then(|b| b.name(format!("org.mpris.MediaPlayer2.control.instance{}", std::process::id())))
            .and_then(|b| b.serve_at("/org/mpris/MediaPlayer2", Root))
            .and_then(|b| b.serve_at("/org/mpris/MediaPlayer2", Player { state: state.clone(), handle }))
            .and_then(|b| b.build());
        match built {
            Ok(conn) => Some(Mpris { conn, state }),
            Err(err) => {
                log::info!("MPRIS indisponible (pas de bus de session ?) : {err}");
                None
            }
        }
    }

    /// Publie un nouvel état ; prévient le bureau de ce qui a changé (la position avance d'elle-même).
    pub fn update(&self, next: MprisState) {
        let changed = {
            let mut st = self.state.lock();
            let changed = (st.playing, st.has_track, st.track_id, &st.title, &st.artist, &st.art_url)
                != (next.playing, next.has_track, next.track_id, &next.title, &next.artist, &next.art_url)
                || (st.volume - next.volume).abs() > 1e-3;
            *st = next;
            changed
        };
        if !changed {
            return;
        }
        let server = self.conn.object_server();
        if let Ok(iface) = server.interface::<_, Player>("/org/mpris/MediaPlayer2") {
            let emitter: &SignalEmitter<'_> = iface.signal_emitter();
            let player = iface.get();
            let _ = zbus::block_on(player.playback_status_changed(emitter));
            let _ = zbus::block_on(player.metadata_changed(emitter));
            let _ = zbus::block_on(player.volume_changed(emitter));
            let _ = zbus::block_on(player.can_play_changed(emitter));
        }
    }
}

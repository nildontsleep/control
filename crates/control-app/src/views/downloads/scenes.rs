//! Exemples pour les rendus sans fenêtre (`--scene downloads-…`) : la page dans ses différents états, sans
//! réseau ni yt-dlp.

use rune::TextEdit;
use serde_json::json;

use control_core::downloads::info::{PlaylistInfo, VideoInfo};
use control_core::downloads::{Info, Job, JobStatus, SearchResult};
use control_core::model::Kind;

use super::{Found, Installing, Probe, ToolsInfo};
use crate::state::{Route, Ui};

fn ready_tools() -> ToolsInfo {
    ToolsInfo::parse(&json!({
        "ytdlp": { "installed": true, "version": "2026.10.21", "supported": true },
        "ffmpeg": { "installed": true, "supported": true },
        "deno": { "installed": false, "supported": true },
    }))
}

fn job(id: i64, title: &str, status: JobStatus) -> Job {
    Job {
        id,
        title: title.into(),
        url: format!("https://example.com/{id}"),
        status,
        kind: Kind::Audio,
        ..Job::default()
    }
}

fn demo_jobs() -> Vec<Job> {
    let mut a = job(1, "Daft Punk - Discovery (Full Album)", JobStatus::Downloading);
    a.is_playlist = true;
    a.current = Some("One More Time".into());
    a.index = Some(3);
    a.count = Some(14);
    a.progress = 0.18;
    a.speed = Some(2.4 * 1024.0 * 1024.0);
    a.eta = Some(184.0);
    let mut b = job(2, "Justice - Genesis", JobStatus::Processing);
    b.progress = 1.0;
    let c = job(3, "Kavinsky - Nightcall", JobStatus::Queued);
    let mut d = job(4, "Rick Astley - Never Gonna Give You Up (Official Video)", JobStatus::Done);
    d.kind = Kind::Video;
    d.files = vec!["a.mp4".into()];
    d.track_ids = vec![1];
    d.progress = 1.0;
    let mut e = job(5, "Private upload", JobStatus::Error);
    e.error = Some("ERROR: [youtube] abc: Video unavailable".into());
    let f = job(6, "Air - La femme d'argent", JobStatus::Canceled);
    let mut g = job(7, "Mix 2026", JobStatus::Done);
    g.is_playlist = true;
    g.files = vec!["1.mp3".into(), "2.mp3".into(), "3.mp3".into()];
    g.count = Some(5);
    g.skipped = Some(2);
    g.error = Some("Sign in to confirm you’re not a bot".into());
    g.playlist_id = Some(1);
    g.track_ids = vec![1, 2, 3];
    vec![d, e, f, g, a, b, c]
}

/// Remplit l'état de la page pour la scène `which`.
pub fn apply(ui: &mut Ui, which: &str) {
    ui.route = Route::Downloads;
    let st = &mut ui.downloads;
    // Pas de liste ni d'outils réels : ceux de l'exemple restent.
    st.started.set(true);
    st.tools = Some(ready_tools());
    match which {
        "queue" => {
            st.jobs = demo_jobs();
            st.url = TextEdit::new("https://www.youtube.com/watch?v=dQw4w9WgXcQ");
            st.probe = Probe::Ready(
                Box::new(Info::Video(VideoInfo {
                    title: Some("Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)".into()),
                    uploader: Some("Rick Astley".into()),
                    duration: Some(213.0),
                    thumbnail: None,
                    music: false,
                })),
                "https://www.youtube.com/watch?v=dQw4w9WgXcQ".into(),
            );
        }
        "playlist" => {
            st.url = TextEdit::new("https://soundcloud.com/someone/sets/city-ports");
            st.probe = Probe::Ready(
                Box::new(Info::Playlist(PlaylistInfo {
                    title: "City Ports".into(),
                    uploader: Some("Someone".into()),
                    count: 12,
                    thumbnail: None,
                    entries: Vec::new(),
                    music: true,
                })),
                "https://soundcloud.com/someone/sets/city-ports".into(),
            );
            st.popup = Some("dl-format");
        }
        "search" => {
            st.url = TextEdit::new("nightcall");
            let item = |i: usize, title: &str, who: &str, secs: f64| SearchResult {
                title: Some(title.into()),
                uploader: Some(who.into()),
                duration: Some(secs),
                thumbnail: None,
                url: format!("https://soundcloud.com/x/{i}"),
                source: "soundcloud".into(),
            };
            st.search = Some((
                "nightcall".into(),
                Found::Items(vec![
                    item(1, "Nightcall", "Kavinsky", 258.0),
                    item(2, "Nightcall (London Grammar cover)", "London Grammar", 342.0),
                    item(3, "Nightcall - Drive OST", "Drive Soundtrack", 255.0),
                ]),
            ));
            st.sent = vec!["https://soundcloud.com/x/2".into()];
        }
        "install" => {
            st.tools = Some(ToolsInfo::default());
            st.installing = Some(Installing {
                tool: "ffmpeg".into(),
                phase: "download".into(),
                received: 31_000_000,
                total: 82_000_000,
            });
        }
        "missing" => {
            st.tools = Some(ToolsInfo::default());
            st.url = TextEdit::new("https://youtu.be/x");
            st.probe = Probe::Error(crate::i18n::t("dl.needTools"));
        }
        _ => {}
    }
}

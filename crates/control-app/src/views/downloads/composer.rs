//! Le haut de la page : champ du lien, aperçu de l'analyse ou résultats de recherche, options de format et
//! bouton Télécharger.

use rune::{Cursor, El, TextEdit, TextStyle, col, el, icon, row, text};
use rune_kit::{Kit, Select, Spinner, Switch, TextInput};
use serde_json::json;

use control_core::downloads::{Info, SearchResult};

use super::queue::thumb;
use super::tools::error_color;
use super::{Found, Probe};
use crate::format;
use crate::i18n::{t, tn, tv};
use crate::icons;
use crate::state::Ui;
use crate::theme::Palette;
use crate::ui::common::{button, ty};

/// (valeur, libellé) des listes de l'original.
const AUDIO_FORMATS: &[(&str, &str)] =
    &[("mp3", "MP3"), ("m4a", "M4A (AAC)"), ("opus", "Opus"), ("flac", "FLAC"), ("best", "Original")];
const AUDIO_QUALITIES: &[(&str, &str)] =
    &[("320", "320 kb/s"), ("256", "256 kb/s"), ("192", "192 kb/s"), ("128", "128 kb/s")];
const VIDEO_QUALITIES: &[(&str, &str)] =
    &[("best", "Max"), ("2160", "4K"), ("1440", "1440p"), ("1080", "1080p"), ("720", "720p"), ("480", "480p")];

/// Liste déroulante liée à un réglage : `options` sont des (valeur, libellé) ; `layer` reçoit la liste ouverte
/// (à placer hors de tout parent qui découpe). Son état ouvert est `DownloadsState::popup`.
#[allow(clippy::too_many_arguments)]
pub fn popup(
    ui: &Ui,
    p: &Palette,
    k: &Kit,
    layer: &mut Vec<El>,
    key: &'static str,
    options: Vec<(&'static str, String)>,
    current: &str,
    set: fn(&mut Ui, &str),
) -> El {
    let labels: Vec<&str> = options.iter().map(|(_, l)| l.as_str()).collect();
    let current = options.iter().find(|(v, _)| *v == current).or(options.first()).map(|(_, l)| l.as_str());
    let open = ui.downloads.popup == Some(key);
    let select = Select::new(key, &labels).current(current).open(open);
    let choices = options.clone();
    select
        .on_toggle(move |s: &mut Ui| s.downloads.popup = if s.downloads.popup == Some(key) { None } else { Some(key) })
        .on_close(|s: &mut Ui| s.downloads.popup = None)
        .on_choose(move |s: &mut Ui, label: &str, _| {
            s.downloads.popup = None;
            if let Some((v, _)) = choices.iter().find(|(_, l)| l == label) {
                set(s, v);
            }
        })
        .build(k, layer)
        .fill(p.fill_strong)
        .radius(p.r(7.0))
}

fn listed(options: &'static [(&'static str, &'static str)]) -> Vec<(&'static str, String)> {
    options.iter().map(|(v, l)| (*v, l.to_string())).collect()
}

/// Contrôle segmenté avec icônes (Audio / Vidéo).
fn kind_picker(ui: &Ui, p: &Palette) -> El {
    let current = if ui.settings.dl.kind == "video" { "video" } else { "audio" };
    let mut r = row().key("dl-kind").h(28.0).padding(2.0).gap(2.0).radius(p.r(8.0)).fill(p.fill).role("radiogroup");
    for (value, label, glyph) in [("audio", t("dl.audio"), &icons::MUSIC_2), ("video", t("dl.video"), &icons::FILM)] {
        let on = value == current;
        let mut seg = row()
            .key(format!("dl-kind-{value}"))
            .h_full()
            .px(10.0)
            .gap(5.0)
            .radius(p.r(6.0))
            .center()
            .cursor(Cursor::Pointer)
            .role("radio")
            .checked(on)
            .child(icon(glyph, 14.0).color(if on { p.text } else { p.text2 }))
            .child(text(label).font(ty::SMALL_MEDIUM).color(if on { p.text } else { p.text2 }))
            .on_press(move |s: &mut Ui| s.set(json!({ "dl": { "kind": value } })));
        seg = if on { seg.fill(p.seg_thumb) } else { seg.hover(p.fill) };
        r = r.child(seg);
    }
    r
}

/// Le champ du lien, haut et arrondi, avec le bouton Coller.
fn url_field(ui: &Ui, p: &Palette, k: &Kit) -> El {
    let st = &ui.downloads;
    let focused = st.url.focused;
    let input = TextInput::new(&st.url)
        .plain()
        .width(None)
        .height(50.0)
        .text_style(TextStyle::new(15.0, 400.0))
        .placeholder(t("dl.placeholder"))
        .label(t("nav.downloads"))
        .spellcheck(false)
        .on_edit(|s: &mut Ui| &mut s.downloads.url)
        .on_change(|s: &mut Ui, _| super::url_changed(s))
        .on_submit(|s: &mut Ui, _| super::url_submitted(s))
        .build(k)
        .key("dl-url")
        .grow(1.0)
        .min_w(0.0);
    let paste = row()
        .key("dl-paste")
        .size(38.0, 38.0)
        .radius(p.r(11.0))
        .center()
        .cursor(Cursor::Pointer)
        .hover(p.fill_hover)
        .role("button")
        .label(t("dl.paste"))
        .help(t("dl.paste"))
        .child(icon(&icons::CLIPBOARD_PASTE, 17.0).color(p.text2))
        .on_press_tx(|_s: &mut Ui, tx| {
            tx.read_clipboard(|s: &mut Ui, content, _| {
                let text = content.text().trim().to_string();
                if !text.is_empty() {
                    s.downloads.url = TextEdit::new(text);
                    super::analyze(s);
                }
            });
        });
    let mut field = row()
        .key("dl-url-field")
        .w_full()
        .h(52.0)
        .pl(16.0)
        .pr(8.0)
        .gap(8.0)
        .items_center()
        .radius(p.r(16.0))
        .fill(if focused { p.fill_hover } else { p.fill })
        .child(icon(&icons::LINK_2, 18.0).color(p.text3))
        .child(input)
        .child(paste);
    field = if focused {
        field.border(1.0, p.accent.alpha(0.7)).outline(4.0, 0.0, p.accent.alpha(0.3))
    } else {
        field.border(1.0, p.separator)
    };
    field
}

fn hint(p: &Palette, k: &Kit, body: Result<String, String>) -> El {
    match body {
        Ok(msg) => row()
            .gap(10.0)
            .px(6.0)
            .py(4.0)
            .items_center()
            .child(Spinner::new().size(14.0).color(p.text2).build(k))
            .child(text(msg).font(ty::BODY).color(p.text2)),
        Err(msg) => row()
            .gap(10.0)
            .px(6.0)
            .py(4.0)
            .items_center()
            .child(icon(&icons::CIRCLE_ALERT, 15.0).color(error_color()))
            .child(text(msg).font(ty::BODY).color(error_color()).wrap().shrink().min_w(0.0)),
    }
}

/// Aperçu de ce que le lien contient.
fn info_card(ui: &Ui, p: &Palette, info: &Info) -> El {
    let (kind, title, sub, thumbnail) = match info {
        Info::Video(v) => {
            let sub = [v.uploader.clone(), v.duration.map(format::time)]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            (t("dl.videoKind"), v.title.clone().unwrap_or_default(), sub, v.thumbnail.as_deref())
        }
        Info::Playlist(pl) => (
            tn("dl.playlistKind", pl.count),
            pl.title.clone(),
            pl.uploader.clone().unwrap_or_default(),
            pl.thumbnail.as_deref(),
        ),
    };
    row()
        .key("dl-info")
        .w_full()
        .gap(16.0)
        .padding(12.0)
        .radius(p.r(16.0))
        .fill(p.group)
        .border(1.0, p.separator)
        .child(thumb(ui, p, thumbnail, 128.0, 72.0, 10.0, &icons::MUSIC_2))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(3.0)
                .justify_center()
                .child(text(kind.to_uppercase()).font(TextStyle::new(11.5, 700.0)).color(p.accent))
                .child(text(title).font(TextStyle::new(16.0, 650.0)).color(p.text).max_lines(2))
                .child(text(sub).font(ty::BODY).color(p.text2).truncate()),
        )
}

fn result_row(ui: &Ui, p: &Palette, i: usize, item: &SearchResult) -> El {
    let sub =
        [item.uploader.clone(), item.duration.map(format::time)].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let sent = ui.downloads.sent.contains(&item.url);
    let mut dl = button(p, &format!("dl-res-get-{i}"), &t("dl.download"), Some(&icons::ARROW_DOWN_TO_LINE), false);
    if sent {
        dl = dl.opacity(0.45).cursor(Cursor::Default);
    } else {
        let item = item.clone();
        dl = dl.on_press(move |s: &mut Ui| super::start_result(s, &item));
    }
    let mut r = row()
        .key(format!("dl-res-{i}"))
        .w_full()
        .gap(14.0)
        .px(12.0)
        .py(9.0)
        .items_center()
        .hover(p.fill)
        .child(thumb(ui, p, item.thumbnail.as_deref(), 64.0, 64.0, 9.0, &icons::MUSIC_2))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(item.title.clone().unwrap_or_else(|| "—".into()))
                        .font(ty::BODY_MEDIUM)
                        .color(p.text)
                        .truncate(),
                )
                .child(text(sub).font(ty::SMALL).color(p.text3).truncate()),
        )
        .child(dl);
    if i > 0 {
        r = r.border_sides([1.0, 0.0, 0.0, 0.0], p.separator);
    }
    r
}

/// Résultats de la recherche, sous l'en-tête « Chercher sur [SoundCloud | YouTube] ».
fn results(ui: &Ui, p: &Palette, k: &Kit, found: &Found) -> El {
    let source = super::search_source(ui);
    let head = row()
        .gap(10.0)
        .px(4.0)
        .items_center()
        .child(text(t("dl.searchIn")).font(TextStyle::new(12.5, 600.0)).color(p.text3))
        .child(crate::views::segmented(
            p,
            "dl-source",
            vec![("soundcloud".into(), "SoundCloud".into()), ("youtube".into(), "YouTube".into())],
            source,
            super::set_search_source,
        ));
    let body = match found {
        Found::Loading => hint(p, k, Ok(t("dl.searching"))),
        Found::Error(e) => hint(p, k, Err(e.clone())),
        Found::Items(items) if items.is_empty() => {
            row().px(6.0).py(4.0).child(text(t("dl.noResults")).font(ty::BODY).color(p.text2))
        }
        Found::Items(items) => {
            let mut list = col()
                .key("dl-results")
                .w_full()
                .radius(p.r(16.0))
                .fill(p.group)
                .border(1.0, p.separator)
                .clip_rounded(p.r(16.0));
            for (i, it) in items.iter().enumerate() {
                list = list.child(result_row(ui, p, i, it));
            }
            list
        }
    };
    col().w_full().gap(10.0).child(head).child(body)
}

/// Options de format et bouton Télécharger (absent pendant une recherche : chaque résultat a le sien).
fn options(ui: &Ui, p: &Palette, k: &Kit, layer: &mut Vec<El>, searching: bool) -> El {
    let d = &ui.settings.dl;
    let audio = d.kind != "video";
    let playlist = match (&ui.downloads.probe, searching) {
        (Probe::Ready(info, _), false) => match info.as_ref() {
            Info::Playlist(pl) => Some(pl.count),
            Info::Video(_) => None,
        },
        _ => None,
    };
    let mut r = row().key("dl-options").w_full().gap(10.0).items_center().flex_wrap().child(kind_picker(ui, p));
    if audio {
        r = r.child(popup(ui, p, k, layer, "dl-format", listed(AUDIO_FORMATS), &d.audio_format, |s, v| {
            s.set(json!({ "dl": { "audioFormat": v } }))
        }));
        if d.audio_format == "mp3" {
            r = r.child(popup(ui, p, k, layer, "dl-abr", listed(AUDIO_QUALITIES), &d.audio_quality, |s, v| {
                s.set(json!({ "dl": { "audioQuality": v } }))
            }));
        }
    } else {
        r = r.child(popup(ui, p, k, layer, "dl-vq", listed(VIDEO_QUALITIES), &d.video_quality, |s, v| {
            s.set(json!({ "dl": { "videoQuality": v } }))
        }));
    }
    if playlist.is_some() {
        let on = d.create_playlist;
        r = r.child(
            row()
                .gap(9.0)
                .items_center()
                .child(
                    Switch::new(on)
                        .on_toggle(move |s: &mut Ui| s.set(json!({ "dl": { "createPlaylist": !on } })))
                        .build(k),
                )
                .child(text(t("dl.createPlaylist")).font(ty::BODY).color(p.text2)),
        );
    }
    r = r.child(el().grow(1.0));
    if !searching {
        let label = match playlist {
            Some(n) => tv("dl.downloadAll", &[("n", &n)]),
            None => t("dl.download"),
        };
        r = r.child(button(p, "dl-start", &label, Some(&icons::ARROW_DOWN_TO_LINE), true).on_press(super::start));
    }
    r
}

pub fn composer(ui: &Ui, p: &Palette, k: &Kit, layer: &mut Vec<El>) -> El {
    let st = &ui.downloads;
    let mut c = col().key("dl-composer").w_full().gap(14.0).child(url_field(ui, p, k));
    match &st.probe {
        Probe::Idle => {}
        Probe::Loading => c = c.child(hint(p, k, Ok(t("dl.analyzing")))),
        Probe::Error(e) => c = c.child(hint(p, k, Err(e.clone()))),
        Probe::Ready(info, _) => c = c.child(info_card(ui, p, info)),
    }
    if let Some((_, found)) = &st.search {
        c = c.child(results(ui, p, k, found));
        c = c.child(options(ui, p, k, layer, true));
    } else if matches!(st.probe, Probe::Ready(..)) {
        c = c.child(options(ui, p, k, layer, false));
    }
    c
}

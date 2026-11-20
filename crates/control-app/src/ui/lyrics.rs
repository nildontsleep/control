//! Paroles du morceau en cours : chargement (fichier voisin, tags, LRCLIB), ligne active, panneau.

use rune::{Cursor, El, col, icon, row, text};

use control_core::lrc::Lrc;
use control_core::lyrics::LyricsResult;
use control_core::model::TrackId;

use crate::i18n::t;
use crate::icons;
use crate::state::{Sheet, Ui};
use crate::theme::Palette;
use crate::ui::common::ty;

/// Paroles connues pour un morceau.
#[derive(Debug, Clone, Default)]
pub enum LyricsState {
    #[default]
    None,
    Loading(TrackId),
    Ready { track: TrackId, result: LyricsResult, lrc: Lrc },
    Failed { track: TrackId, message: String, code: Option<&'static str> },
}

impl LyricsState {
    pub fn track(&self) -> Option<TrackId> {
        match self {
            LyricsState::None => None,
            LyricsState::Loading(t) => Some(*t),
            LyricsState::Ready { track, .. } | LyricsState::Failed { track, .. } => Some(*track),
        }
    }

    pub fn lrc(&self) -> Option<&Lrc> {
        match self {
            LyricsState::Ready { lrc, .. } if lrc.has_text() => Some(lrc),
            _ => None,
        }
    }
}

/// Charge les paroles du morceau en cours s'il le faut (`refresh` : refaire la recherche en ligne).
pub fn ensure(ui: &mut Ui, refresh: bool) {
    let Some(id) = ui.player.current_id() else {
        ui.lyrics = LyricsState::None;
        return;
    };
    if !refresh && ui.lyrics.track() == Some(id) {
        return;
    }
    ui.lyrics = LyricsState::Loading(id);
    ui.lyric_line = None;
    ui.spawn(
        move |b| b.lyrics.get(id, refresh).map_err(|e| (e.to_string(), e.code())),
        move |s, res, _| {
            if s.lyrics.track() != Some(id) {
                return;
            }
            s.lyrics = match res {
                Ok(result) => {
                    let lrc = result.parsed();
                    LyricsState::Ready { track: id, result, lrc }
                }
                Err((message, code)) => LyricsState::Failed { track: id, message, code },
            };
        },
    );
}

/// Ligne active à `position` ; `true` si elle a changé.
pub fn follow(ui: &mut Ui, position: f64) -> bool {
    let line = ui.lyrics.lrc().filter(|l| l.synced).and_then(|l| l.line_at(position));
    if line != ui.lyric_line {
        ui.lyric_line = line;
        return true;
    }
    false
}

fn status(p: &Palette, glyph: &'static rune::Icon, title: String, body: Option<String>, actions: Vec<El>) -> El {
    let mut c = col().w_full().py(40.0).gap(8.0).items_center().child(icon(glyph, 30.0).color(p.text3)).child(
        text(title).font(ty::HEADLINE).color(p.text),
    );
    if let Some(b) = body {
        c = c.child(text(b).font(ty::BODY).color(p.text2).wrap().max_w(320.0).align(rune::Align::Center));
    }
    c.child(row().gap(8.0).mt(6.0).children(actions))
}

fn small_button(p: &Palette, key: &str, glyph: &'static rune::Icon, label: String) -> El {
    row()
        .key(key.to_string())
        .h(28.0)
        .px(10.0)
        .gap(6.0)
        .radius(p.r(7.0))
        .items_center()
        .fill(p.fill)
        .hover(p.fill_hover)
        .cursor(Cursor::Pointer)
        .role("button")
        .label(label.clone())
        .child(icon(glyph, 13.0).color(p.text2))
        .child(text(label).font(ty::SMALL_MEDIUM).color(p.text))
}

fn open_editor(s: &mut Ui) {
    let current = match &s.lyrics {
        LyricsState::Ready { result, .. } => result.text().unwrap_or_default().to_string(),
        _ => String::new(),
    };
    s.lyrics_edit = rune::TextEdit::new(current);
    s.lyrics_edit.focused = true;
    s.sheet = Some(Sheet::LyricsEdit);
}

/// Panneau des paroles de « À l'écoute ».
pub fn panel(ui: &Ui, p: &Palette) -> El {
    let refresh = small_button(p, "lyr-refresh", &icons::REFRESH_CW, t("lyr.refresh")).on_press(|s: &mut Ui| ensure(s, true));
    let edit = small_button(p, "lyr-edit", &icons::SQUARE_PEN, t("lyr.edit")).on_press(open_editor);
    match &ui.lyrics {
        LyricsState::None => status(p, &icons::MIC_VOCAL, t("lyr.noTrack"), None, vec![]),
        LyricsState::Loading(_) => status(p, &icons::LOADER, t("lyr.searching"), None, vec![]),
        LyricsState::Failed { message, code, .. } => {
            let body = match *code {
                Some("busy") => t("lyr.errorBusy"),
                Some("network") => t("lyr.errorNetwork"),
                _ => message.clone(),
            };
            status(p, &icons::TRIANGLE_ALERT, t("lyr.errorTitle"), Some(body), vec![refresh])
        }
        LyricsState::Ready { result, lrc, .. } => {
            if result.instrumental {
                return status(p, &icons::AUDIO_LINES, t("lyr.instrumental"), None, vec![edit]);
            }
            if !lrc.has_text() {
                let paste = small_button(p, "lyr-paste", &icons::CLIPBOARD_PASTE, t("lyr.paste")).on_press(open_editor);
                return status(p, &icons::MIC_VOCAL, t("lyr.noneTitle"), Some(t("lyr.noneText")), vec![refresh, paste]);
            }
            let save = small_button(p, "lyr-save", &icons::DOWNLOAD, t("lyr.saveFile")).on_press(|s: &mut Ui| {
                let Some(id) = s.player.current_id() else { return };
                s.spawn(
                    move |b| b.lyrics.save_sidecar(id).map_err(|e| e.to_string()),
                    |s, r, _| match r {
                        Ok(_) => s.toast(t("lyr.saved"), "file-audio"),
                        Err(e) => s.toast(e, "triangle-alert"),
                    },
                );
            });
            let mut lines = col().w_full().gap(14.0).py(10.0);
            for (i, l) in lrc.lines.iter().enumerate() {
                let active = ui.lyric_line == Some(i);
                let past = ui.lyric_line.is_some_and(|a| i < a);
                let label = if l.is_gap() { "• • •".to_string() } else { l.text.clone() };
                let color = if !lrc.synced || active { p.text } else if past { p.text3 } else { p.text2 };
                let mut line = text(label)
                    .key(format!("lyr-{i}"))
                    .font(if active { rune::TextStyle::new(20.0, 700.0) } else { rune::TextStyle::new(18.0, 650.0) })
                    .color(color)
                    .wrap()
                    .w_full();
                if let Some(t0) = l.t {
                    line = line.cursor(Cursor::Pointer).on_press(move |s: &mut Ui| s.player.seek(t0));
                }
                lines = lines.child(line);
            }
            col()
                .w_full()
                .gap(8.0)
                .child(row().gap(6.0).flex_wrap().justify_end().w_full().child(save).child(edit).child(refresh))
                .child(lines)
        }
    }
}

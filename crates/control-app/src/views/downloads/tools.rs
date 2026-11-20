//! Outils yt-dlp, ffmpeg et deno : carte d'installation (avec sa progression), ou ligne « Outils prêts ».

use rune::{Color, Cursor, El, col, el, icon, row, text};
use serde_json::Value;

use crate::format;
use crate::i18n::{t, tv};
use crate::icons;
use crate::state::Ui;
use crate::theme::Palette;
use crate::ui::common::{button, ty};

/// Un outil, tel que `tools:status` le décrit.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolLine {
    pub installed: bool,
    pub version: Option<String>,
    /// Une version prête existe pour ce système.
    pub supported: bool,
}

/// Contenu de `tools:status` (forme JSON d'origine).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolsInfo {
    pub ytdlp: ToolLine,
    pub ffmpeg: ToolLine,
    pub deno: ToolLine,
}

impl ToolsInfo {
    pub fn parse(v: &Value) -> ToolsInfo {
        let line = |k: &str| {
            let x = v.get(k);
            let flag = |f: &str| x.and_then(|x| x.get(f)).and_then(Value::as_bool).unwrap_or(false);
            ToolLine {
                installed: flag("installed"),
                version: x
                    .and_then(|x| x.get("version"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                supported: flag("supported"),
            }
        };
        ToolsInfo { ytdlp: line("ytdlp"), ffmpeg: line("ffmpeg"), deno: line("deno") }
    }

    /// yt-dlp et ffmpeg sont là : les téléchargements peuvent commencer.
    pub fn ready(&self) -> bool {
        self.ytdlp.installed && self.ffmpeg.installed
    }
}

/// Barre de progression fine (téléchargements, installation).
pub fn bar(p: &Palette, frac: Option<f32>) -> El {
    let fill = el().h_full().radius(p.r(2.0)).fill(p.accent);
    let fill = match frac {
        Some(f) => fill.w_frac(f.clamp(0.0, 1.0)),
        // Durée inconnue : barre pleine, atténuée.
        None => fill.w_full().opacity(0.55),
    };
    row().w_full().h(4.0).radius(p.r(2.0)).fill(p.control).clip_rounded(p.r(2.0)).child(fill)
}

/// Petit bouton texte discret, à l'accent (« Mettre à jour yt-dlp »).
pub fn link_button(p: &Palette, key: &str, label: String, glyph: Option<&'static rune::Icon>) -> El {
    let mut b = row()
        .key(key.to_string())
        .h(26.0)
        .px(8.0)
        .gap(5.0)
        .radius(p.r(7.0))
        .items_center()
        .cursor(Cursor::Pointer)
        .role("button")
        .label(label.clone())
        .hover(p.fill);
    if let Some(g) = glyph {
        b = b.child(icon(g, 14.0).color(p.accent));
    }
    b.child(text(label).font(ty::SMALL_MEDIUM).color(p.accent))
}

fn card(p: &Palette, glyph: &'static rune::Icon, body: El, action: Option<El>) -> El {
    let badge =
        row().size(46.0, 46.0).radius(p.r(13.0)).center().fill(p.accent).child(icon(glyph, 22.0).color(p.on_accent));
    let mut c = row()
        .key("dl-tools-card")
        .w_full()
        .gap(16.0)
        .px(18.0)
        .py(16.0)
        .items_center()
        .radius(p.r(16.0))
        .linear_gradient(135f32.to_radians(), [(0.0, p.accent.alpha(0.2)), (1.0, p.accent.alpha(0.06))])
        .border(1.0, p.accent.alpha(0.35))
        .child(badge)
        .child(body.grow(1.0).min_w(0.0));
    if let Some(a) = action {
        c = c.child(a);
    }
    c
}

fn tool_label(key: &str) -> &str {
    match key {
        "ytdlp" => "yt-dlp",
        other => other,
    }
}

/// La boîte des outils : rien tant que leur état n'est pas connu.
pub fn tools_box(ui: &Ui, p: &Palette) -> El {
    let st = &ui.downloads;
    let Some(status) = &st.tools else { return el() };
    let wrap = |e: El| col().w_full().mt(22.0).child(e);
    if let Some(inst) = &st.installing {
        let pct = (inst.total > 0).then(|| inst.received as f32 / inst.total as f32);
        let sub = if inst.phase == "extract" {
            t("tools.extracting")
        } else if inst.total > 0 {
            format!("{} / {}", format::bytes(inst.received as f64), format::bytes(inst.total as f64))
        } else {
            t("tools.preparing")
        };
        let body = col()
            .gap(4.0)
            .child(text(tv("tools.installing", &[("tool", &tool_label(&inst.tool))])).font(ty::HEADLINE).color(p.text))
            .child(text(sub).font(ty::BODY).color(p.text2))
            .child(bar(p, pct).mt(6.0));
        return wrap(card(p, &icons::CLOUD_DOWNLOAD, body, None));
    }
    if !status.ready() {
        let body = col()
            .gap(4.0)
            .child(text(t("tools.needTitle")).font(ty::HEADLINE).color(p.text))
            .child(text(t("tools.needText")).font(ty::BODY).color(p.text2).wrap());
        let install = button(p, "dl-tools-install", &t("tools.install"), Some(&icons::ARROW_DOWN_TO_LINE), true)
            .on_press(super::install_tools);
        return wrap(card(p, &icons::WRENCH, body, Some(install)));
    }
    let mut line = row()
        .key("dl-tools-line")
        .gap(8.0)
        .items_center()
        .child(icon(&icons::CIRCLE_CHECK, 15.0).color(p.success))
        .child(
            text(tv("tools.ready", &[("v", &status.ytdlp.version.clone().unwrap_or_default())]))
                .font(ty::SMALL)
                .color(p.text2),
        );
    if !status.deno.installed && status.deno.supported {
        line = line.child(link_button(p, "dl-tools-deno", t("tools.addDeno"), None).on_press(super::install_tools));
    }
    line = line.child(
        link_button(p, "dl-tools-update", t("tools.update"), Some(&icons::REFRESH_CW)).on_press(super::update_tools),
    );
    wrap(line)
}

/// Couleur des erreurs (`#ff6b61` de l'original).
pub fn error_color() -> Color {
    crate::theme::rgba(255, 107, 97, 1.0)
}

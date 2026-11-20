//! Vue de l'éditeur de tags : pochette et informations à gauche, formulaire (ou outil) à droite, pied avec l'état
//! et les boutons.

use rune::{Color, Cursor, El, SurfaceFit, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, Kit, TextArea, TextInput};

use control_core::tags::CoverSource;

use super::model::{self, CoverChange, CoverView, Field, Scope};
use super::widgets::{Popup, popup_select, small_button};
use super::{IMAGE_EXT, Phase, Tool, pick_cover, request_close, save, stage, tools, undo};
use crate::i18n::{t, tn};
use crate::icons;
use crate::state::Ui;
use crate::theme::Palette;
use crate::ui::common::{CoverKind, cover, ty};

fn label(p: &Palette, s: String, dirty: bool) -> El {
    text(s).font(ty::CAPTION).color(if dirty { p.accent } else { p.text3 })
}

/// Un champ du formulaire (sans son titre).
fn input(ui: &Ui, k: &Kit, f: Field) -> El {
    let st = &ui.tag_editor;
    let i = f.index();
    let (_, mixed) = st.model.field_value(f);
    let placeholder = match (mixed, f.numeric()) {
        (false, _) => String::new(),
        (true, true) => "…".into(),
        (true, false) => t("tag.mixed"),
    };
    let on_change = move |s: &mut Ui, _: &mut Tx| {
        let st = &mut s.tag_editor;
        let mut v = st.fields[i].text.clone();
        if f.numeric() {
            let c = model::clean_number(&v);
            if c != v {
                st.fields[i].replace_text(c.clone());
                v = c;
            }
        }
        st.model.set_field(f, &v);
    };
    let label = t(&format!("tag.f.{}", f.key()));
    match f {
        Field::Comment | Field::Lyrics => TextArea::new(&st.fields[i])
            .width(None)
            .rows(if f == Field::Lyrics { 4 } else { 2 })
            .placeholder(placeholder)
            .label(label)
            .spellcheck(false)
            .on_edit(move |s: &mut Ui| &mut s.tag_editor.fields[i])
            .on_change(on_change)
            .on_submit(|s: &mut Ui, _| save(s))
            .build(k)
            .key(format!("tg-{}", f.key())),
        _ => TextInput::new(&st.fields[i])
            .width(None)
            .large()
            .placeholder(placeholder)
            .label(label)
            .spellcheck(false)
            .on_edit(move |s: &mut Ui| &mut s.tag_editor.fields[i])
            .on_change(on_change)
            .on_key(|s: &mut Ui, ev: &rune::KeyEvent, _: &mut Tx| {
                if ev.key == rune::Key::Enter && ev.mods.cmd {
                    save(s);
                    return true;
                }
                false
            })
            .build(k)
            .key(format!("tg-{}", f.key())),
    }
}

fn field(ui: &Ui, p: &Palette, k: &Kit, f: Field) -> El {
    col()
        .gap(5.0)
        .w_full()
        .min_w(0.0)
        .child(label(p, t(&format!("tag.f.{}", f.key())), ui.tag_editor.model.field_dirty(f)))
        .child(input(ui, k, f))
}

/// « Piste 3 sur 12 ».
fn pair(ui: &Ui, p: &Palette, k: &Kit, title: &str, a: Field, b: Field) -> El {
    let m = &ui.tag_editor.model;
    let dirty = m.field_dirty(a) || m.field_dirty(b);
    col().gap(5.0).child(label(p, t(title), dirty)).child(
        row()
            .gap(6.0)
            .items_center()
            .child(el().w(52.0).child(input(ui, k, a)))
            .child(text(t("tag.of")).font(ty::SMALL).color(p.text3))
            .child(el().w(52.0).child(input(ui, k, b))),
    )
}

fn form(ui: &Ui, p: &Palette, k: &Kit) -> El {
    let half = |f| field(ui, p, k, f).grow(1.0);
    col()
        .w_full()
        .gap(12.0)
        .child(field(ui, p, k, Field::Title))
        .child(row().w_full().gap(10.0).child(half(Field::Artist)).child(half(Field::AlbumArtist)))
        .child(field(ui, p, k, Field::Album))
        .child(
            row()
                .w_full()
                .gap(10.0)
                .items_end()
                .child(half(Field::Genre))
                .child(field(ui, p, k, Field::Year).w(76.0).min_w(76.0).grow(0.0))
                .child(pair(ui, p, k, "tag.f.track", Field::TrackNo, Field::TrackTotal))
                .child(pair(ui, p, k, "tag.f.disc", Field::DiscNo, Field::DiscTotal)),
        )
        .child(field(ui, p, k, Field::Composer))
        .child(field(ui, p, k, Field::Comment))
        .child(field(ui, p, k, Field::Lyrics))
}

fn tools_bar(p: &Palette, k: &Kit) -> El {
    let mut bar = row()
        .w_full()
        .gap(6.0)
        .flex_wrap()
        .items_center()
        .pt(14.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], p.separator)
        .child(text(t("tag.tools")).font(ty::CAPTION).color(p.text3).mr(4.0));
    for (tool, key, glyph) in [
        (Tool::Filename, "filename", &icons::SCAN_TEXT),
        (Tool::Rename, "rename", &icons::FILE_PEN),
        (Tool::Number, "number", &icons::LIST_ORDERED),
        (Tool::Replace, "replace", &icons::REPLACE),
        (Tool::Case, "case", &icons::CASE_SENSITIVE),
    ] {
        bar = bar.child(
            small_button(t(&format!("tag.tool.{key}")), Some(glyph))
                .on_press(move |s: &mut Ui| tools::open_tool(s, tool))
                .build(k)
                .key(format!("tg-tool-{key}")),
        );
    }
    bar.child(
        small_button(t("tag.tool.tidy"), Some(&icons::WAND_SPARKLES))
            .on_press(|s: &mut Ui| {
                let n = s.tag_editor.model.tidy();
                s.tag_editor.sync_fields();
                s.toast(if n > 0 { tn("tag.tidyDone", n) } else { t("tag.tidyNone") }, "wand-sparkles");
            })
            .build(k)
            .key("tg-tool-tidy"),
    )
}

fn cover_box(ui: &Ui, p: &Palette) -> El {
    let st = &ui.tag_editor;
    let (view, mixed) = st.model.cover_view();
    let size = 236.0;
    let r = p.r(12.0);
    let seed = st.model.targets().first().map(|&i| {
        let row = &st.model.rows[i];
        let a = row.value(Field::Album);
        if a.is_empty() { row.value(Field::Title) } else { a }
    });
    let seed = seed.unwrap_or_default();
    let staged = match &view {
        CoverView::Staged(token) => st.staged.get(token).cloned(),
        _ => None,
    };
    let art = match (&view, staged) {
        (_, Some(img)) => el().size(size, size).radius(r).clip_rounded(r).pixels_fit(img.pixels(), SurfaceFit::Fill),
        (CoverView::Hash(h), _) => cover(ui, Some(h), &seed, size, r, CoverKind::Music),
        _ => cover(ui, None, &seed, size, r, CoverKind::Music),
    };
    let badge = if mixed {
        Some(t("tag.coverMixed"))
    } else if view == CoverView::None {
        Some(t("tag.coverNone"))
    } else {
        None
    };
    let mut b = el()
        .key("tg-cover")
        .size(size, size)
        .radius(r)
        .cursor(Cursor::Pointer)
        .role("button")
        .label(t("tag.coverChoose"))
        .shadow(10.0, 30.0, Color::BLACK.alpha(0.28))
        .child(art.opacity(if mixed { 0.75 } else { 1.0 }));
    if let Some(text_) = badge {
        b = b.child(
            row()
                .absolute(8.0, size - 30.0)
                .h(22.0)
                .px(9.0)
                .radius(11.0)
                .items_center()
                .fill(Color::BLACK.alpha(0.55))
                .child(text(text_).font(ty::CAPTION).color(Color::WHITE.alpha(0.92)))
                .inert(),
        );
    }
    if st.drop_over || st.cover_busy {
        let hint = col()
            .absolute(0.0, 0.0)
            .size(size, size)
            .radius(r)
            .center()
            .gap(6.0)
            .fill(Color::BLACK.alpha(0.5))
            .child(icon(&icons::IMAGE_PLUS, 22.0).color(Color::WHITE))
            .child(
                text(t(if st.cover_busy { "tag.reading" } else { "tag.coverDrop" }))
                    .font(ty::SMALL_MEDIUM)
                    .color(Color::WHITE),
            )
            .inert();
        b = b.child(hint);
    }
    if st.drop_over {
        b = b.border(3.0, p.accent);
    }
    b.on_press_tx(|_: &mut Ui, tx| pick_cover(tx))
        .on_drop_files(|s: &mut Ui, files: &[std::path::PathBuf], _| {
            s.tag_editor.drop_over = false;
            if let Some(f) = files.first() {
                stage(s, CoverSource { path: Some(f.to_string_lossy().into_owned()), ..Default::default() }, false);
            }
        })
        .accepts_extensions(IMAGE_EXT)
        .on_drop_hover(|s: &mut Ui, over, _| s.tag_editor.drop_over = over)
}

fn info(ui: &Ui, p: &Palette) -> El {
    let m = &ui.tag_editor.model;
    let list = m.targets();
    let mut lines: Vec<(String, String)> = Vec::new();
    if let [one] = list.as_slice() {
        let r = &m.rows[*one];
        let ext = model::ext_of(&r.path);
        let file = match &r.rename {
            Some(n) => format!("{}{ext}  →  {n}{ext}", r.file_base()),
            None => format!("{}{ext}", r.file_base()),
        };
        let fmt: Vec<String> =
            [r.info.codec.clone(), r.info.bitrate.map(|b| format!("{b} kb/s"))].into_iter().flatten().collect();
        lines.push((t("tag.info.file"), file));
        lines.push((t("tag.info.format"), if fmt.is_empty() { "—".into() } else { fmt.join(" · ") }));
        lines.push((t("tag.info.duration"), crate::format::time(r.info.duration)));
        lines.push((t("tag.info.size"), crate::format::bytes(r.info.size as f64)));
    } else {
        let secs: f64 = list.iter().map(|&i| m.rows[i].info.duration).sum();
        let size: i64 = list.iter().map(|&i| m.rows[i].info.size).sum();
        lines.push((t("tag.info.selection"), tn("count.tracks", list.len())));
        lines.push((t("tag.info.duration"), crate::format::time(secs)));
        lines.push((t("tag.info.size"), crate::format::bytes(size as f64)));
    }
    let mut c = col().w_full().gap(5.0).mt(6.0);
    for (k_, v) in lines {
        c = c.child(
            row()
                .w_full()
                .gap(10.0)
                .items_start()
                .child(text(k_).font(ty::SMALL).color(p.text3).w(56.0))
                .child(text(v).font(ty::SMALL).color(p.text2).wrap_anywhere().grow(1.0).min_w(0.0)),
        );
    }
    c
}

fn left(ui: &Ui, p: &Palette, k: &Kit) -> El {
    let actions = row()
        .w(236.0)
        .gap(6.0)
        .child(
            small_button(t("tag.coverChoose"), None)
                .on_press_tx(|_: &mut Ui, tx| pick_cover(tx))
                .build(k)
                .grow(1.0)
                .key("tg-cover-choose"),
        )
        .child(
            small_button(t("tag.coverSearchShort"), Some(&icons::SEARCH))
                .on_press(|s: &mut Ui| tools::open_tool(s, Tool::Search))
                .build(k)
                .grow(1.0)
                .key("tg-cover-search"),
        )
        .child(
            IconButton::new(&icons::TRASH_2)
                .control_size(ControlSize::Small)
                .label(t("tag.coverRemove"))
                .help(t("tag.coverRemove"))
                .on_press(|s: &mut Ui| s.tag_editor.model.set_cover(Some(CoverChange::Remove)))
                .build(k)
                .key("tg-cover-remove"),
        );
    col().w(236.0).gap(12.0).child(cover_box(ui, p)).child(actions).child(info(ui, p))
}

/// Précédent, liste des morceaux, suivant.
fn scope_bar(ui: &Ui, k: &Kit, layer: &mut Vec<El>) -> El {
    let m = &ui.tag_editor.model;
    let n = m.rows.len();
    let mut options = vec![tn("tag.all", n)];
    options.extend(m.rows.iter().enumerate().map(|(i, r)| format!("{}. {}", i + 1, r.title())));
    let current = match m.scope {
        Scope::All => 0,
        Scope::One(i) => i + 1,
    };
    let (prev_ok, next_ok) = match m.scope {
        Scope::All => (false, false),
        Scope::One(i) => (i > 0, i + 1 < n),
    };
    let open = ui.tag_editor.popup.as_deref() == Some("tg-scope");
    let spec = Popup { key: "tg-scope", options, current: Some(current), width: 380.0, placeholder: String::new() };
    let select = popup_select(
        k,
        layer,
        open,
        spec,
        |s| &mut s.tag_editor.popup,
        |s, i| {
            s.tag_editor.model.scope = if i == 0 { Scope::All } else { Scope::One(i - 1) };
            s.tag_editor.sync_fields();
        },
    );
    let step = |delta: isize| {
        move |s: &mut Ui| {
            s.tag_editor.model.step(delta);
            s.tag_editor.sync_fields();
        }
    };
    row()
        .gap(6.0)
        .items_center()
        .mb(14.0)
        .child(
            IconButton::new(&icons::CHEVRON_LEFT)
                .label(t("tag.prev"))
                .help(t("tag.prev"))
                .disabled(!prev_ok)
                .on_press(step(-1))
                .build(k)
                .key("tg-prev"),
        )
        .child(select)
        .child(
            IconButton::new(&icons::CHEVRON_RIGHT)
                .label(t("tag.next"))
                .help(t("tag.next"))
                .disabled(!next_ok)
                .on_press(step(1))
                .build(k)
                .key("tg-next"),
        )
}

pub(super) fn footer(ui: &Ui, p: &Palette, k: &Kit) -> El {
    let st = &ui.tag_editor;
    if st.confirm_discard {
        return row()
            .w_full()
            .gap(8.0)
            .items_center()
            .child(text(t("tag.discardTitle")).font(ty::BODY_MEDIUM).color(p.danger).grow(1.0))
            .child(
                Button::new(t("tag.keepEditing"))
                    .on_press(|s: &mut Ui| s.tag_editor.confirm_discard = false)
                    .build(k)
                    .key("tg-keep"),
            )
            .child(
                Button::new(t("tag.discard"))
                    .variant(ButtonVariant::Danger)
                    .on_press(|s: &mut Ui| {
                        s.tag_editor.confirm_discard = false;
                        s.sheet = None;
                    })
                    .build(k)
                    .key("tg-discard"),
            );
    }
    let ready = st.phase == Phase::Ready;
    let n = st.model.dirty_count();
    let mut parts = vec![if n > 0 { tn("tag.status", n) } else { t("tag.statusNone") }];
    if st.skipped > 0 {
        parts.push(tn("tag.skipped", st.skipped));
    }
    let mut status = row().grow(1.0).min_w(0.0).gap(4.0).items_center();
    if ready {
        status = status.child(text(parts.join(" · ")).font(ty::BODY).color(p.text2));
    }
    let failed = st.model.failed();
    if let Some(first) = failed.first() {
        let more = if failed.len() > 1 { "…" } else { "" };
        let msg = format!(" — {} : {}{more}", tn("tag.failed", failed.len()), first.error.clone().unwrap_or_default());
        status = status.child(text(msg).font(ty::BODY).color(p.danger).truncate().shrink().min_w(0.0));
    }
    let mut out = row().w_full().gap(8.0).items_center();
    if st.can_undo {
        out = out.child(
            Button::new(t("tag.undoLast"))
                .icon(&icons::UNDO_2)
                .variant(ButtonVariant::Ghost)
                .on_press(undo)
                .build(k)
                .key("tg-undo"),
        );
    }
    let save_label = if st.saving {
        t("tag.saving")
    } else if n > 1 {
        tn("tag.saveN", n)
    } else {
        t("tag.save")
    };
    out.child(status).child(Button::new(t("common.cancel")).on_press(request_close).build(k).key("tg-cancel")).child(
        Button::new(save_label)
            .variant(ButtonVariant::Contrast)
            .disabled(st.saving || n == 0 || !ready)
            .on_press(save)
            .build(k)
            .key("tg-save"),
    )
}

pub(super) fn body(ui: &Ui, p: &Palette, k: &Kit, layer: &mut Vec<El>) -> El {
    let st = &ui.tag_editor;
    match &st.phase {
        Phase::Loading => {
            return col()
                .w_full()
                .h(300.0)
                .center()
                .gap(10.0)
                .child(rune_kit::Spinner::new().build(k))
                .child(text(t("tag.reading")).font(ty::BODY).color(p.text2));
        }
        Phase::Nothing(msg) => {
            return col()
                .w_full()
                .h(260.0)
                .center()
                .gap(8.0)
                .child(icon(&icons::TAGS, 28.0).color(p.text3))
                .child(text(t("tag.nothingEditable")).font(ty::BODY_MEDIUM).color(p.text))
                .child(text(msg.clone()).font(ty::SMALL).color(p.text3));
        }
        Phase::Ready => {}
    }
    let mut right = col().grow(1.0).min_w(0.0);
    if st.model.rows.len() > 1 {
        right = right.child(scope_bar(ui, k, layer));
    }
    right = match st.tool {
        Some(tool) => right.child(tools::panel(ui, p, k, layer, tool)),
        None => right.child(form(ui, p, k)).child(tools_bar(p, k).mt(16.0)),
    };
    row().w_full().gap(22.0).items_start().child(left(ui, p, k)).child(right)
}

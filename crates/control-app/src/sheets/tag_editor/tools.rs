//! Outils par lot de l'éditeur de tags (tags depuis le nom, renommage, numérotation, remplacement, casse) et
//! recherche de pochettes en ligne. Chaque outil montre un aperçu ; « Appliquer » le reporte dans le formulaire,
//! rien n'est écrit avant « Enregistrer ».

use rune::{Color, Cursor, El, SurfaceFit, TextEdit, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Checkbox, Kit, TextInput};

use control_core::tagpattern::{CaseMode, PATTERN_PRESETS, ReplaceOptions};
use control_core::tags::CoverSource;

use super::model::{self, Field};
use super::widgets::{Popup, popup_select, small_button};
use super::{Hit, Search, Tool, stage};
use crate::i18n::{t, tn, tv};
use crate::icons;
use crate::state::Ui;
use crate::theme::Palette;
use crate::ui::common::ty;

const CASES: [(CaseMode, &str); 4] = [
    (CaseMode::Title, "title"),
    (CaseMode::Sentence, "sentence"),
    (CaseMode::Lower, "lower"),
    (CaseMode::Upper, "upper"),
];

/// Lignes montrées au plus dans un aperçu.
const PREVIEW_MAX: usize = 300;

pub(super) fn open_tool(s: &mut Ui, tool: Tool) {
    let st = &mut s.tag_editor;
    st.tool = Some(tool);
    st.popup = None;
    match tool {
        Tool::Replace => st.find.focused = true,
        Tool::Search => {
            let (artist, album) = match st.model.targets().first() {
                Some(&i) => {
                    let r = &st.model.rows[i];
                    let a = r.value(Field::AlbumArtist);
                    (if a.is_empty() { r.value(Field::Artist) } else { a }, r.value(Field::Album))
                }
                None => Default::default(),
            };
            let go = !artist.is_empty() || !album.is_empty();
            st.search_artist.replace_text(artist);
            st.search_album.replace_text(album);
            st.search = Search::Idle;
            if go {
                run_search(s);
            }
        }
        _ => {}
    }
}

fn close_tool(s: &mut Ui) {
    let st = &mut s.tag_editor;
    st.tool = None;
    st.popup = None;
    st.sync_fields();
}

/// Cherche des pochettes (Apple Music et Deezer), puis télécharge leurs aperçus un par un.
fn run_search(s: &mut Ui) {
    let st = &mut s.tag_editor;
    st.search_epoch += 1;
    st.search = Search::Loading;
    let g = st.search_epoch;
    let (artist, album) = (st.search_artist.text.clone(), st.search_album.text.clone());
    let dir = super::search_dir(s);
    s.spawn(
        move |b| b.tags.search_covers(&artist, &album),
        move |s, res, _| {
            if s.tag_editor.search_epoch != g {
                return;
            }
            let hits = match res {
                Ok(list) => list,
                Err(err) => {
                    log::warn!("Recherche de pochettes : {err}");
                    s.tag_editor.search = Search::Failed;
                    return;
                }
            };
            let urls: Vec<String> = hits.iter().map(|h| h.preview.clone()).collect();
            s.tag_editor.search = Search::Done(hits.into_iter().map(|hit| Hit { hit, image: None }).collect());
            for (i, url) in urls.into_iter().enumerate() {
                let path = dir.join(format!("{g}-{i}.jpg"));
                s.spawn(
                    move |_| control_core::tools::download(&url, &path, |_, _| {}).map(|()| path),
                    move |s, res, _| {
                        let st = &mut s.tag_editor;
                        let (Ok(path), true) = (res, st.search_epoch == g) else { return };
                        let scale = s.scale;
                        if let (Search::Done(hits), Some(images)) = (&mut st.search, &st.images)
                            && let Some(h) = hits.get_mut(i)
                        {
                            h.image = Some(images.file(path, 124.0, scale));
                        }
                    },
                );
            }
        },
    );
}

fn head(p: &Palette, k: &Kit, title: String, hint: Option<String>) -> El {
    let mut words = col().gap(3.0).grow(1.0).min_w(0.0).child(text(title).font(ty::HEADLINE).color(p.text));
    if let Some(h) = hint {
        words = words.child(text(h).font(ty::SMALL).color(p.text2).wrap());
    }
    row()
        .w_full()
        .gap(14.0)
        .items_start()
        .child(small_button(t("tag.back"), Some(&icons::ARROW_LEFT)).on_press(close_tool).build(k).key("tg-back"))
        .child(words)
}

/// Aperçu en deux colonnes ; une ligne « atténuée » ne changera pas.
fn table(p: &Palette, head: [String; 2], rows: Vec<(bool, [String; 2])>) -> El {
    if rows.is_empty() {
        return text(t("tag.nothing")).font(ty::BODY).color(p.text3).py(10.0);
    }
    let cell = |s: String, color: Color, weight| text(s).font(weight).color(color).truncate().grow(1.0).min_w(0.0);
    let line = |cells: [El; 2]| {
        let [a, b] = cells;
        row()
            .w_full()
            .gap(14.0)
            .px(12.0)
            .py(6.0)
            .border_sides([1.0, 0.0, 0.0, 0.0], p.separator)
            .child(a)
            .child(b.grow(1.25))
    };
    let [h1, h2] = head;
    let mut list = col().w_full();
    let n = rows.len();
    for (dim, [a, b]) in rows.into_iter().take(PREVIEW_MAX) {
        let c = if dim { p.text3 } else { p.text };
        list = list.child(line([cell(a, c, ty::SMALL), cell(b, c, ty::SMALL)]));
    }
    if n > PREVIEW_MAX {
        list = list.child(
            row()
                .w_full()
                .py(6.0)
                .justify_center()
                .child(text(format!("+ {}", n - PREVIEW_MAX)).font(ty::SMALL).color(p.text3)),
        );
    }
    col()
        .w_full()
        .radius(p.r(10.0))
        .border(1.0, p.separator)
        .clip_rounded(p.r(10.0))
        .child(
            row()
                .w_full()
                .gap(14.0)
                .px(12.0)
                .py(7.0)
                .fill(p.fill)
                .child(cell(h1, p.text3, ty::CAPTION))
                .child(cell(h2, p.text3, ty::CAPTION).grow(1.25)),
        )
        .child(
            col()
                .key("tg-preview")
                .w_full()
                .max_h(290.0)
                .scroll(rune::ScrollSpec { stick_bottom: false })
                .child(list.border_sides([0.0; 4], p.separator)),
        )
}

fn foot(k: &Kit, enabled: bool, apply: impl Fn(&mut Ui) + 'static) -> El {
    row().w_full().justify_end().child(
        Button::new(t("tag.apply"))
            .variant(ButtonVariant::Contrast)
            .disabled(!enabled)
            .on_press(move |s: &mut Ui| {
                apply(s);
                close_tool(s);
            })
            .build(k)
            .key("tg-apply"),
    )
}

fn summary(p: &Palette, s: String) -> El {
    text(s).font(ty::SMALL_MEDIUM).color(p.text2)
}

fn line_input(edit: &TextEdit, placeholder: String, access: fn(&mut Ui) -> &mut TextEdit) -> TextInput {
    TextInput::new(edit).width(None).placeholder(placeholder).spellcheck(false).on_edit(move |s: &mut Ui| access(s))
}

/// Motif et menu des modèles.
fn pattern(ui: &Ui, k: &Kit, layer: &mut Vec<El>, key: &str, access: fn(&mut Ui) -> &mut TextEdit) -> El {
    let edit = match key {
        "rename" => &ui.tag_editor.pat_rename,
        _ => &ui.tag_editor.pat_filename,
    };
    let current = PATTERN_PRESETS.iter().position(|p| *p == edit.text);
    let input =
        line_input(edit, String::new(), access).text_style(ty::NUMBERS).build(k).key(format!("tg-pattern-{key}"));
    let popup_key = format!("tg-presets-{key}");
    let open = ui.tag_editor.popup.as_deref() == Some(popup_key.as_str());
    let presets = popup_select(
        k,
        layer,
        open,
        Popup {
            key: &popup_key,
            options: PATTERN_PRESETS.iter().map(|p| p.to_string()).collect(),
            current,
            width: 200.0,
            placeholder: t("tag.presets"),
        },
        |s| &mut s.tag_editor.popup,
        move |s, i| access(s).replace_text(PATTERN_PRESETS[i]),
    );
    row().w_full().gap(8.0).items_center().child(input.grow(1.0).min_w(0.0)).child(presets)
}

fn filename(ui: &Ui, p: &Palette, k: &Kit, layer: &mut Vec<El>) -> El {
    let st = &ui.tag_editor;
    let plan = model::plan_filename(&st.model, &st.pat_filename.text);
    let ok = plan.iter().filter(|x| x.parsed.is_some()).count();
    let rows = plan
        .iter()
        .map(|x| {
            let result = match &x.parsed {
                Some(fields) => fields
                    .iter()
                    .map(|(f, v)| format!("{}: {v}", t(&format!("tag.short.{}", f.key()))))
                    .collect::<Vec<_>>()
                    .join("  ·  "),
                None => t("tag.noMatch"),
            };
            (x.parsed.is_none(), [x.name.clone(), result])
        })
        .collect();
    col()
        .w_full()
        .gap(12.0)
        .child(head(p, k, t("tag.tool.filename"), Some(t("tag.patternHint"))))
        .child(pattern(ui, k, layer, "filename", |s| &mut s.tag_editor.pat_filename))
        .child(summary(p, tv("tag.matches", &[("ok", &ok), ("n", &plan.len())])))
        .child(table(p, [t("tag.info.file"), t("tag.result")], rows))
        .child(foot(k, ok > 0, |s| {
            let st = &mut s.tag_editor;
            let plan = model::plan_filename(&st.model, &st.pat_filename.text);
            model::apply_filename(&mut st.model, &plan);
        }))
}

fn rename(ui: &Ui, p: &Palette, k: &Kit, layer: &mut Vec<El>) -> El {
    let st = &ui.tag_editor;
    let plan = model::plan_rename(&st.model, &st.pat_rename.text);
    let changed = plan.iter().filter(|x| x.changes()).count();
    let rows = plan
        .iter()
        .map(|x| {
            let ext = model::ext_of(&st.model.rows[x.row].path);
            let next = if x.next.is_empty() {
                t("tag.emptyName")
            } else {
                format!("{}{ext}{}", x.next, if x.duplicate { " ⚠" } else { "" })
            };
            (!x.changes(), [format!("{}{ext}", x.current), next])
        })
        .collect();
    col()
        .w_full()
        .gap(12.0)
        .child(head(p, k, t("tag.tool.rename"), Some(t("tag.renameHint"))))
        .child(pattern(ui, k, layer, "rename", |s| &mut s.tag_editor.pat_rename))
        .child(summary(p, tn("tag.renameCount", changed)))
        .child(table(p, [t("tag.current"), t("tag.newName")], rows))
        .child(foot(k, changed > 0, |s| {
            let st = &mut s.tag_editor;
            let plan = model::plan_rename(&st.model, &st.pat_rename.text);
            model::apply_rename(&mut st.model, &plan);
        }))
}

fn number_start(s: &Ui) -> u32 {
    s.tag_editor.num_start.text.parse::<u32>().unwrap_or(0).min(999)
}

fn number(ui: &Ui, p: &Palette, k: &Kit) -> El {
    let st = &ui.tag_editor;
    let plan = model::plan_number(&st.model, number_start(ui), st.num_total);
    let rows = plan
        .iter()
        .map(|x| {
            let n = match x.total {
                Some(t) => format!("{} / {t}", x.track_no),
                None => x.track_no.to_string(),
            };
            (false, [st.model.rows[x.row].title(), n])
        })
        .collect();
    let start = line_input(&st.num_start, String::new(), |s| &mut s.tag_editor.num_start)
        .on_change(|s: &mut Ui, _: &mut Tx| {
            let e = &mut s.tag_editor.num_start;
            let clean: String = model::clean_number(&e.text).chars().take(3).collect();
            if clean != e.text {
                e.replace_text(clean);
            }
        })
        .build(k)
        .key("tg-num-start");
    let total = Checkbox::new(st.num_total)
        .label(t("tag.setTotal"))
        .on_toggle(|s: &mut Ui| s.tag_editor.num_total = !s.tag_editor.num_total)
        .build(k)
        .key("tg-num-total");
    col()
        .w_full()
        .gap(12.0)
        .child(head(p, k, t("tag.tool.number"), Some(t("tag.numberHint"))))
        .child(
            row()
                .gap(16.0)
                .items_end()
                .child(
                    col()
                        .gap(5.0)
                        .child(text(t("tag.start")).font(ty::CAPTION).color(p.text3))
                        .child(el().w(76.0).child(start)),
                )
                .child(total.mb(6.0)),
        )
        .child(summary(p, tn("count.tracks", plan.len())))
        .child(table(p, [t("tag.f.title"), t("tag.f.track")], rows))
        .child(foot(k, !plan.is_empty(), |s| {
            let start = number_start(s);
            let st = &mut s.tag_editor;
            let plan = model::plan_number(&st.model, start, st.num_total);
            model::apply_number(&mut st.model, &plan);
        }))
}

/// Champ visé par « Remplacer » ou « Casse » (`None` : tous les champs texte).
fn field_select(
    ui: &Ui,
    k: &Kit,
    layer: &mut Vec<El>,
    key: &str,
    current: Option<Field>,
    set: fn(&mut Ui, Option<Field>),
) -> El {
    let mut options = vec![t("tag.allText")];
    options.extend(Field::TEXT.iter().map(|f| t(&format!("tag.f.{}", f.key()))));
    let cur = current.and_then(|f| Field::TEXT.iter().position(|x| *x == f)).map_or(0, |i| i + 1);
    let open = ui.tag_editor.popup.as_deref() == Some(key);
    popup_select(
        k,
        layer,
        open,
        Popup { key, options, current: Some(cur), width: 220.0, placeholder: String::new() },
        |s| &mut s.tag_editor.popup,
        move |s, i| set(s, if i == 0 { None } else { Some(Field::TEXT[i - 1]) }),
    )
}

fn opt_row(p: &Palette, label: String, control: El) -> El {
    row().gap(10.0).items_center().child(text(label).font(ty::CAPTION).color(p.text3).w(64.0)).child(control)
}

fn changes_table(p: &Palette, plan: &[model::Change]) -> El {
    let rows = plan.iter().map(|c| (false, [c.before.clone(), c.after.clone()])).collect();
    table(p, [t("tag.current"), t("tag.result")], rows)
}

fn replace_options(s: &Ui) -> ReplaceOptions {
    let st = &s.tag_editor;
    ReplaceOptions {
        find: st.find.text.clone(),
        replace: st.replace_with.text.clone(),
        regex: st.regex,
        match_case: st.match_case,
    }
}

fn replace(ui: &Ui, p: &Palette, k: &Kit, layer: &mut Vec<El>) -> El {
    let st = &ui.tag_editor;
    let plan = model::plan_replace(&st.model, st.replace_field, &replace_options(ui));
    let field = field_select(ui, k, layer, "tg-replace-field", st.replace_field, |s, f| s.tag_editor.replace_field = f);
    let find = line_input(&st.find, t("tag.find"), |s| &mut s.tag_editor.find).build(k).key("tg-find");
    let with = line_input(&st.replace_with, t("tag.replaceWith"), |s| &mut s.tag_editor.replace_with)
        .build(k)
        .key("tg-replace-with");
    let checks = row()
        .gap(18.0)
        .child(
            Checkbox::new(st.match_case)
                .label(t("tag.matchCase"))
                .on_toggle(|s: &mut Ui| s.tag_editor.match_case = !s.tag_editor.match_case)
                .build(k),
        )
        .child(
            Checkbox::new(st.regex)
                .label(t("tag.regex"))
                .on_toggle(|s: &mut Ui| s.tag_editor.regex = !s.tag_editor.regex)
                .build(k),
        );
    col()
        .w_full()
        .gap(12.0)
        .child(head(p, k, t("tag.tool.replace"), Some(t("tag.replaceHint"))))
        .child(opt_row(p, t("tag.field"), field))
        .child(find)
        .child(with)
        .child(checks)
        .child(summary(p, tn("tag.changes", plan.len())))
        .child(changes_table(p, &plan))
        .child(foot(k, !plan.is_empty(), |s| {
            let o = replace_options(s);
            let st = &mut s.tag_editor;
            let plan = model::plan_replace(&st.model, st.replace_field, &o);
            model::apply_changes(&mut st.model, &plan);
        }))
}

fn case(ui: &Ui, p: &Palette, k: &Kit, layer: &mut Vec<El>) -> El {
    let st = &ui.tag_editor;
    let plan = model::plan_case(&st.model, st.case_field, st.case_mode);
    let field = field_select(ui, k, layer, "tg-case-field", st.case_field, |s, f| s.tag_editor.case_field = f);
    let open = st.popup.as_deref() == Some("tg-case-mode");
    let mode = popup_select(
        k,
        layer,
        open,
        Popup {
            key: "tg-case-mode",
            options: CASES.iter().map(|(_, n)| t(&format!("tag.case.{n}"))).collect(),
            current: CASES.iter().position(|(m, _)| *m == st.case_mode),
            width: 220.0,
            placeholder: String::new(),
        },
        |s| &mut s.tag_editor.popup,
        |s, i| s.tag_editor.case_mode = CASES[i].0,
    );
    col()
        .w_full()
        .gap(12.0)
        .child(head(p, k, t("tag.tool.case"), Some(t("tag.caseHint"))))
        .child(opt_row(p, t("tag.field"), field))
        .child(opt_row(p, t("tag.tool.case"), mode))
        .child(summary(p, tn("tag.changes", plan.len())))
        .child(changes_table(p, &plan))
        .child(foot(k, !plan.is_empty(), |s| {
            let st = &mut s.tag_editor;
            let plan = model::plan_case(&st.model, st.case_field, st.case_mode);
            model::apply_changes(&mut st.model, &plan);
        }))
}

fn result_card(p: &Palette, i: usize, h: &Hit) -> El {
    let r = p.r(8.0);
    let art = match &h.image {
        Some(img) => el().size(124.0, 124.0).radius(r).clip_rounded(r).pixels_fit(img.pixels(), SurfaceFit::Fill),
        None => el().size(124.0, 124.0).radius(r).fill(p.fill_strong),
    };
    let year = h.hit.year.map(|y| y.to_string());
    let sub: Vec<&str> = [Some(h.hit.artist.as_str()), year.as_deref(), Some(h.hit.source.as_str())]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    let full = h.hit.full.clone();
    col()
        .key(format!("tg-hit-{i}"))
        .w(124.0)
        .gap(4.0)
        .cursor(Cursor::Pointer)
        .role("button")
        .label(format!("{} — {}", h.hit.title, h.hit.artist))
        .help(format!("{} — {}", h.hit.title, h.hit.artist))
        .child(art)
        .child(text(h.hit.title.clone()).font(ty::SMALL_MEDIUM).color(p.text).truncate())
        .child(text(sub.join(" · ")).font(ty::SMALL).color(p.text3).truncate())
        .on_press(move |s: &mut Ui| stage(s, CoverSource { url: Some(full.clone()), ..Default::default() }, true))
}

fn search(ui: &Ui, p: &Palette, k: &Kit) -> El {
    let st = &ui.tag_editor;
    let submit = |s: &mut Ui, _: &mut Tx| run_search(s);
    let artist = line_input(&st.search_artist, t("tag.f.artist"), |s| &mut s.tag_editor.search_artist)
        .on_submit(submit)
        .build(k)
        .key("tg-search-artist");
    let album = line_input(&st.search_album, t("tag.f.album"), |s| &mut s.tag_editor.search_album)
        .on_submit(submit)
        .build(k)
        .key("tg-search-album");
    let go = Button::new(t("tag.search"))
        .icon(&icons::SEARCH)
        .variant(ButtonVariant::Contrast)
        .on_press(run_search)
        .build(k)
        .key("tg-search-go");
    let results = match &st.search {
        Search::Idle => el(),
        Search::Loading => row()
            .w_full()
            .py(30.0)
            .gap(10.0)
            .justify_center()
            .items_center()
            .child(rune_kit::Spinner::new().build(k))
            .child(text(t("tag.searching")).font(ty::BODY).color(p.text2)),
        Search::Failed => text(t("tag.searchError")).font(ty::BODY).color(p.text3).py(20.0),
        Search::Done(hits) if hits.is_empty() => text(t("tag.searchEmpty")).font(ty::BODY).color(p.text3).py(20.0),
        Search::Done(hits) => {
            let mut grid = row().w_full().gap(12.0).flex_wrap();
            for (i, h) in hits.iter().enumerate() {
                grid = grid.child(result_card(p, i, h));
            }
            grid.opacity(if st.cover_busy { 0.6 } else { 1.0 })
        }
    };
    col()
        .w_full()
        .gap(12.0)
        .child(head(p, k, t("tag.coverSearch"), Some(t("tag.searchHint"))))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .items_center()
                .child(artist.grow(1.0).min_w(0.0))
                .child(album.grow(1.0).min_w(0.0))
                .child(go),
        )
        .child(results)
}

pub(super) fn panel(ui: &Ui, p: &Palette, k: &Kit, layer: &mut Vec<El>, tool: Tool) -> El {
    let body = match tool {
        Tool::Filename => filename(ui, p, k, layer),
        Tool::Rename => rename(ui, p, k, layer),
        Tool::Number => number(ui, p, k),
        Tool::Replace => replace(ui, p, k, layer),
        Tool::Case => case(ui, p, k, layer),
        Tool::Search => search(ui, p, k),
    };
    body.key("tg-tool")
}

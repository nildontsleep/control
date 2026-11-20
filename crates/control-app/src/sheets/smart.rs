//! Playlists intelligentes (porté de `src/renderer/js/ui/smart.js`) : règles, tri, limite, modèles et nombre de
//! morceaux trouvés en direct ; et personnalisation d'une playlist (`customize`, porté de `ui/playlist-edit.js`).

mod customize;

use std::cell::Cell;

use rune::{Cursor, El, TextEdit, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, IconButton, Kit, TextInput};
use serde_json::{Value, json};

use control_core::model::{PlaylistId, PlaylistKind, SmartRule, SmartSpec};
use control_core::smart::{self, FieldType};

use super::tag_editor::widgets::{Popup, card, popup_select, small_button};
use crate::i18n::{t, tn};
use crate::icons;
use crate::state::{Route, Sheet, Ui};
use crate::theme::Palette;
use crate::ui::common::ty;

pub use customize::open_customize;

/// Scènes de rendu : ce qui s'ouvre une fois la bibliothèque chargée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmartScene {
    New,
    /// Règles de la première playlist intelligente.
    EditFirst,
    /// Personnalisation de la première playlist.
    CustomizeFirst,
}

pub struct SmartState {
    /// Playlist modifiée (`None` : nouvelle playlist intelligente).
    pub playlist: Option<PlaylistId>,
    pub spec: SmartSpec,
    pub name: TextEdit,
    /// Texte de la valeur de chaque règle.
    pub values: Vec<TextEdit>,
    pub limit: TextEdit,
    pub popup: Option<String>,
    // Personnalisation.
    pub description: TextEdit,
    pub status: Option<String>,
    /// Grille des pochettes des morceaux ouverte.
    pub picking: bool,
    pub busy: bool,
    pub scene: Cell<Option<SmartScene>>,
}

impl Default for SmartState {
    fn default() -> Self {
        SmartState {
            playlist: None,
            spec: SmartSpec::default(),
            name: TextEdit::new(""),
            values: Vec::new(),
            limit: TextEdit::new("0"),
            popup: None,
            description: TextEdit::new(""),
            status: None,
            picking: false,
            busy: false,
            scene: Cell::new(None),
        }
    }
}

/// Valeur d'une règle telle qu'on la saisit.
fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Saisie d'une règle vers sa valeur : texte tel quel, sinon nombre (0 si illisible, comme `Number(x) || 0`).
fn value_of(field: &str, text: &str) -> Value {
    match smart::field_type(field) {
        Some(FieldType::Text) | None => json!(text),
        Some(_) => match text.trim().parse::<f64>() {
            Ok(n) if n.fract() == 0.0 && n.abs() < 1e15 => json!(n as i64),
            Ok(n) if n.is_finite() => json!(n),
            _ => json!(0),
        },
    }
}

/// Tri : alphabétique dans l'ordre croissant, le reste du plus grand au plus petit.
fn sort_dir(key: &str) -> &'static str {
    if key == "title" || key == "artist" { "asc" } else { "desc" }
}

fn preset_names() -> Vec<String> {
    smart::presets().iter().map(|(k, _)| t(&format!("preset.{k}"))).collect()
}

impl SmartState {
    /// Remet les champs de saisie d'après les règles.
    fn load_spec(&mut self, spec: SmartSpec) {
        self.values = spec.rules.iter().map(|r| TextEdit::new(value_text(&r.value))).collect();
        self.limit = TextEdit::new(spec.limit.max(0).to_string());
        self.spec = spec;
        self.popup = None;
    }

    /// Choix d'un modèle : ses règles, et son nom tant qu'on n'en a pas donné un.
    fn apply_preset(&mut self, key: &str) {
        let Some((_, spec)) = smart::presets().into_iter().find(|(k, _)| *k == key) else { return };
        self.load_spec(spec);
        let name = self.name.text.trim().to_string();
        if self.playlist.is_none() && (name.is_empty() || name == t("pl.newSmart") || preset_names().contains(&name)) {
            self.name.replace_text(t(&format!("preset.{key}")));
        }
    }

    fn set_field(&mut self, i: usize, field: &str) {
        let Some(ft) = smart::field_type(field) else { return };
        if let Some(r) = self.spec.rules.get_mut(i) {
            r.field = field.to_string();
            r.op = smart::ops(ft)[0].to_string();
            r.value = smart::default_value(ft);
            self.values[i] = TextEdit::new(value_text(&r.value));
        }
    }

    fn add_rule(&mut self) {
        self.spec.rules.push(SmartRule { field: "genre".into(), op: "contains".into(), value: json!("") });
        self.values.push(TextEdit::new(""));
    }

    fn remove_rule(&mut self, i: usize) {
        if i < self.spec.rules.len() {
            self.spec.rules.remove(i);
            self.values.remove(i);
        }
    }

    fn set_sort(&mut self, key: &str) {
        self.spec.sort.key = key.to_string();
        self.spec.sort.dir = sort_dir(key).to_string();
    }

    /// Règles à enregistrer.
    fn final_spec(&self) -> SmartSpec {
        let mut spec = self.spec.clone();
        spec.rules.retain(|r| !r.field.is_empty());
        spec
    }
}

/// Ouvre l'éditeur de règles (nouvelle playlist si `None`).
pub fn open_smart(ui: &mut Ui, playlist: Option<PlaylistId>) {
    let pl = playlist.and_then(|id| ui.cat.playlist(id)).cloned();
    let st = &mut ui.smart;
    st.playlist = pl.as_ref().map(|p| p.id);
    let spec = pl
        .as_ref()
        .and_then(|p| p.rules.clone())
        .or_else(|| smart::presets().into_iter().next().map(|(_, s)| s))
        .unwrap_or_default();
    st.load_spec(spec);
    st.name = TextEdit::new(pl.map(|p| p.name).unwrap_or_else(|| t("pl.newSmart")));
    st.name.focused = true;
    ui.menu = None;
    ui.sheet = Some(Sheet::Smart);
}

fn save(s: &mut Ui) {
    let name = s.smart.name.text.trim().to_string();
    let name = if name.is_empty() { t("pl.newSmart") } else { name };
    let spec = s.smart.final_spec();
    s.sheet = None;
    match s.smart.playlist {
        Some(id) => {
            let old = s.cat.playlist(id).map(|p| p.name.clone()).unwrap_or_default();
            s.spawn(
                move |b| {
                    b.library.set_playlist_rules(id, &spec);
                    if name != old {
                        b.library.rename_playlist(id, &name);
                    }
                },
                |s, (), _| s.toast(t("toast.saved"), "check"),
            );
        }
        None => s.spawn(
            move |b| b.library.create_playlist(&name, PlaylistKind::Smart, Some(&spec)),
            |s, id, _| {
                if let Some(id) = id {
                    s.navigate(Route::Playlist(id));
                }
            },
        ),
    }
}

fn section(p: &Palette, label: String, content: El) -> El {
    col().w_full().gap(8.0).child(text(label).font(ty::CAPTION).color(p.text3)).child(content)
}

fn rule_row(ui: &Ui, p: &Palette, k: &Kit, layer: &mut Vec<El>, i: usize, rule: &SmartRule) -> El {
    let st = &ui.smart;
    let ft = smart::field_type(&rule.field).unwrap_or(FieldType::Text);
    let fields: Vec<&str> = smart::FIELDS.iter().map(|(f, _)| *f).collect();
    let field_key = format!("sm-field-{i}");
    let field = popup_select(
        k,
        layer,
        st.popup.as_deref() == Some(field_key.as_str()),
        Popup {
            key: &field_key,
            options: fields.iter().map(|f| t(&format!("field.{f}"))).collect(),
            current: fields.iter().position(|f| *f == rule.field),
            width: 170.0,
            placeholder: String::new(),
        },
        |s| &mut s.smart.popup,
        move |s, j| s.smart.set_field(i, smart::FIELDS[j].0),
    );
    let ops = smart::ops(ft);
    let op_key = format!("sm-op-{i}");
    let op = popup_select(
        k,
        layer,
        st.popup.as_deref() == Some(op_key.as_str()),
        Popup {
            key: &op_key,
            options: ops.iter().map(|o| t(&format!("op.{o}"))).collect(),
            current: ops.iter().position(|o| *o == rule.op),
            width: 170.0,
            placeholder: String::new(),
        },
        |s| &mut s.smart.popup,
        move |s, j| {
            if let Some(r) = s.smart.spec.rules.get_mut(i) {
                r.op = ops[j].to_string();
            }
        },
    );
    let numeric = ft != FieldType::Text;
    let value = TextInput::new(&st.values[i])
        .width(None)
        .spellcheck(false)
        .on_edit(move |s: &mut Ui| &mut s.smart.values[i])
        .on_change(move |s: &mut Ui, _: &mut Tx| {
            let st = &mut s.smart;
            if numeric {
                let clean: String = st.values[i].text.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
                if clean != st.values[i].text {
                    st.values[i].replace_text(clean);
                }
            }
            if let Some(r) = st.spec.rules.get_mut(i) {
                r.value = value_of(&r.field, &st.values[i].text);
            }
        })
        .build(k)
        .key(format!("sm-value-{i}"));
    let suffix = match ft {
        FieldType::Date => Some(t("smart.days")),
        _ if rule.field == "duration" => Some(t("smart.minutes")),
        _ => None,
    };
    let mut r = row()
        .w_full()
        .gap(6.0)
        .items_center()
        .child(field)
        .child(op)
        .child(rune::el().grow(1.0).min_w(60.0).child(value));
    if let Some(sfx) = suffix {
        r = r.child(text(sfx).font(ty::SMALL).color(p.text2));
    }
    r.child(
        IconButton::new(&icons::X)
            .label(t("common.delete"))
            .help(t("common.delete"))
            .on_press(move |s: &mut Ui| s.smart.remove_rule(i))
            .build(k)
            .key(format!("sm-remove-{i}")),
    )
}

fn chips(p: &Palette) -> El {
    let mut r = row().w_full().gap(6.0).flex_wrap();
    for (key, _) in smart::presets() {
        r = r.child(
            row()
                .key(format!("sm-preset-{key}"))
                .h(28.0)
                .px(12.0)
                .radius(14.0)
                .center()
                .fill(p.fill)
                .hover(p.fill_hover)
                .cursor(Cursor::Pointer)
                .role("button")
                .child(text(t(&format!("preset.{key}"))).font(ty::SMALL_MEDIUM).color(p.text))
                .on_press(move |s: &mut Ui| s.smart.apply_preset(key)),
        );
    }
    r
}

fn rules_body(ui: &Ui, p: &Palette, k: &Kit, layer: &mut Vec<El>) -> El {
    let st = &ui.smart;
    let name = TextInput::new(&st.name)
        .width(None)
        .placeholder(t("pl.namePlaceholder"))
        .on_edit(|s: &mut Ui| &mut s.smart.name)
        .on_submit(|s: &mut Ui, _| save(s))
        .build(k)
        .key("sm-name");
    let match_opts = ["all", "any"];
    let matching = popup_select(
        k,
        layer,
        st.popup.as_deref() == Some("sm-match"),
        Popup {
            key: "sm-match",
            options: vec![t("smart.matchAll"), t("smart.matchAny")],
            current: match_opts.iter().position(|m| *m == st.spec.match_),
            width: 220.0,
            placeholder: String::new(),
        },
        |s| &mut s.smart.popup,
        move |s, i| s.smart.spec.match_ = match_opts[i].to_string(),
    );
    let mut rules = col().w_full().gap(8.0).child(matching);
    for (i, rule) in st.spec.rules.iter().enumerate() {
        rules = rules.child(rule_row(ui, p, k, layer, i, rule));
    }
    rules = rules.child(
        small_button(t("smart.addRule"), Some(&icons::PLUS))
            .on_press(|s: &mut Ui| s.smart.add_rule())
            .build(k)
            .key("sm-add")
            .self_start(),
    );
    let sort = popup_select(
        k,
        layer,
        st.popup.as_deref() == Some("sm-sort"),
        Popup {
            key: "sm-sort",
            options: smart::SORT_KEYS.iter().map(|k| t(&format!("sort.{k}"))).collect(),
            current: smart::SORT_KEYS.iter().position(|k| *k == st.spec.sort.key),
            width: 200.0,
            placeholder: String::new(),
        },
        |s| &mut s.smart.popup,
        |s, i| s.smart.set_sort(smart::SORT_KEYS[i]),
    );
    let limit = TextInput::new(&st.limit)
        .width(None)
        .on_edit(|s: &mut Ui| &mut s.smart.limit)
        .on_change(|s: &mut Ui, _: &mut Tx| {
            let st = &mut s.smart;
            let clean: String = st.limit.text.chars().filter(char::is_ascii_digit).take(6).collect();
            if clean != st.limit.text {
                st.limit.replace_text(clean.clone());
            }
            st.spec.limit = clean.parse().unwrap_or(0);
        })
        .build(k)
        .key("sm-limit")
        .help(t("smart.limitHint"));
    col()
        .w_full()
        .gap(18.0)
        .child(section(p, t("pl.name"), name))
        .child(section(p, t("smart.presets"), chips(p)))
        .child(section(p, t("smart.rules"), rules))
        .child(
            row()
                .w_full()
                .gap(10.0)
                .items_center()
                .child(text(t("smart.sortBy")).font(ty::CAPTION).color(p.text3))
                .child(sort)
                .child(text(t("smart.limit")).font(ty::CAPTION).color(p.text3).ml(8.0))
                .child(rune::el().w(80.0).child(limit))
                .child(text(t("smart.limitHint")).font(ty::SMALL).color(p.text3)),
        )
}

fn rules_footer(ui: &Ui, p: &Palette, k: &Kit) -> El {
    let st = &ui.smart;
    // Le compte ne dépend pas du tri : un tri aléatoire n'a pas à mélanger à chaque image.
    let mut spec = st.final_spec();
    spec.sort.key = "title".into();
    let n = ui.cat.eval_smart(&spec, control_core::util::now_ms()).len();
    let confirm = if st.playlist.is_some() { t("common.save") } else { t("common.create") };
    row()
        .w_full()
        .gap(8.0)
        .items_center()
        .child(text(tn("smart.matches", n)).font(ty::BODY_MEDIUM).color(p.text2).grow(1.0).key("sm-count"))
        .child(Button::new(t("common.cancel")).on_press(|s: &mut Ui| s.sheet = None).build(k).key("sm-cancel"))
        .child(Button::new(confirm).variant(ButtonVariant::Contrast).on_press(save).build(k).key("sm-save"))
}

/// Scènes : ouvre ce qui est demandé dès que la bibliothèque est chargée.
fn honour_scene(ui: &Ui) {
    let Some(scene) = ui.smart.scene.get() else { return };
    if !ui.ready {
        return;
    }
    ui.smart.scene.set(None);
    let smart_id = ui.cat.playlists.iter().find(|p| p.kind == PlaylistKind::Smart).map(|p| p.id);
    let first = ui.cat.playlists.first().map(|p| p.id);
    ui.spawn(
        |_| (),
        move |s, (), _| match scene {
            SmartScene::New => open_smart(s, None),
            SmartScene::EditFirst => open_smart(s, smart_id),
            SmartScene::CustomizeFirst => {
                if let Some(id) = first {
                    open_customize(s, id);
                }
            }
        },
    );
}

pub fn sheet(ui: &Ui, p: &Palette, k: &Kit, layer: &mut Vec<El>) {
    honour_scene(ui);
    let mut extra = Vec::new();
    match ui.sheet {
        Some(Sheet::Smart) => {
            let title = if ui.smart.playlist.is_some() { t("pl.editRules") } else { t("pl.newSmart") };
            let body = rules_body(ui, p, k, &mut extra);
            let footer = rules_footer(ui, p, k);
            layer.push(card(p, "sheet-smart", title, (640.0, 760.0), body, footer, |s| s.sheet = None));
        }
        Some(Sheet::PlaylistEdit) => customize::sheet(ui, p, k, layer),
        _ => {}
    }
    layer.extend(extra);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_values_and_presets() {
        let _guard = crate::i18n::TEST_LANG.lock().unwrap_or_else(|e| e.into_inner());
        crate::i18n::set_language("fr");
        assert_eq!(value_of("genre", "Rock"), json!("Rock"));
        assert_eq!(value_of("playCount", "12"), json!(12));
        assert_eq!(value_of("duration", "3.5"), json!(3.5));
        assert_eq!(value_of("addedAt", "abc"), json!(0));
        assert_eq!(value_text(&json!(30)), "30");
        assert_eq!(sort_dir("artist"), "asc");
        assert_eq!(sort_dir("playCount"), "desc");

        let mut st = SmartState { name: TextEdit::new(t("pl.newSmart")), ..Default::default() };
        st.apply_preset("recent");
        assert_eq!(st.name.text, t("preset.recent"));
        assert_eq!(st.spec.rules[0].field, "addedAt");
        assert_eq!(st.values[0].text, "30");
        // Un nom choisi n'est pas remplacé.
        st.name = TextEdit::new("Ma sélection");
        st.apply_preset("genre");
        assert_eq!(st.name.text, "Ma sélection");

        st.add_rule();
        assert_eq!(st.spec.rules.len(), 2);
        st.set_field(1, "year");
        assert_eq!((st.spec.rules[1].op.as_str(), &st.spec.rules[1].value), ("is", &json!(0)));
        st.spec.rules.push(SmartRule { field: String::new(), op: "is".into(), value: json!(0) });
        st.values.push(TextEdit::new(""));
        assert_eq!(st.final_spec().rules.len(), 2);
        st.remove_rule(0);
        assert_eq!(st.spec.rules[0].field, "year");
        st.set_sort("artist");
        assert_eq!(st.spec.sort.dir, "asc");
    }
}

//! Éditeur de tags façon mp3tag (porté de `src/renderer/js/ui/tag-editor.js`) : un ou plusieurs morceaux,
//! pochette, outils par lot avec aperçu, renommage des fichiers. Rien n'est écrit avant « Enregistrer » ;
//! le dernier lot peut ensuite être annulé.
//!
//! Les fichiers sont lus et écrits sur un thread de travail. Sous Linux, réécrire le morceau en cours de
//! lecture ne gêne pas le décodeur (le fichier ouvert reste lisible) : rien n'est suspendu pendant l'écriture.

mod model;
mod tools;
mod view;
pub(crate) mod widgets;

use std::cell::Cell;
use std::collections::HashMap;

use rune::{El, Image, ImageCache, TextEdit, Tx};
use rune_kit::Kit;

use control_core::model::TrackId;
use control_core::tagpattern::CaseMode;
use control_core::tags::{CoverHit, CoverSource};

use crate::i18n::{t, tn, tv};
use crate::state::{Sheet, Ui};
use crate::theme::Palette;

pub use model::Field;
use model::{CoverChange, Model, Row, TrackInfo};

/// Extensions d'images acceptées (choix, dépôt).
const IMAGE_EXT: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp"];

#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    Loading,
    /// Aucun fichier modifiable (message du premier refus).
    Nothing(String),
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Filename,
    Rename,
    Number,
    Replace,
    Case,
    Search,
}

/// Une pochette trouvée en ligne, avec son aperçu une fois téléchargé.
pub struct Hit {
    pub hit: CoverHit,
    pub image: Option<Image>,
}

pub enum Search {
    Idle,
    Loading,
    Failed,
    Done(Vec<Hit>),
}

/// Ouverture différée (scènes de rendu) : les `n` premiers morceaux audio, et éventuellement un outil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneOpen {
    pub count: usize,
    pub tool: Option<Tool>,
}

pub struct TagEditorState {
    pub phase: Phase,
    pub model: Model,
    /// Morceaux non modifiables (format, fichier absent).
    pub skipped: usize,
    /// Texte des champs, un par [`Field`].
    pub fields: Vec<TextEdit>,
    pub tool: Option<Tool>,
    pub saving: bool,
    pub confirm_discard: bool,
    /// Liste déroulante ouverte.
    pub popup: Option<String>,
    pub cover_busy: bool,
    pub drop_over: bool,
    // Outils. Les motifs restent d'une ouverture à l'autre, comme dans l'original.
    pub pat_filename: TextEdit,
    pub pat_rename: TextEdit,
    pub num_start: TextEdit,
    pub num_total: bool,
    pub find: TextEdit,
    pub replace_with: TextEdit,
    pub match_case: bool,
    pub regex: bool,
    pub replace_field: Option<Field>,
    pub case_field: Option<Field>,
    pub case_mode: CaseMode,
    pub search_artist: TextEdit,
    pub search_album: TextEdit,
    pub search: Search,
    search_epoch: u64,
    /// Le dernier lot enregistré peut être annulé.
    pub can_undo: bool,
    images: Option<ImageCache>,
    /// Aperçus des pochettes préparées, par jeton.
    staged: HashMap<String, Image>,
    epoch: u64,
    pub scene: Cell<Option<SceneOpen>>,
}

impl Default for TagEditorState {
    fn default() -> Self {
        TagEditorState {
            phase: Phase::Loading,
            model: Model::default(),
            skipped: 0,
            fields: Field::ALL.iter().map(|_| TextEdit::new("")).collect(),
            tool: None,
            saving: false,
            confirm_discard: false,
            popup: None,
            cover_busy: false,
            drop_over: false,
            pat_filename: TextEdit::new("%track% - %title%"),
            pat_rename: TextEdit::new("%track% - %title%"),
            num_start: TextEdit::new("1"),
            num_total: true,
            find: TextEdit::new(""),
            replace_with: TextEdit::new(""),
            match_case: false,
            regex: false,
            replace_field: Some(Field::Title),
            case_field: None,
            case_mode: CaseMode::Title,
            search_artist: TextEdit::new(""),
            search_album: TextEdit::new(""),
            search: Search::Idle,
            search_epoch: 0,
            can_undo: false,
            images: None,
            staged: HashMap::new(),
            epoch: 0,
            scene: Cell::new(None),
        }
    }
}

impl TagEditorState {
    fn dirty(&self) -> bool {
        !self.saving && self.model.dirty_count() > 0
    }

    /// Recopie les valeurs du modèle dans les champs (après un changement de morceau ou un outil).
    fn sync_fields(&mut self) {
        for f in Field::ALL {
            let (v, _) = self.model.field_value(f);
            if self.fields[f.index()].text != v {
                self.fields[f.index()].replace_text(v);
            }
        }
    }
}

// ─── Ouverture, lecture ──────────────────────────────────────────────────────

/// Ouvre l'éditeur pour ces morceaux.
pub fn open(ui: &mut Ui, ids: Vec<TrackId>) {
    let mut unique: Vec<TrackId> = Vec::with_capacity(ids.len());
    for id in ids {
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    if unique.is_empty() {
        return;
    }
    let infos: HashMap<TrackId, TrackInfo> = unique
        .iter()
        .filter_map(|id| {
            let tr = ui.cat.track(*id)?;
            Some((
                *id,
                TrackInfo {
                    cover: tr.cover.clone(),
                    codec: tr.codec.clone(),
                    bitrate: tr.bitrate,
                    duration: tr.duration,
                    size: tr.size,
                },
            ))
        })
        .collect();
    let st = &mut ui.tag_editor;
    st.epoch += 1;
    st.phase = Phase::Loading;
    st.model = Model::default();
    st.skipped = 0;
    st.tool = None;
    st.saving = false;
    st.confirm_discard = false;
    st.popup = None;
    st.cover_busy = false;
    st.drop_over = false;
    st.search = Search::Idle;
    st.staged.clear();
    if st.images.is_none()
        && let Some(w) = ui.waker.clone()
    {
        st.images = Some(ImageCache::new(w));
    }
    let epoch = st.epoch;
    ui.menu = None;
    ui.sheet = Some(Sheet::TagEditor);
    let tmp = search_dir(ui);
    ui.spawn(
        move |b| {
            // Aperçus de la recherche précédente : plus utiles.
            let _ = std::fs::remove_dir_all(&tmp);
            (b.tags.read(&unique), b.tags.can_undo())
        },
        move |s, (results, can_undo), _| {
            let st = &mut s.tag_editor;
            if st.epoch != epoch {
                return;
            }
            st.can_undo = can_undo;
            let mut rows = Vec::new();
            let mut refused = Vec::new();
            for r in results {
                match (r.fields, r.error) {
                    (Some(fields), None) => {
                        let info = infos.get(&r.id).cloned().unwrap_or_default();
                        rows.push(Row::new(r.id, r.path.unwrap_or_default(), fields, info));
                    }
                    (_, err) => refused.push(err.map(|e| e.message).unwrap_or_default()),
                }
            }
            st.skipped = refused.len();
            if rows.is_empty() {
                st.phase = Phase::Nothing(refused.into_iter().next().unwrap_or_default());
                return;
            }
            st.model = Model::new(rows);
            st.phase = Phase::Ready;
            st.sync_fields();
            st.fields[Field::Title.index()].focused = true;
        },
    );
}

/// Dossier des aperçus téléchargés par la recherche de pochettes.
fn search_dir(ui: &Ui) -> std::path::PathBuf {
    ui.backend.data_dir.join("tmp").join("cover-search")
}

/// Fermer : demande confirmation s'il reste des modifications.
fn request_close(s: &mut Ui) {
    if s.tag_editor.dirty() {
        s.tag_editor.confirm_discard = true;
    } else {
        s.sheet = None;
    }
}

// ─── Pochette ────────────────────────────────────────────────────────────────

/// Prépare une pochette (fichier ou adresse) puis l'applique aux morceaux visés.
fn stage(s: &mut Ui, src: CoverSource, then_close_tool: bool) {
    let st = &mut s.tag_editor;
    st.cover_busy = true;
    let epoch = st.epoch;
    s.spawn(
        move |b| b.tags.stage_cover(&src),
        move |s, res, _| {
            let st = &mut s.tag_editor;
            if st.epoch != epoch {
                return;
            }
            st.cover_busy = false;
            match res {
                Ok(staged) => {
                    if let (Some(images), Some(bytes)) = (&st.images, model::decode_data_url(&staged.preview)) {
                        st.staged.insert(staged.token.clone(), images.load(bytes));
                    }
                    st.model.set_cover(Some(CoverChange::Staged { token: staged.token }));
                    if then_close_tool && st.tool == Some(Tool::Search) {
                        st.tool = None;
                    }
                }
                Err(err) => s.toast(err.to_string(), "triangle-alert"),
            }
        },
    );
}

fn pick_cover(tx: &mut Tx) {
    let options =
        rune::FileDialogOptions { title: t("tag.coverChoose"), ..Default::default() }.filter("Images", IMAGE_EXT);
    tx.open_file(options, |s: &mut Ui, result, _| {
        if let Ok(Some(path)) = result {
            stage(s, CoverSource { path: Some(path.to_string_lossy().into_owned()), ..Default::default() }, false);
        }
    });
}

// ─── Enregistrement, annulation ──────────────────────────────────────────────

fn save(s: &mut Ui) {
    let st = &mut s.tag_editor;
    if st.saving || st.phase != Phase::Ready || st.model.dirty_count() == 0 {
        return;
    }
    st.saving = true;
    st.confirm_discard = false;
    st.popup = None;
    let edits = st.model.edits();
    let epoch = st.epoch;
    s.spawn(
        move |b| b.tags.write(&edits),
        move |s, outcome, _| {
            let st = &mut s.tag_editor;
            let ok = if st.epoch == epoch {
                st.saving = false;
                let ok = st.model.apply_outcome(&outcome);
                st.can_undo = outcome.undoable;
                if ok == outcome.results.len() {
                    if s.sheet == Some(Sheet::TagEditor) {
                        s.sheet = None;
                    }
                } else {
                    st.sync_fields();
                }
                ok
            } else {
                outcome.results.iter().filter(|r| r.ok).count()
            };
            if ok > 0 {
                s.toast(tn("tag.saved", ok), "tags");
            }
        },
    );
}

/// Annule le dernier lot enregistré (fichiers remis dans l'état d'avant), puis relit l'éditeur s'il est ouvert.
fn undo(s: &mut Ui) {
    s.tag_editor.can_undo = false;
    s.spawn(
        |b| b.tags.undo(),
        |s, out, _| {
            let msg = if out.failed > 0 {
                tv("tag.undoPartial", &[("n", &out.restored), ("f", &out.failed)])
            } else {
                tn("tag.undone", out.restored)
            };
            s.toast(msg, "undo-2");
            if s.sheet == Some(Sheet::TagEditor) {
                let ids: Vec<TrackId> = s.tag_editor.model.rows.iter().map(|r| r.id).collect();
                open(s, ids);
            }
        },
    );
}

pub fn sheet(ui: &Ui, p: &Palette, k: &Kit, layer: &mut Vec<El>) {
    // Scènes de rendu : on ouvre sur les premiers morceaux une fois la bibliothèque chargée.
    if let Some(scene) = ui.tag_editor.scene.get()
        && ui.ready
    {
        ui.tag_editor.scene.set(None);
        let ids: Vec<TrackId> = ui.cat.audio.iter().take(scene.count).map(|&e| ui.cat.entries[e].track.id).collect();
        ui.spawn(
            |_| (),
            move |s, (), _| {
                open(s, ids);
                s.tag_editor.tool = scene.tool;
            },
        );
    }
    let mut extra = Vec::new();
    let body = view::body(ui, p, k, &mut extra);
    let footer = view::footer(ui, p, k);
    layer.push(widgets::card(p, "sheet-tags", t("tag.sheetTitle"), (980.0, 820.0), body, footer, request_close));
    layer.extend(extra);
}

/// Morceaux de la sélection, dans l'ordre de la bibliothèque (raccourci Ctrl/Cmd+I).
pub fn open_selection(s: &mut Ui) -> bool {
    if s.selection.ids.is_empty() {
        return false;
    }
    let mut ids: Vec<(usize, TrackId)> =
        s.selection.ids.iter().filter_map(|id| s.cat.by_id.get(id).map(|&e| (e, *id))).collect();
    ids.sort_unstable();
    open(s, ids.into_iter().map(|(_, id)| id).collect());
    true
}

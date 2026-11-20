//! Liste de morceaux virtualisée : seules les lignes visibles sont construites, quelle que soit la taille de la
//! bibliothèque. La ligne 0 porte l'en-tête de la page et celui des colonnes.

use std::borrow::Cow;
use std::rc::Rc;

use rune::{Color, Cursor, El, GrowthSpec, ScrollSpec, Tx, col, el, icon, row, text};

use control_core::model::{PlaylistId, TrackId};

use crate::format;
use crate::i18n::t;
use crate::icons;
use crate::state::{MenuTarget, OpenMenu, Ui};
use crate::theme::Palette;
use crate::ui::common::{CoverKind, cover, ty};

/// Colonnes affichées selon la vue (comme les dispositions de l'original).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Songs,
    Album,
    Playlist,
    Artist,
    Search,
}

impl Layout {
    fn cells(self) -> &'static [Cell] {
        use Cell::*;
        match self {
            Layout::Songs => &[Title, Artist, Album, Plays, Time],
            Layout::Album => &[Title, Plays, Time],
            Layout::Playlist | Layout::Search => &[Title, Artist, Album, Time],
            Layout::Artist => &[Title, Album, Plays, Time],
        }
    }

    fn has_cover(self) -> bool {
        self != Layout::Album
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cell {
    Title,
    Artist,
    Album,
    Plays,
    Time,
}

impl Cell {
    fn sort_key(self) -> &'static str {
        match self {
            Cell::Title => "title",
            Cell::Artist => "artist",
            Cell::Album => "album",
            Cell::Plays => "playCount",
            Cell::Time => "duration",
        }
    }

    fn label_key(self) -> &'static str {
        match self {
            Cell::Title => "col.title",
            Cell::Artist => "col.artist",
            Cell::Album => "col.album",
            Cell::Plays => "col.plays",
            Cell::Time => "col.time",
        }
    }

    /// Largeur fixe, ou poids de la place restante.
    fn size(self) -> (Option<f32>, f32) {
        match self {
            Cell::Title => (None, 2.2),
            Cell::Artist | Cell::Album => (None, 1.3),
            Cell::Plays => (Some(64.0), 0.0),
            Cell::Time => (Some(64.0), 0.0),
        }
    }
}

const ROW_H: f32 = 54.0;
const NUM_W: f32 = 44.0;

/// Une liste de morceaux à afficher.
pub struct TrackList {
    pub key: &'static str,
    pub layout: Layout,
    pub ids: Rc<Vec<TrackId>>,
    /// Entrées de playlist (même longueur que `ids`), pour retirer ou réordonner.
    pub entries: Option<Rc<Vec<i64>>>,
    pub playlist: Option<PlaylistId>,
    /// Tri actif (clé, décroissant) et action d'un clic sur une colonne.
    pub sort: Option<(String, bool)>,
    pub on_sort: Option<fn(&mut Ui, &'static str)>,
    /// Numérotation : position dans la liste, ou numéro de piste (albums).
    pub track_numbers: bool,
}

/// Construit la liste dans sa vue défilante, avec l'en-tête de page `head` au-dessus (reconstruit avec l'état
/// chaque fois que la ligne 0 l'est).
pub fn track_list(ui: &Ui, p: &Palette, list: TrackList, head: impl Fn(&Ui, &Palette) -> El + 'static) -> El {
    let layout = list.layout;
    let ids = list.ids.clone();
    let entries = list.entries.clone();
    let count = ids.len() + 1;
    let header = Rc::new(ColumnsSpec { layout: list.layout, sort: list.sort.clone(), on_sort: list.on_sort });
    let palette = *p;
    let playlist = list.playlist;
    let numbers = list.track_numbers;
    let key_ids = ids.clone();
    let rows = col()
        .key(format!("{}-rows", list.key))
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows(
            count,
            ROW_H,
            move |_s: &Ui, i| {
                if i == 0 { Cow::Borrowed("head") } else { Cow::Owned(format!("t{}-{}", key_ids[i - 1], i)) }
            },
            move |s: &Ui, i| {
                if i == 0 {
                    return col().w_full().child(head(s, &palette)).child(columns_header(&palette, &header));
                }
                track_row(s, &palette, layout, &ids, entries.as_deref().map(|v| v.as_slice()), playlist, i - 1, numbers)
            },
        );
    col()
        .key(list.key)
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: false })
        .scroll_edge_fade(8.0)
        .child(rows.px(32.0).pb(120.0))
        .fill(Color::TRANSPARENT)
        .opacity(if ui.ready { 1.0 } else { 0.0 })
}

/// Ce qu'il faut pour dessiner l'en-tête des colonnes.
struct ColumnsSpec {
    layout: Layout,
    sort: Option<(String, bool)>,
    on_sort: Option<fn(&mut Ui, &'static str)>,
}

fn columns_header(p: &Palette, list: &ColumnsSpec) -> El {
    let mut h = row()
        .w_full()
        .h(36.0)
        .items_center()
        .radius(p.r(8.0))
        .fill(if p.dark { Color::BLACK.alpha(0.18) } else { Color::BLACK.alpha(0.04) })
        .child(text("#").font(ty::CAPTION).color(p.text3).w(NUM_W).align(rune::Align::Center));
    for (n, cell) in list.layout.cells().iter().enumerate() {
        let key = cell.sort_key();
        let active = list.sort.as_ref().is_some_and(|(k, _)| k == key);
        let desc = list.sort.as_ref().is_some_and(|(_, d)| *d);
        let mut label = row().gap(4.0).items_center().child(text(t(cell.label_key())).font(ty::CAPTION).color(if active { p.text } else { p.text3 }));
        if active {
            label = label.child(icon(if desc { &icons::CHEVRON_DOWN } else { &icons::CHEVRON_UP }, 12.0).color(p.text2));
        }
        let mut c = row().h_full().items_center();
        if n == 0 && list.layout.has_cover() {
            c = c.pl(50.0);
        }
        if matches!(cell, Cell::Plays | Cell::Time) {
            c = c.justify_end().pr(14.0);
        }
        c = size_cell(c, *cell);
        if let (Some(on_sort), true) = (list.on_sort, list.sort.is_some()) {
            c = c.cursor(Cursor::Pointer).on_press(move |s: &mut Ui| on_sort(s, key));
        }
        h = h.child(c.child(label));
    }
    h.mb(6.0)
}

fn size_cell(c: El, cell: Cell) -> El {
    match cell.size() {
        (Some(w), _) => c.w(w),
        // Base nulle : seules les proportions comptent, l'en-tête et les lignes s'alignent quel que soit le texte.
        (None, g) => c.w(0.0).grow(g).min_w(0.0),
    }
}

#[allow(clippy::too_many_arguments)]
fn track_row(
    s: &Ui,
    p: &Palette,
    layout: Layout,
    ids: &Rc<Vec<TrackId>>,
    entries: Option<&[i64]>,
    playlist: Option<PlaylistId>,
    i: usize,
    numbers: bool,
) -> El {
    let id = ids[i];
    let Some(e) = s.cat.entry(id) else { return el().h(ROW_H) };
    let tr = &e.track;
    let playing = s.player.current_id() == Some(id);
    let selected = s.selection.ids.contains(&id);
    let title_color = if playing { p.accent } else { p.text };

    let num: El = if playing {
        icon(&icons::AUDIO_LINES, 16.0).color(p.accent)
    } else {
        let n = if numbers { tr.track_no.map(|n| n.to_string()).unwrap_or_default() } else { (i + 1).to_string() };
        text(n).font(ty::NUMBERS).color(p.text3)
    };
    let mut r = row()
        .w_full()
        .h(ROW_H)
        .items_center()
        .radius(p.r(10.0))
        .cursor(Cursor::Default)
        .hover(p.fill)
        .child(row().w(NUM_W).center().child(num));
    if selected {
        r = r.fill(p.accent.alpha(0.22));
    }
    for (n, cell) in layout.cells().iter().enumerate() {
        let c = match cell {
            Cell::Title => {
                let mut c = row().gap(12.0).items_center();
                if n == 0 && layout.has_cover() {
                    c = c.child(cover(s, tr.cover.as_deref(), if tr.album.is_empty() { &tr.title } else { &tr.album }, 38.0, p.r(6.0), if tr.is_video() { CoverKind::Video } else { CoverKind::Music }));
                }
                c.child(text(tr.title.clone()).font(ty::BODY_MEDIUM).color(title_color).truncate().min_w(0.0).shrink())
            }
            Cell::Artist => {
                let label = if e.artist_label.is_empty() { "—".to_string() } else { e.artist_label.clone() };
                row().child(text(label).font(ty::BODY).color(p.text2).truncate().min_w(0.0).shrink())
            }
            Cell::Album => {
                let label = if tr.album.is_empty() { "—".to_string() } else { tr.album.clone() };
                row().child(text(label).font(ty::BODY).color(p.text2).truncate().min_w(0.0).shrink())
            }
            Cell::Plays => row().justify_end().pr(14.0).child(
                text(if tr.play_count > 0 { tr.play_count.to_string() } else { String::new() }).font(ty::NUMBERS).color(p.text3),
            ),
            Cell::Time => row().justify_end().pr(14.0).child(text(format::time(tr.duration)).font(ty::NUMBERS).color(p.text2)),
        };
        r = r.child(size_cell(c.h_full().items_center().min_w(0.0).pr(12.0), *cell));
    }

    let list_for_press = ids.clone();
    let list_for_play = ids.clone();
    let list_for_menu = ids.clone();
    let entry = entries.and_then(|v| v.get(i).copied());
    let list_for_drag = ids.clone();
    r.on_drag(move |st: &mut Ui, d: &mut rune::Drag, _tx| match d.phase {
        rune::Phase::Down => {
            let ids = st.targets(id, &list_for_drag);
            let origin = st.pointer.get();
            st.track_drag = Some(crate::state::TrackDrag { ids, origin, at: origin, active: false });
        }
        rune::Phase::Move => {
            if let Some(dr) = &mut st.track_drag {
                dr.at = (dr.origin.0 + d.dx, dr.origin.1 + d.dy);
                dr.active |= d.dx.abs() + d.dy.abs() > 6.0;
            }
        }
        rune::Phase::Up => {
            if let Some(dr) = st.track_drag.take()
                && !d.cancelled()
                && let Some(pl) = dr.target(st)
            {
                crate::ui::menu::add_to_playlist(st, pl, dr.ids);
            }
        }
    })
    .on_press_tx(move |st: &mut Ui, tx: &mut Tx| {
        let mods = tx.mods();
        st.select(id, &list_for_press, mods.cmd, mods.shift);
    })
    .on_double_press(move |st: &mut Ui| {
        let cat = st.cat.clone();
        let backend = st.backend.clone();
        st.player.play_list(&list_for_play, i, None, &cat, &backend);
    })
    .on_secondary_press(move |st: &mut Ui, at, _tx| {
        let ids = st.targets(id, &list_for_menu);
        if !st.selection.ids.contains(&id) {
            st.select(id, &list_for_menu, false, false);
        }
        let entries = entry.into_iter().collect();
        st.menu = Some(OpenMenu { at, target: MenuTarget::Tracks { ids, entries, playlist }, submenu: false });
    })
}

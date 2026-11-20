//! Lecture. L'original intégrait le lecteur vidrift dans une iframe et enregistrait la progression qu'il envoyait
//! par `postMessage`. Rune n'a pas de vue web : on construit la même adresse d'intégration et on l'ouvre dans le
//! navigateur du système. Aucune position ne revient de là-bas ; on note seulement que l'épisode a été ouvert (voir
//! [`opened_record`]), ce qui suffit à la fiche pour savoir où reprendre et proposer l'épisode suivant.

use serde_json::Value;

use control_core::media::{Details, EmbedOptions, EmbedTarget, Progress, WatchRecord};

use super::Opened;
use super::cards::{Item, error_text, title_route};
use crate::i18n::t;
use crate::state::Ui;

/// Ce qu'il faut savoir d'un titre pour l'enregistrer : titre, visuels, année, genres.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Meta {
    pub title: String,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub year: Option<i64>,
    pub genres: Vec<Value>,
}

impl Meta {
    pub fn of(d: &Details) -> Meta {
        match d {
            Details::Tmdb(t) => Meta {
                title: t.item.title.clone(),
                poster: t.item.poster.clone(),
                backdrop: t.item.backdrop.clone(),
                year: t.item.year,
                genres: t.genres.iter().map(|g| Value::String(g.clone())).collect(),
            },
            Details::Anime(a) => Meta {
                title: a.title.clone(),
                poster: a.poster.clone(),
                backdrop: a.backdrop.clone(),
                year: a.year,
                genres: a.genres.iter().map(|g| Value::String(g.clone())).collect(),
            },
        }
    }

    fn of_item(i: &Item) -> Meta {
        Meta {
            title: i.title.clone(),
            poster: i.poster.clone(),
            backdrop: i.backdrop.clone(),
            year: i.year,
            genres: Vec::new(),
        }
    }

    fn of_progress(rows: &[Progress]) -> Meta {
        rows.first()
            .map(|r| Meta {
                title: r.title.clone(),
                poster: r.poster.clone(),
                backdrop: r.backdrop.clone(),
                year: r.year,
                genres: r.genres.as_array().cloned().unwrap_or_default(),
            })
            .unwrap_or_default()
    }
}

/// Ligne de progression à enregistrer à l'ouverture d'un épisode. Sans lecteur intégré, on ne connaît pas la
/// position : un épisode déjà commencé garde la sienne (elle n'est que rafraîchie, pour passer en tête), un nouvel
/// épisode part de 0. Une position nulle n'ajoute aucun temps regardé et n'entre ni dans « Continuer à regarder »
/// ni dans l'historique (qui demandent 10 s), mais la fiche s'en sert pour savoir où l'on en est.
pub fn opened_record(
    kind: &str,
    id: &str,
    season: Option<i64>,
    episode: Option<i64>,
    rows: &[Progress],
    meta: &Meta,
) -> WatchRecord {
    let known = rows.iter().find(|r| r.season == season && r.episode == episode);
    WatchRecord {
        kind: kind.to_string(),
        id: id.to_string(),
        season,
        episode,
        position: known.map_or(0.0, |r| r.position),
        duration: known.map_or(0.0, |r| r.duration),
        finished: known.is_some_and(|r| r.finished),
        title: meta.title.clone(),
        poster: meta.poster.clone(),
        backdrop: meta.backdrop.clone(),
        year: meta.year,
        genres: meta.genres.clone(),
    }
}

/// Couleur d'accent `rrggbb` pour habiller le lecteur.
fn accent_hex(ui: &Ui) -> Option<String> {
    let c = crate::theme::Palette::from_settings(&ui.settings, true).accent;
    let enc = |v: f32| (rune::color::srgb_encode(v.clamp(0.0, 1.0)) * 255.0).round() as u8;
    Some(format!("{:02x}{:02x}{:02x}", enc(c.r), enc(c.g), enc(c.b)))
}

/// Ouvre l'épisode (ou le film) dans le navigateur et le note comme ouvert.
pub fn open(s: &mut Ui, kind: &str, id: &str, season: Option<i64>, episode: Option<i64>) {
    open_meta(s, kind, id, season, episode, None);
}

/// [`open`], avec les métadonnées d'une affiche pour un titre dont on ne sait encore rien.
fn open_meta(s: &mut Ui, kind: &str, id: &str, season: Option<i64>, episode: Option<i64>, fallback: Option<Meta>) {
    // Pas de musique en fond sous un film.
    if s.player.is_playing() {
        s.player.toggle();
    }
    // Métadonnées : la fiche ouverte si c'est elle, sinon ce que la progression connaît déjà.
    let page_meta = (s.watch.title.kind == kind && s.watch.title.id == id)
        .then(|| s.watch.title.details.ready().map(Meta::of))
        .flatten();
    let color = accent_hex(s);
    let (kind, id) = (kind.to_string(), id.to_string());
    s.watch.opened = Some(Opened { kind: kind.clone(), id: id.clone(), season, episode });
    s.spawn(
        move |b| {
            let rows = b.media.watch_show(&kind, &id).unwrap_or_default();
            let meta = page_meta.unwrap_or_else(|| match fallback {
                Some(m) if rows.is_empty() => m,
                _ => Meta::of_progress(&rows),
            });
            let target = EmbedTarget { kind: kind.clone(), id: id.clone(), season, episode };
            let opts = EmbedOptions {
                title: (!meta.title.is_empty()).then(|| meta.title.clone()),
                color,
                poster: meta.backdrop.clone(),
                dub: false,
            };
            let url = b.media.embed_url(&target, &opts);
            crate::os::open_url(&url);
            let saved = b.media.watch_save(&opened_record(&kind, &id, season, episode, &rows, &meta));
            (saved, kind, id)
        },
        |s, (saved, kind, id), _| {
            match saved {
                Ok(_) => s.toast(t("watch.opened"), "external-link"),
                Err(err) => s.toast(error_text(&err), "triangle-alert"),
            }
            if s.watch.title.kind == kind && s.watch.title.id == id {
                super::load_progress(s);
            }
            super::load_local(s);
        },
    );
}

/// Marque l'épisode ouvert comme vu (le lecteur du navigateur ne le dit pas) : la fiche passe au suivant.
pub fn mark_seen(s: &mut Ui, opened: Opened) {
    let meta = (s.watch.title.kind == opened.kind && s.watch.title.id == opened.id)
        .then(|| s.watch.title.details.ready().map(Meta::of))
        .flatten();
    s.spawn(
        move |b| {
            let rows = b.media.watch_show(&opened.kind, &opened.id).unwrap_or_default();
            let meta = meta.unwrap_or_else(|| Meta::of_progress(&rows));
            let mut rec = opened_record(&opened.kind, &opened.id, opened.season, opened.episode, &rows, &meta);
            rec.finished = true;
            b.media.watch_save(&rec)
        },
        |s, res, _| {
            if let Err(err) = res {
                s.toast(error_text(&err), "triangle-alert");
            }
            super::load_progress(s);
            super::load_local(s);
        },
    );
}

/// Lecture depuis une affiche ou la bannière (port de `media/play.js`) : un film se lance ; une série ou un animé
/// reprend l'épisode en cours, ou commence au premier ; si le dernier épisode vu est fini, la fiche (qui sait
/// calculer le suivant) s'ouvre.
pub fn quick_play(s: &mut Ui, item: Item) {
    if item.kind == "movie" {
        open_with_meta(s, &item, None, None);
        return;
    }
    let (kind, id) = (item.kind.clone(), item.id.clone());
    s.spawn(
        move |b| b.media.watch_show(&kind, &id).unwrap_or_default(),
        move |s, rows, _| match rows.first() {
            None if item.kind == "anime" && item.format == "MOVIE" => open_with_meta(s, &item, None, None),
            None => open_with_meta(s, &item, (item.kind == "tv").then_some(1), Some(1)),
            Some(last) if !last.finished => open(s, &item.kind, &item.id, last.season, last.episode),
            Some(_) => s.navigate(title_route(&item.kind, &item.id)),
        },
    );
}

/// [`open`] depuis une affiche, avec ses métadonnées pour un titre encore jamais ouvert.
fn open_with_meta(s: &mut Ui, item: &Item, season: Option<i64>, episode: Option<i64>) {
    open_meta(s, &item.kind, &item.id, season, episode, Some(Meta::of_item(item)));
}

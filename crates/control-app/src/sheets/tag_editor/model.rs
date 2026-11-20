//! Modèle de l'éditeur de tags, sans interface : valeurs lues, modifications en attente, « Plusieurs valeurs »,
//! outils par lot (aperçu puis application) et préparation de l'écriture.
//!
//! Comme l'original, une modification n'est gardée que si elle diffère de la valeur lue : un champ qui diffère
//! d'un morceau à l'autre n'est donc écrit que si on l'a touché.

use std::collections::BTreeMap;

use control_core::model::TrackId;
use control_core::tagpattern::{self, CaseMode, PartialFields, ReplaceOptions};
use control_core::tags::{CoverEdit, TagEdit, TagFields, TagSet, WriteOutcome};

/// Un champ du formulaire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    Title,
    Artist,
    AlbumArtist,
    Album,
    Genre,
    Year,
    TrackNo,
    TrackTotal,
    DiscNo,
    DiscTotal,
    Composer,
    Comment,
    Lyrics,
}

impl Field {
    pub const ALL: [Field; 13] = [
        Field::Title,
        Field::Artist,
        Field::AlbumArtist,
        Field::Album,
        Field::Genre,
        Field::Year,
        Field::TrackNo,
        Field::TrackTotal,
        Field::DiscNo,
        Field::DiscTotal,
        Field::Composer,
        Field::Comment,
        Field::Lyrics,
    ];
    /// Champs texte que les outils (remplacer, casse, nettoyer) touchent : pas les paroles, comme l'original.
    pub const TEXT: [Field; 7] =
        [Field::Title, Field::Artist, Field::AlbumArtist, Field::Album, Field::Genre, Field::Composer, Field::Comment];

    /// Nom JSON du champ (celui des clés `tag.f.*` et `tag.short.*`).
    pub fn key(self) -> &'static str {
        match self {
            Field::Title => "title",
            Field::Artist => "artist",
            Field::AlbumArtist => "albumArtist",
            Field::Album => "album",
            Field::Genre => "genre",
            Field::Year => "year",
            Field::TrackNo => "trackNo",
            Field::TrackTotal => "trackTotal",
            Field::DiscNo => "discNo",
            Field::DiscTotal => "discTotal",
            Field::Composer => "composer",
            Field::Comment => "comment",
            Field::Lyrics => "lyrics",
        }
    }

    pub fn from_key(key: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.key() == key)
    }

    pub fn numeric(self) -> bool {
        matches!(self, Field::Year | Field::TrackNo | Field::TrackTotal | Field::DiscNo | Field::DiscTotal)
    }

    pub fn index(self) -> usize {
        self as usize
    }

    /// Valeur lue, telle qu'affichée (un nombre à 0 est vide).
    fn read(self, f: &TagFields) -> String {
        let number = |n: u32| if n == 0 { String::new() } else { n.to_string() };
        match self {
            Field::Title => f.title.clone(),
            Field::Artist => f.artist.clone(),
            Field::AlbumArtist => f.album_artist.clone(),
            Field::Album => f.album.clone(),
            Field::Genre => f.genre.clone(),
            Field::Composer => f.composer.clone(),
            Field::Comment => f.comment.clone(),
            Field::Lyrics => f.lyrics.clone(),
            Field::Year => number(f.year),
            Field::TrackNo => number(f.track_no),
            Field::TrackTotal => number(f.track_total),
            Field::DiscNo => number(f.disc_no),
            Field::DiscTotal => number(f.disc_total),
        }
    }

    /// Écrit une valeur affichée dans des champs (un nombre illisible vaut 0, comme `Number(x) || 0`).
    fn write(self, f: &mut TagFields, value: &str) {
        let n = || value.trim().parse::<u32>().unwrap_or(0);
        match self {
            Field::Title => f.title = value.to_string(),
            Field::Artist => f.artist = value.to_string(),
            Field::AlbumArtist => f.album_artist = value.to_string(),
            Field::Album => f.album = value.to_string(),
            Field::Genre => f.genre = value.to_string(),
            Field::Composer => f.composer = value.to_string(),
            Field::Comment => f.comment = value.to_string(),
            Field::Lyrics => f.lyrics = value.to_string(),
            Field::Year => f.year = n(),
            Field::TrackNo => f.track_no = n(),
            Field::TrackTotal => f.track_total = n(),
            Field::DiscNo => f.disc_no = n(),
            Field::DiscTotal => f.disc_total = n(),
        }
    }

    /// Met la valeur dans un lot d'écriture.
    fn put(self, set: &mut TagSet, value: &str) {
        let text = || Some(value.to_string());
        let n = || Some(value.trim().parse::<u32>().unwrap_or(0));
        match self {
            Field::Title => set.title = text(),
            Field::Artist => set.artist = text(),
            Field::AlbumArtist => set.album_artist = text(),
            Field::Album => set.album = text(),
            Field::Genre => set.genre = text(),
            Field::Composer => set.composer = text(),
            Field::Comment => set.comment = text(),
            Field::Lyrics => set.lyrics = text(),
            Field::Year => set.year = n(),
            Field::TrackNo => set.track_no = n(),
            Field::TrackTotal => set.track_total = n(),
            Field::DiscNo => set.disc_no = n(),
            Field::DiscTotal => set.disc_total = n(),
        }
    }
}

/// Saisie d'un champ numérique : chiffres seulement, quatre au plus.
pub fn clean_number(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).take(4).collect()
}

/// Nom du fichier sans dossier ni extension.
pub fn base_name(path: &str) -> String {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match file.rfind('.') {
        Some(i) if i > 0 => file[..i].to_string(),
        _ => file.to_string(),
    }
}

/// Extension avec son point (« .mp3 »), ou rien.
pub fn ext_of(path: &str) -> String {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match file.rfind('.') {
        Some(i) if i > 0 => file[i..].to_string(),
        _ => String::new(),
    }
}

/// Octets d'une adresse `data:…;base64,…` (l'aperçu d'une pochette préparée).
pub fn decode_data_url(url: &str) -> Option<Vec<u8>> {
    let b64 = url.strip_prefix("data:").and_then(|r| r.split_once(',')).map_or(url, |(_, b)| b);
    let mut out = Vec::with_capacity(b64.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in b64.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Pochette choisie mais pas encore écrite.
#[derive(Clone, Debug, PartialEq)]
pub enum CoverChange {
    /// Préparée par `Tags::stage_cover` (son jeton).
    Staged {
        token: String,
    },
    Remove,
}

/// Ce que la bibliothèque sait du morceau (pour la pochette et les informations sur le fichier).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackInfo {
    pub cover: Option<String>,
    pub codec: Option<String>,
    pub bitrate: Option<i64>,
    pub duration: f64,
    pub size: i64,
}

/// Un morceau dans l'éditeur.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: TrackId,
    pub path: String,
    pub base: TagFields,
    pub edits: BTreeMap<Field, String>,
    pub cover: Option<CoverChange>,
    /// Nouveau nom de fichier (sans extension).
    pub rename: Option<String>,
    pub error: Option<String>,
    pub info: TrackInfo,
}

impl Row {
    pub fn new(id: TrackId, path: String, base: TagFields, info: TrackInfo) -> Row {
        Row { id, path, base, edits: BTreeMap::new(), cover: None, rename: None, error: None, info }
    }

    pub fn value(&self, f: Field) -> String {
        self.edits.get(&f).cloned().unwrap_or_else(|| f.read(&self.base))
    }

    /// Change un champ ; revenir à la valeur lue efface la modification.
    pub fn set(&mut self, f: Field, value: &str) {
        if value == f.read(&self.base) {
            self.edits.remove(&f);
        } else {
            self.edits.insert(f, value.to_string());
        }
    }

    pub fn is_dirty(&self) -> bool {
        !self.edits.is_empty() || self.cover.is_some() || self.rename.is_some()
    }

    /// Champs actuels, modifications comprises (pour composer un nom de fichier).
    pub fn fields(&self) -> TagFields {
        let mut out = self.base.clone();
        for (f, v) in &self.edits {
            f.write(&mut out, v);
        }
        out
    }

    pub fn file_base(&self) -> String {
        base_name(&self.path)
    }

    pub fn title(&self) -> String {
        let t = self.value(Field::Title);
        if t.is_empty() { self.file_base() } else { t }
    }

    /// Ce qu'il faut écrire pour ce morceau.
    pub fn edit(&self) -> TagEdit {
        let mut set = TagSet::default();
        for (f, v) in &self.edits {
            f.put(&mut set, v);
        }
        let cover = match &self.cover {
            Some(CoverChange::Staged { token }) => Some(CoverEdit::Token { token: token.clone() }),
            Some(CoverChange::Remove) => Some(CoverEdit::Remove { remove: true }),
            None => None,
        };
        TagEdit { id: self.id, set, cover, rename: self.rename.clone() }
    }

    /// Clé de la pochette affichée (pour savoir si elles diffèrent).
    fn cover_key(&self) -> String {
        match &self.cover {
            Some(CoverChange::Remove) => "none".into(),
            Some(CoverChange::Staged { token }) => format!("t:{token}"),
            None => self.info.cover.clone().unwrap_or_else(|| "none".into()),
        }
    }
}

/// Morceaux visés : tous, ou un seul (navigation précédent / suivant).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Scope {
    #[default]
    All,
    One(usize),
}

/// Ce que montre la pochette pour les morceaux visés.
#[derive(Clone, Debug, PartialEq)]
pub enum CoverView {
    /// Jeton de la pochette préparée.
    Staged(String),
    Hash(String),
    None,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub rows: Vec<Row>,
    pub scope: Scope,
}

impl Model {
    pub fn new(rows: Vec<Row>) -> Model {
        let scope = if rows.len() > 1 { Scope::All } else { Scope::One(0) };
        Model { rows, scope }
    }

    /// Indices des morceaux visés.
    pub fn targets(&self) -> Vec<usize> {
        match self.scope {
            Scope::One(i) if i < self.rows.len() => vec![i],
            _ => (0..self.rows.len()).collect(),
        }
    }

    /// Valeur commune d'un champ, ou `(“”, true)` si elle diffère d'un morceau à l'autre.
    pub fn field_value(&self, f: Field) -> (String, bool) {
        let mut values = self.targets().into_iter().map(|i| self.rows[i].value(f));
        let first = values.next().unwrap_or_default();
        if values.any(|v| v != first) { (String::new(), true) } else { (first, false) }
    }

    pub fn field_dirty(&self, f: Field) -> bool {
        self.targets().into_iter().any(|i| self.rows[i].edits.contains_key(&f))
    }

    /// Saisie dans le formulaire : la valeur va à tous les morceaux visés.
    pub fn set_field(&mut self, f: Field, value: &str) {
        for i in self.targets() {
            self.rows[i].set(f, value);
        }
    }

    pub fn dirty_count(&self) -> usize {
        self.rows.iter().filter(|r| r.is_dirty()).count()
    }

    pub fn failed(&self) -> Vec<&Row> {
        self.rows.iter().filter(|r| r.error.is_some()).collect()
    }

    /// Lot à écrire : les morceaux modifiés seulement.
    pub fn edits(&self) -> Vec<TagEdit> {
        self.rows.iter().filter(|r| r.is_dirty()).map(Row::edit).collect()
    }

    /// Après l'écriture : les réussites deviennent les nouvelles valeurs lues, les échecs gardent leurs
    /// modifications et leur message. Renvoie le nombre de réussites.
    pub fn apply_outcome(&mut self, outcome: &WriteOutcome) -> usize {
        let mut ok = 0;
        for res in &outcome.results {
            let Some(row) = self.rows.iter_mut().find(|r| r.id == res.id) else { continue };
            if res.ok {
                ok += 1;
                row.base = row.fields();
                row.edits.clear();
                row.cover = None;
                row.rename = None;
                row.error = None;
                if let Some(p) = &res.path {
                    row.path.clone_from(p);
                }
            } else {
                row.error = Some(res.error.as_ref().map(|e| e.message.clone()).unwrap_or_default());
            }
        }
        ok
    }

    pub fn set_cover(&mut self, change: Option<CoverChange>) {
        for i in self.targets() {
            self.rows[i].cover.clone_from(&change);
        }
    }

    /// Pochette des morceaux visés, et si elles diffèrent.
    pub fn cover_view(&self) -> (CoverView, bool) {
        let targets = self.targets();
        let Some(&first) = targets.first() else { return (CoverView::None, false) };
        let key = self.rows[first].cover_key();
        let mixed = targets.iter().any(|&i| self.rows[i].cover_key() != key);
        let row = &self.rows[first];
        let view = match &row.cover {
            Some(CoverChange::Remove) => CoverView::None,
            Some(CoverChange::Staged { token }) => CoverView::Staged(token.clone()),
            None => row.info.cover.clone().map_or(CoverView::None, CoverView::Hash),
        };
        (view, mixed)
    }

    pub fn step(&mut self, delta: isize) {
        if let Scope::One(i) = self.scope {
            let next = i as isize + delta;
            if next >= 0 && (next as usize) < self.rows.len() {
                self.scope = Scope::One(next as usize);
            }
        }
    }

    /// « Nettoyer » : espaces en trop et parasites (« (Official Video) »…). Renvoie le nombre de champs changés.
    pub fn tidy(&mut self) -> usize {
        let mut n = 0;
        for i in self.targets() {
            for f in Field::TEXT {
                let before = self.rows[i].value(f);
                let after = tagpattern::tidy(&before);
                if after != before {
                    self.rows[i].set(f, &after);
                    n += 1;
                }
            }
        }
        n
    }
}

// ─── Outils par lot : un aperçu, puis on applique ────────────────────────────

/// Valeur d'un champ relu dans un nom de fichier.
fn partial_value(p: &PartialFields, f: Field) -> Option<String> {
    let n = |v: Option<u32>| v.map(|n| n.to_string());
    match f {
        Field::Title => p.title.clone(),
        Field::Artist => p.artist.clone(),
        Field::AlbumArtist => p.album_artist.clone(),
        Field::Album => p.album.clone(),
        Field::Genre => p.genre.clone(),
        Field::Composer => p.composer.clone(),
        Field::Year => n(p.year),
        Field::TrackNo => n(p.track_no),
        Field::TrackTotal => n(p.track_total),
        Field::DiscNo => n(p.disc_no),
        Field::DiscTotal => n(p.disc_total),
        Field::Comment | Field::Lyrics => None,
    }
}

/// « Tags depuis le nom » : ce que le motif lit dans chaque nom de fichier.
#[derive(Clone, Debug, PartialEq)]
pub struct FilenamePlan {
    pub row: usize,
    pub name: String,
    /// Champs lus, dans l'ordre du motif ; `None` si le nom ne correspond pas.
    pub parsed: Option<Vec<(Field, String)>>,
}

pub fn plan_filename(m: &Model, pattern: &str) -> Vec<FilenamePlan> {
    let compiled = tagpattern::compile_pattern(pattern);
    m.targets()
        .into_iter()
        .map(|row| {
            let name = m.rows[row].file_base();
            let parsed = compiled.as_ref().and_then(|c| {
                let p = c.parse(&name)?;
                let mut out: Vec<(Field, String)> = Vec::new();
                for key in c.keys() {
                    if let Some(f) = Field::from_key(key)
                        && !out.iter().any(|(g, _)| *g == f)
                        && let Some(v) = partial_value(&p, f)
                    {
                        out.push((f, v));
                    }
                }
                Some(out)
            });
            FilenamePlan { row, name, parsed }
        })
        .collect()
}

pub fn apply_filename(m: &mut Model, plan: &[FilenamePlan]) {
    for p in plan {
        for (f, v) in p.parsed.iter().flatten() {
            m.rows[p.row].set(*f, v);
        }
    }
}

/// « Renommer les fichiers » : nom actuel, nouveau nom, et s'il est en double.
#[derive(Clone, Debug, PartialEq)]
pub struct RenamePlan {
    pub row: usize,
    pub current: String,
    pub next: String,
    pub duplicate: bool,
}

impl RenamePlan {
    pub fn changes(&self) -> bool {
        !self.next.is_empty() && self.next != self.current
    }
}

pub fn plan_rename(m: &Model, pattern: &str) -> Vec<RenamePlan> {
    let rows = m.targets();
    let names: Vec<String> = rows.iter().map(|&i| tagpattern::render_pattern(pattern, &m.rows[i].fields())).collect();
    let dup = tagpattern::duplicate_names(&names);
    rows.into_iter()
        .zip(names)
        .zip(dup)
        .map(|((row, next), duplicate)| RenamePlan { row, current: m.rows[row].file_base(), next, duplicate })
        .collect()
}

pub fn apply_rename(m: &mut Model, plan: &[RenamePlan]) {
    for p in plan {
        m.rows[p.row].rename = p.changes().then(|| p.next.clone());
    }
}

/// « Numéroter » : numéros proposés dans l'ordre de la liste.
#[derive(Clone, Debug, PartialEq)]
pub struct NumberPlan {
    pub row: usize,
    pub track_no: u32,
    pub total: Option<u32>,
}

pub fn plan_number(m: &Model, start: u32, with_total: bool) -> Vec<NumberPlan> {
    let rows = m.targets();
    let nums = tagpattern::numbering(rows.len(), start, with_total);
    rows.into_iter().zip(nums).map(|(row, n)| NumberPlan { row, track_no: n.track_no, total: n.track_total }).collect()
}

pub fn apply_number(m: &mut Model, plan: &[NumberPlan]) {
    for p in plan {
        m.rows[p.row].set(Field::TrackNo, &p.track_no.to_string());
        if let Some(t) = p.total {
            m.rows[p.row].set(Field::TrackTotal, &t.to_string());
        }
    }
}

/// Un champ changé par « Remplacer » ou « Casse ».
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub row: usize,
    pub field: Field,
    pub before: String,
    pub after: String,
}

fn plan_text(m: &Model, field: Option<Field>, compute: impl Fn(&str) -> String) -> Vec<Change> {
    let fields: Vec<Field> = field.map_or_else(|| Field::TEXT.to_vec(), |f| vec![f]);
    let mut out = Vec::new();
    for row in m.targets() {
        for &f in &fields {
            let before = m.rows[row].value(f);
            if before.is_empty() {
                continue;
            }
            let after = compute(&before);
            if after != before {
                out.push(Change { row, field: f, before, after });
            }
        }
    }
    out
}

/// `field` à `None` : tous les champs texte.
pub fn plan_replace(m: &Model, field: Option<Field>, o: &ReplaceOptions) -> Vec<Change> {
    plan_text(m, field, |v| tagpattern::replace_text(v, o))
}

pub fn plan_case(m: &Model, field: Option<Field>, mode: CaseMode) -> Vec<Change> {
    plan_text(m, field, |v| tagpattern::change_case(v, mode))
}

pub fn apply_changes(m: &mut Model, plan: &[Change]) {
    for c in plan {
        m.rows[c.row].set(c.field, &c.after);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use control_core::tags::{TagError, WriteResult};

    fn row(id: TrackId, path: &str, title: &str, artist: &str, track: u32) -> Row {
        let base = TagFields { title: title.into(), artist: artist.into(), track_no: track, ..Default::default() };
        Row::new(id, path.into(), base, TrackInfo::default())
    }

    fn model() -> Model {
        Model::new(vec![
            row(1, "/m/01 - Bloom.mp3", "Bloom", "Aurora Vale", 1),
            row(2, "/m/02 - Greenhouse.flac", "Greenhouse", "Aurora Vale", 2),
            row(3, "/m/odd name.mp3", "", "Kaito Ren", 0),
        ])
    }

    #[test]
    fn mixed_values_are_written_only_when_edited() {
        let mut m = model();
        assert_eq!(m.scope, Scope::All);
        assert_eq!(m.field_value(Field::Artist), (String::new(), true));
        assert_eq!(m.field_value(Field::Album), (String::new(), false));
        assert!(m.edits().is_empty());

        // Un champ commun modifié va à tous ; les champs « Plusieurs valeurs » non touchés restent absents.
        m.set_field(Field::Album, "Glass Garden");
        let edits = m.edits();
        assert_eq!(edits.len(), 3);
        assert!(edits.iter().all(|e| e.set.album.as_deref() == Some("Glass Garden") && e.set.artist.is_none()));
        assert!(m.field_dirty(Field::Album) && !m.field_dirty(Field::Artist));

        // Revenir à la valeur lue annule la modification.
        m.set_field(Field::Album, "");
        assert_eq!(m.dirty_count(), 0);

        // Un seul morceau visé : seul lui change ; les nombres partent en nombres.
        m.scope = Scope::One(1);
        m.set_field(Field::Year, "2021");
        m.set_field(Field::TrackNo, "");
        let edits = m.edits();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].id, 2);
        assert_eq!(edits[0].set.year, Some(2021));
        assert_eq!(edits[0].set.track_no, Some(0));
        m.step(1);
        assert_eq!(m.scope, Scope::One(2));
        m.step(1);
        assert_eq!(m.scope, Scope::One(2));
        m.step(-5);
        assert_eq!(m.scope, Scope::One(2));
    }

    #[test]
    fn names_and_numbers() {
        assert_eq!(base_name("/a/b/03 - Afterglow.mp3"), "03 - Afterglow");
        assert_eq!(ext_of("/a/b/03 - Afterglow.mp3"), ".mp3");
        assert_eq!(base_name("C:\\x\\.hidden"), ".hidden");
        assert_eq!(ext_of("/a/noext"), "");
        assert_eq!(clean_number("20a21x9"), "2021");
        assert_eq!(model().rows[2].title(), "odd name");
        assert_eq!(decode_data_url("data:image/jpeg;base64,aGVsbG8gd29ybGQ=").as_deref(), Some(&b"hello world"[..]));
        assert_eq!(decode_data_url("SGk").as_deref(), Some(&b"Hi"[..]));
        assert_eq!(decode_data_url("data:x;base64,@@"), None);
    }

    #[test]
    fn filename_tool_reads_tags_from_names() {
        let mut m = model();
        let plan = plan_filename(&m, "%track% - %title%");
        assert_eq!(plan.len(), 3);
        assert_eq!(plan[0].parsed, Some(vec![(Field::TrackNo, "1".into()), (Field::Title, "Bloom".into())]));
        assert!(plan[2].parsed.is_none());
        assert!(plan_filename(&m, "no markers").iter().all(|p| p.parsed.is_none()));
        let plan = plan_filename(&m, "%track% - %artist%");
        apply_filename(&mut m, &plan);
        assert_eq!(m.rows[1].value(Field::Artist), "Greenhouse");
        assert_eq!(m.rows[2].edits.len(), 0);
    }

    #[test]
    fn rename_tool_uses_pending_edits_and_flags_duplicates() {
        let mut m = model();
        m.rows[2].set(Field::Title, "Bloom");
        m.rows[2].set(Field::TrackNo, "1");
        let plan = plan_rename(&m, "%track% - %title%");
        assert_eq!(plan[0].next, "01 - Bloom");
        assert!(!plan[0].changes());
        assert!(plan[0].duplicate && plan[2].duplicate && !plan[1].duplicate);
        apply_rename(&mut m, &plan);
        assert_eq!(m.rows[0].rename, None);
        assert_eq!(m.rows[2].rename.as_deref(), Some("01 - Bloom"));
        assert_eq!(m.rows[2].edit().rename.as_deref(), Some("01 - Bloom"));
    }

    #[test]
    fn number_replace_case_and_tidy() {
        let mut m = model();
        let plan = plan_number(&m, 5, true);
        assert_eq!(
            plan.iter().map(|p| (p.track_no, p.total)).collect::<Vec<_>>(),
            vec![(5, Some(7)), (6, Some(7)), (7, Some(7))]
        );
        apply_number(&mut m, &plan);
        assert_eq!(m.rows[2].value(Field::TrackNo), "7");
        assert_eq!(m.rows[2].value(Field::TrackTotal), "7");

        let o = ReplaceOptions { find: "aurora".into(), replace: "Dawn".into(), ..Default::default() };
        let plan = plan_replace(&m, Some(Field::Artist), &o);
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].after, "Dawn Vale");
        let rx =
            ReplaceOptions { find: r"(\w+) (\w+)".into(), replace: "$2 $1".into(), regex: true, ..Default::default() };
        assert_eq!(plan_replace(&m, Some(Field::Artist), &rx)[2].after, "Ren Kaito");
        apply_changes(&mut m, &plan);
        assert_eq!(m.rows[1].value(Field::Artist), "Dawn Vale");

        let plan = plan_case(&m, None, CaseMode::Upper);
        assert!(plan.iter().any(|c| c.field == Field::Title && c.after == "BLOOM"));
        // Les champs vides ne comptent pas.
        assert!(plan.iter().all(|c| !c.before.is_empty()));

        m.rows[0].set(Field::Title, "  Bloom   (Official Video) ");
        m.scope = Scope::One(0);
        assert_eq!(m.tidy(), 1);
        assert_eq!(m.rows[0].value(Field::Title), "Bloom");
        assert!(!m.rows[0].edits.contains_key(&Field::Title));
    }

    #[test]
    fn covers_and_outcome() {
        let mut m = model();
        m.rows[0].info.cover = Some("abc".into());
        m.rows[1].info.cover = Some("abc".into());
        m.scope = Scope::One(0);
        assert_eq!(m.cover_view(), (CoverView::Hash("abc".into()), false));
        m.scope = Scope::All;
        assert_eq!(m.cover_view(), (CoverView::Hash("abc".into()), true));
        m.set_cover(Some(CoverChange::Remove));
        assert_eq!(m.cover_view(), (CoverView::None, false));
        assert!(matches!(m.rows[2].edit().cover, Some(CoverEdit::Remove { remove: true })));

        m.rows[0].set(Field::Title, "New");
        let outcome = WriteOutcome {
            results: vec![
                WriteResult { id: 1, ok: true, path: Some("/m/New.mp3".into()), renamed: Some(true), error: None },
                WriteResult {
                    id: 2,
                    ok: false,
                    path: None,
                    renamed: None,
                    error: Some(TagError {
                        code: control_core::tags::ErrorCode::Readonly,
                        message: "Lecture seule".into(),
                    }),
                },
            ],
            undoable: true,
        };
        assert_eq!(m.apply_outcome(&outcome), 1);
        assert_eq!(m.rows[0].base.title, "New");
        assert!(!m.rows[0].is_dirty());
        assert_eq!(m.rows[0].path, "/m/New.mp3");
        assert_eq!(m.rows[1].error.as_deref(), Some("Lecture seule"));
        assert!(m.rows[1].is_dirty());
        assert_eq!(m.failed().len(), 1);
    }
}

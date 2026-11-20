//! Réglages : onglets Général, Apparence, Bibliothèque, Streaming, Téléchargements, À propos ; égaliseur.

use rune::{Color, Cursor, Drag, El, col, el, icon, row, text};
use rune_kit::{Kit, Switch, TextInput};
use serde_json::{Value, json};

use crate::i18n::{t, tn};
use crate::icons;
use crate::state::{Sheet, SettingsTab, Ui};
use crate::theme::{ACCENTS, Palette, hex};
use crate::ui::common::ty;
use crate::views::segmented;

/// Préréglages de l'égaliseur (gains en dB des 10 bandes), ceux de l'original.
pub const EQ_BANDS: [u32; 10] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
pub const EQ_PRESETS: &[(&str, [f64; 10])] = &[
    ("flat", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("bass", [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("treble", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 4.0, 5.0, 6.0]),
    ("vocal", [-2.0, -2.0, -1.0, 1.0, 3.0, 3.0, 2.0, 0.0, -1.0, -2.0]),
    ("rock", [4.0, 3.0, 2.0, -1.0, -2.0, -1.0, 2.0, 3.0, 4.0, 4.0]),
    ("electronic", [5.0, 4.0, 1.0, 0.0, -2.0, 2.0, 1.0, 2.0, 4.0, 5.0]),
    ("classical", [3.0, 2.0, 1.0, 1.0, -1.0, -1.0, 0.0, 2.0, 3.0, 4.0]),
    ("acoustic", [4.0, 3.0, 2.0, 1.0, 1.0, 1.0, 2.0, 3.0, 3.0, 2.0]),
    ("jazz", [3.0, 2.0, 1.0, 2.0, -2.0, -2.0, 0.0, 1.0, 2.0, 3.0]),
    ("loudness", [6.0, 4.0, 0.0, 0.0, -1.0, 0.0, -1.0, 0.0, 4.0, 2.0]),
];

const TABS: &[(SettingsTab, &str, &rune::Icon)] = &[
    (SettingsTab::General, "general", &icons::SETTINGS),
    (SettingsTab::Appearance, "appearance", &icons::PALETTE),
    (SettingsTab::Library, "library", &icons::HARD_DRIVE),
    (SettingsTab::Streaming, "streaming", &icons::CLAPPERBOARD),
    (SettingsTab::Downloads, "downloads", &icons::CLOUD_DOWNLOAD),
    (SettingsTab::About, "about", &icons::INFO),
];

fn group(p: &Palette, title: String, rows: Vec<El>, footer: Option<String>) -> El {
    let mut body = col().w_full().radius(p.r(12.0)).fill(p.group).border(1.0, p.separator).clip_rounded(p.r(12.0));
    let n = rows.len();
    for (i, r) in rows.into_iter().enumerate() {
        body = body.child(r);
        if i + 1 < n {
            body = body.child(el().h(1.0).w_full().ml(14.0).fill(p.separator));
        }
    }
    let mut g = col().w_full().gap(6.0).mb(18.0).child(text(title).font(ty::CAPTION).color(p.text3).px(4.0)).child(body);
    if let Some(f) = footer {
        g = g.child(text(f).font(ty::SMALL).color(p.text3).wrap().px(4.0));
    }
    g
}

fn line(p: &Palette, label: String, hint: Option<String>, control: El) -> El {
    let mut left = col().gap(2.0).grow(1.0).min_w(0.0).child(text(label).font(ty::BODY).color(p.text));
    if let Some(h) = hint {
        left = left.child(text(h).font(ty::SMALL).color(p.text2).wrap());
    }
    row().w_full().min_h(44.0).px(14.0).py(8.0).gap(14.0).items_center().child(left).child(control)
}

fn toggle(k: &Kit, on: bool, patch: impl Fn(bool) -> Value + 'static) -> El {
    Switch::new(on).on_toggle(move |s: &mut Ui| s.set(patch(!on))).build(k)
}

/// Curseur à pas entiers, avec sa valeur affichée.
fn int_slider(p: &Palette, key: &str, value: f64, min: f64, max: f64, label: String, set: fn(&mut Ui, f64)) -> El {
    let w = 170.0;
    let frac = ((value - min) / (max - min)).clamp(0.0, 1.0) as f32;
    el().key(key.to_string())
        .w(w)
        .h(18.0)
        .center_y()
        .cursor(Cursor::Pointer)
        .role("slider")
        .label(label)
        .range_value(value, min, max)
        .child(
            el().w_full().h(4.0).radius(2.0).fill(p.control).child(el().w(w * frac).h(4.0).radius(2.0).fill(p.accent)).child(
                el().absolute(w * frac - 8.0, -6.0).size(16.0, 16.0).radius(8.0).fill(Color::WHITE).shadow(1.0, 4.0, Color::BLACK.alpha(0.3)),
            ),
        )
        .on_drag(move |s: &mut Ui, d: &mut Drag, _| {
            if d.w > 0.0 {
                let v = min + ((d.x / d.w).clamp(0.0, 1.0) as f64) * (max - min);
                set(s, v.round());
            }
        })
        .on_step(move |s: &mut Ui, delta, _| set(s, (value + delta as f64).clamp(min, max)))
}

fn general(ui: &Ui, p: &Palette, k: &Kit) -> Vec<El> {
    let s = &ui.settings;
    let crossfade_label = if s.crossfade > 0.0 { tn("set.seconds", s.crossfade as i64) } else { t("set.off") };
    let mut playback = vec![
        line(
            p,
            t("set.crossfade"),
            Some(t("set.crossfadeHint")),
            row().gap(10.0).items_center().child(text(crossfade_label).font(ty::SMALL).color(p.text2)).child(int_slider(
                p,
                "set-crossfade",
                s.crossfade,
                0.0,
                12.0,
                t("set.crossfade"),
                |s, v| s.set(json!({ "crossfade": v })),
            )),
        ),
        line(p, t("set.normalize"), Some(t("set.normalizeHint")), toggle(k, s.normalize, |v| json!({ "normalize": v }))),
    ];
    let target = format!("{}", s.normalize_target as i64);
    playback.push(
        line(
            p,
            t("set.normalizeTarget"),
            None,
            segmented(
                p,
                "set-target",
                [-18, -16, -14].iter().map(|v| (v.to_string(), t(&format!("set.normalizeTarget.{}", -v)))).collect(),
                &target,
                |s, v| s.set(json!({ "normalizeTarget": v.parse::<i64>().unwrap_or(-16) })),
            ),
        )
        .opacity(if s.normalize { 1.0 } else { 0.45 }),
    );
    playback.push(line(
        p,
        t("eq.title"),
        None,
        rune_kit::Button::new(t("common.change")).on_press(|s: &mut Ui| s.sheet = Some(Sheet::Equalizer)).build(k),
    ));
    vec![
        group(
            p,
            t("set.interface"),
            vec![
                line(p, t("set.compact"), Some(t("set.compactHint")), toggle(k, s.compact, |v| json!({ "compact": v }))),
                line(p, t("set.animations"), None, toggle(k, s.animations, |v| json!({ "animations": v }))),
            ],
            None,
        ),
        group(p, t("set.playback"), playback, None),
        group(
            p,
            t("dc.title"),
            vec![
                line(p, t("dc.enable"), Some(t("dc.enableHint")), toggle(k, s.discord.enabled, |v| json!({ "discord": { "enabled": v } }))),
                line(p, t("dc.music"), None, toggle(k, s.discord.music, |v| json!({ "discord": { "music": v } }))),
                line(p, t("dc.video"), None, toggle(k, s.discord.video, |v| json!({ "discord": { "video": v } }))),
                line(p, t("dc.streaming"), None, toggle(k, s.discord.streaming, |v| json!({ "discord": { "streaming": v } }))),
                line(p, t("dc.hide"), Some(t("dc.hideHint")), toggle(k, s.discord.hide_titles, |v| json!({ "discord": { "hideTitles": v } }))),
                line(p, t("dc.covers"), Some(t("dc.coversHint")), toggle(k, s.discord.covers, |v| json!({ "discord": { "covers": v } }))),
            ],
            Some(t("dc.footer")),
        ),
        group(
            p,
            t("set.language"),
            vec![line(
                p,
                t("set.language"),
                None,
                segmented(
                    p,
                    "set-lang",
                    ["auto", "fr", "en"].iter().map(|v| (v.to_string(), t(&format!("lang.{v}")))).collect(),
                    &s.language,
                    |s, v| s.set(json!({ "language": v })),
                ),
            )],
            Some(t("set.languageHint")),
        ),
    ]
}

fn appearance(ui: &Ui, p: &Palette, k: &Kit) -> Vec<El> {
    let s = &ui.settings;
    let mut swatches = row().gap(8.0).items_center();
    for (name, dark, light) in ACCENTS {
        let c = hex(if p.dark { dark } else { light }).unwrap_or(p.accent);
        let on = s.accent == *name;
        let n = name.to_string();
        swatches = swatches.child(
            el().key(format!("acc-{name}"))
                .size(22.0, 22.0)
                .radius(11.0)
                .fill(c)
                .cursor(Cursor::Pointer)
                .role("radio")
                .label(t(&format!("accent.{name}")))
                .checked(on)
                .border(if on { 2.5 } else { 0.0 }, p.text)
                .on_press(move |s: &mut Ui| s.set(json!({ "accent": n }))),
        );
    }
    let pct = |v: f64| format!("{} %", v.round() as i64);
    vec![
        group(
            p,
            t("set.colors"),
            vec![
                line(
                    p,
                    t("set.theme"),
                    None,
                    segmented(
                        p,
                        "set-theme",
                        ["auto", "light", "dark", "oled"].iter().map(|v| (v.to_string(), t(&format!("theme.{v}")))).collect(),
                        &s.theme,
                        |s, v| s.set(json!({ "theme": v })),
                    ),
                ),
                line(p, t("set.accent"), None, swatches),
                line(p, t("set.ambient"), Some(t("set.ambientHint")), toggle(k, s.ambient, |v| json!({ "ambient": v }))),
            ],
            None,
        ),
        group(
            p,
            t("set.shape"),
            vec![
                line(
                    p,
                    t("set.radius"),
                    None,
                    row().gap(10.0).items_center().child(text(pct(s.radius)).font(ty::SMALL).color(p.text2)).child(int_slider(
                        p,
                        "set-radius",
                        s.radius,
                        0.0,
                        200.0,
                        t("set.radius"),
                        |s, v| s.set(json!({ "radius": v })),
                    )),
                ),
                line(
                    p,
                    t("set.translucency"),
                    None,
                    row().gap(10.0).items_center().child(text(pct(s.translucency)).font(ty::SMALL).color(p.text2)).child(
                        int_slider(p, "set-tl", s.translucency, 30.0, 150.0, t("set.translucency"), |s, v| {
                            s.set(json!({ "translucency": v }))
                        }),
                    ),
                ),
            ],
            None,
        ),
    ]
}

fn library(ui: &Ui, p: &Palette, k: &Kit) -> Vec<El> {
    let s = &ui.settings;
    let mut folders: Vec<El> = Vec::new();
    for f in &s.library_folders {
        let folder = f.clone();
        folders.push(
            row()
                .w_full()
                .min_h(40.0)
                .px(14.0)
                .gap(10.0)
                .items_center()
                .child(icon(&icons::FOLDER, 16.0).color(p.accent))
                .child(text(f.clone()).font(ty::BODY).color(p.text).truncate_middle().grow(1.0).min_w(0.0))
                .child(
                    rune_kit::Button::new(t("common.remove"))
                        .on_press(move |s: &mut Ui| {
                            let folder = folder.clone();
                            let rest: Vec<String> = s.settings.library_folders.iter().filter(|x| **x != folder).cloned().collect();
                            s.backend.store.patch(&json!({ "libraryFolders": rest }));
                            s.settings = s.backend.store.settings();
                            s.spawn_quiet(move |b| b.library.remove_folder(&folder));
                        })
                        .build(k),
                ),
        );
    }
    if folders.is_empty() {
        folders.push(line(p, t("set.noFolders"), None, el()));
    }
    let actions = row()
        .gap(8.0)
        .px(14.0)
        .py(10.0)
        .child(rune_kit::Button::new(t("set.addFolder")).on_press_tx(|s: &mut Ui, tx| s.choose_library_folder(tx)).build(k))
        .child(rune_kit::Button::new(t("set.scanNow")).on_press(|s: &mut Ui| s.rescan()).build(k));
    folders.push(actions);
    let dl = if s.download_dir.is_empty() {
        crate::paths::default_download_dir().to_string_lossy().into_owned()
    } else {
        s.download_dir.clone()
    };
    vec![
        group(p, t("set.folders"), folders, Some(t("set.foldersHint"))),
        group(
            p,
            t("set.downloadDir"),
            vec![line(
                p,
                dl,
                None,
                rune_kit::Button::new(t("common.change"))
                    .on_press_tx(|_s: &mut Ui, tx| {
                        let options = rune::FileDialogOptions { title: t("set.downloadDir"), ..Default::default() };
                        tx.open_folder(options, |s: &mut Ui, r, _| {
                            if let Ok(Some(path)) = r {
                                s.set(json!({ "downloadDir": path.to_string_lossy() }));
                            }
                        });
                    })
                    .build(k),
            )],
            None,
        ),
        group(
            p,
            t("set.data"),
            vec![line(
                p,
                ui.backend.data_dir.to_string_lossy().into_owned(),
                None,
                rune_kit::Button::new(t("about.dataDir"))
                    .on_press(|s: &mut Ui| crate::os::open_path(&s.backend.data_dir.to_string_lossy()))
                    .build(k),
            )],
            None,
        ),
    ]
}

fn streaming(ui: &Ui, p: &Palette, k: &Kit) -> Vec<El> {
    let s = &ui.settings;
    let key = TextInput::new(&ui.field_edit)
        .placeholder(t("set.tmdbKey"))
        .on_edit(|s: &mut Ui| &mut s.field_edit)
        .on_submit(|s: &mut Ui, _| {
            let v = s.field_edit.text.trim().to_string();
            s.set(json!({ "media": { "tmdbKey": v } }));
            s.toast(t("toast.saved"), "check");
        })
        .build(k)
        .w(280.0);
    vec![group(
        p,
        t("set.tab.streaming"),
        vec![
            line(p, t("set.tmdbKey"), Some(t("set.tmdbHint")), key),
            line(
                p,
                t("set.animeTitles"),
                None,
                segmented(
                    p,
                    "set-anime-titles",
                    vec![("romaji".into(), "Romaji".into()), ("english".into(), t("lang.en"))],
                    &s.media.anime_titles,
                    |s, v| s.set(json!({ "media": { "animeTitles": v } })),
                ),
            ),
            line(p, t("set.dubDefault"), Some(t("set.dubHint")), toggle(k, s.media.dub, |v| json!({ "media": { "dub": v } }))),
        ],
        None,
    )]
}

fn downloads(ui: &Ui, p: &Palette, k: &Kit) -> Vec<El> {
    let d = &ui.settings.dl;
    let conc = format!("{}", d.concurrency());
    vec![group(
        p,
        t("set.tab.downloads"),
        vec![
            line(
                p,
                t("set.concurrency"),
                None,
                segmented(p, "set-conc", (1..=4).map(|n| (n.to_string(), n.to_string())).collect(), &conc, |s, v| {
                    s.set(json!({ "dl": { "concurrency": v.parse::<i64>().unwrap_or(2) } }))
                }),
            ),
            line(p, t("set.autoUpdate"), Some(t("set.autoUpdateHint")), toggle(k, d.auto_update, |v| json!({ "dl": { "autoUpdate": v } }))),
            line(p, t("dl.createPlaylist"), None, toggle(k, d.create_playlist, |v| json!({ "dl": { "createPlaylist": v } }))),
        ],
        None,
    )]
}

fn about(ui: &Ui, p: &Palette, _k: &Kit) -> Vec<El> {
    let _ = ui;
    vec![group(
        p,
        t("set.tab.about"),
        vec![
            line(p, "control".into(), Some(crate::i18n::tv("about.version", &[("v", &env!("CARGO_PKG_VERSION"))])), el()),
            line(p, t("about.license"), Some("MIT · Rune (MIT) · Inter (OFL) · Lucide (ISC)".into()), el()),
        ],
        None,
    )]
}

pub fn equalizer(ui: &Ui, p: &Palette, k: &Kit) -> El {
    let eq = &ui.settings.eq;
    let mut bands = row().gap(10.0).h(200.0).items_end().justify_center();
    for (i, f) in EQ_BANDS.iter().enumerate() {
        let g = eq.gains.get(i).copied().unwrap_or(0.0);
        let h = 160.0;
        let frac = ((g + 12.0) / 24.0).clamp(0.0, 1.0) as f32;
        let label = if *f >= 1000 { format!("{}k", f / 1000) } else { f.to_string() };
        bands = bands.child(
            col()
                .gap(6.0)
                .items_center()
                .child(text(format!("{g:+.0}")).font(ty::NUMBERS).color(p.text2))
                .child(
                    el().key(format!("eq-{i}"))
                        .w(28.0)
                        .h(h)
                        .cursor(Cursor::Pointer)
                        .role("slider")
                        .label(format!("{label} Hz"))
                        .range_value(g, -12.0, 12.0)
                        .child(el().absolute(12.0, 0.0).w(4.0).h(h).radius(2.0).fill(p.control))
                        .child(el().absolute(4.0, h * (1.0 - frac) - 10.0).size(20.0, 20.0).radius(10.0).fill(p.accent))
                        .on_drag(move |s: &mut Ui, d: &mut Drag, _| {
                            if d.h > 0.0 {
                                let v = (((1.0 - (d.y / d.h).clamp(0.0, 1.0)) * 24.0 - 12.0) as f64).round();
                                let mut gains = s.settings.eq.gains.clone();
                                gains[i] = v;
                                s.set(json!({ "eq": { "gains": gains, "preset": "custom", "enabled": true } }));
                            }
                        }),
                )
                .child(text(label).font(ty::CAPTION).color(p.text3)),
        );
    }
    let mut presets = row().gap(6.0).flex_wrap();
    for (name, gains) in EQ_PRESETS {
        let on = eq.preset == *name;
        let n = name.to_string();
        let g = gains.to_vec();
        presets = presets.child(
            row()
                .key(format!("eqp-{name}"))
                .h(28.0)
                .px(10.0)
                .radius(14.0)
                .center()
                .fill(if on { p.accent } else { p.fill })
                .cursor(Cursor::Pointer)
                .child(text(t(&format!("eq.{name}"))).font(ty::SMALL_MEDIUM).color(if on { p.on_accent } else { p.text }))
                .on_press(move |s: &mut Ui| s.set(json!({ "eq": { "preset": n, "gains": g, "enabled": true } }))),
        );
    }
    col()
        .w_full()
        .gap(16.0)
        .child(line(p, t("eq.enable"), None, toggle(k, eq.enabled, |v| json!({ "eq": { "enabled": v } }))))
        .child(bands.opacity(if eq.enabled { 1.0 } else { 0.5 }))
        .child(presets)
}

pub fn sheet(ui: &Ui, p: &Palette, k: &Kit, layer: &mut Vec<El>) {
    let Some(sheet) = &ui.sheet else { return };
    if *sheet == Sheet::Equalizer {
        layer.push(crate::sheets::panel(p, "sheet-eq", t("eq.title"), 600.0, 460.0, None, equalizer(ui, p, k)));
        return;
    }
    let Sheet::Settings(tab) = sheet else { return };
    let tab = *tab;
    let mut nav = row().w_full().gap(4.0).justify_center().pb(12.0);
    for (id, name, glyph) in TABS {
        let on = *id == tab;
        let target = *id;
        nav = nav.child(
            col()
                .key(format!("set-tab-{name}"))
                .w(84.0)
                .py(6.0)
                .gap(4.0)
                .items_center()
                .radius(p.r(8.0))
                .cursor(Cursor::Pointer)
                .role("tab")
                .checked(on)
                .fill(if on { p.fill_strong } else { Color::TRANSPARENT })
                .hover(p.fill)
                .child(icon(glyph, 22.0).color(if on { p.accent } else { p.text2 }))
                .child(text(t(&format!("set.tab.{name}"))).font(ty::SMALL_MEDIUM).color(if on { p.text } else { p.text2 }))
                .on_press(move |s: &mut Ui| {
                    if target == SettingsTab::Streaming {
                        s.field_edit = rune::TextEdit::new(s.settings.media.tmdb_key.clone());
                    }
                    s.sheet = Some(Sheet::Settings(target));
                }),
        );
    }
    let groups = match tab {
        SettingsTab::General => general(ui, p, k),
        SettingsTab::Appearance => appearance(ui, p, k),
        SettingsTab::Library => library(ui, p, k),
        SettingsTab::Streaming => streaming(ui, p, k),
        SettingsTab::Downloads => downloads(ui, p, k),
        SettingsTab::About => about(ui, p, k),
    };
    layer.push(crate::sheets::panel(p, "sheet-settings", t("nav.settings"), 680.0, 720.0, Some(nav), col().w_full().children(groups)));
}

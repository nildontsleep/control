//! Couleurs de l'interface, reprises des variables CSS d'origine (thèmes sombre, clair et OLED, accents macOS).

use rune::Color;

use control_core::settings::Settings;

/// Couleur sRGB (0–255) avec opacité, comme `rgb(r g b / a)` en CSS.
pub fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    let lin = |c: u8| rune::color::srgb_decode(c as f32 / 255.0);
    Color { r: lin(r), g: lin(g), b: lin(b), a }
}

pub fn hex(s: &str) -> Option<Color> {
    rune::color::hex(s)
}

/// Accents macOS : (nom, sombre, clair).
pub const ACCENTS: &[(&str, &str, &str)] = &[
    ("blue", "#0a84ff", "#007aff"),
    ("purple", "#bf5af2", "#a550d8"),
    ("pink", "#ff375f", "#f2295a"),
    ("red", "#ff453a", "#ff3b30"),
    ("orange", "#ff9f0a", "#f08c00"),
    ("yellow", "#ffd60a", "#e6b800"),
    ("green", "#30d158", "#28b745"),
    ("graphite", "#98989d", "#8e8e93"),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub oled: bool,
    pub bg: Color,
    pub sidebar: Color,
    pub bar: Color,
    pub veil: Color,
    pub text: Color,
    pub text2: Color,
    pub text3: Color,
    pub panel: Color,
    pub panel_solid: Color,
    pub panel_border: Color,
    pub group: Color,
    pub separator: Color,
    pub fill: Color,
    pub fill_hover: Color,
    pub fill_strong: Color,
    pub control: Color,
    pub seg_thumb: Color,
    pub menu: Color,
    pub accent: Color,
    pub accent_soft: Color,
    pub on_accent: Color,
    pub danger: Color,
    pub success: Color,
    /// Rayon des coins, multiplié par le réglage « Arrondi ».
    pub radius_k: f32,
    /// Opacité des panneaux translucides (réglage « Translucidité »).
    pub tl: f32,
}

impl Palette {
    pub fn from_settings(s: &Settings, system_dark: bool) -> Palette {
        let oled = s.theme == "oled";
        let dark = match s.theme.as_str() {
            "light" => false,
            "dark" | "oled" => true,
            _ => system_dark,
        };
        let tl = (s.translucency / 100.0).clamp(0.3, 1.5) as f32;
        let accent_hex = if s.accent == "custom" {
            s.custom_accent.clone()
        } else {
            let (_, d, l) = ACCENTS.iter().find(|(n, _, _)| *n == s.accent).copied().unwrap_or(ACCENTS[2]);
            (if dark { d } else { l }).to_string()
        };
        let accent = hex(&accent_hex).unwrap_or_else(|| rgba(10, 132, 255, 1.0));
        // Texte sur l'accent : noir sur un accent clair (jaune, graphite clair…), blanc sinon.
        let on_accent = if accent.luminance() > 0.45 { rgba(0, 0, 0, 0.88) } else { Color::WHITE };
        let radius_k = (s.radius / 100.0).clamp(0.0, 2.0) as f32;
        if dark {
            let depth = (s.oled_level / 100.0).clamp(0.0, 1.0) as f32;
            let bg = if oled { rgba(0, 0, 0, 1.0).mix(rgba(13, 13, 16, 1.0), depth) } else { rgba(13, 13, 16, 1.0) };
            Palette {
                dark,
                oled,
                bg,
                sidebar: rgba(30, 30, 36, (0.62 * tl).min(1.0)),
                bar: rgba(36, 36, 42, (0.66 * tl).min(1.0)),
                veil: rgba(13, 13, 16, 0.55),
                text: rgba(255, 255, 255, 0.92),
                text2: rgba(235, 235, 245, 0.6),
                text3: rgba(235, 235, 245, 0.34),
                panel: rgba(32, 32, 36, (0.66 * tl).min(1.0)),
                panel_solid: rgba(38, 38, 42, 1.0),
                panel_border: rgba(255, 255, 255, 0.11),
                group: rgba(255, 255, 255, 0.05),
                separator: rgba(255, 255, 255, 0.08),
                fill: rgba(255, 255, 255, 0.07),
                fill_hover: rgba(255, 255, 255, 0.12),
                fill_strong: rgba(255, 255, 255, 0.15),
                control: rgba(255, 255, 255, 0.16),
                seg_thumb: rgba(255, 255, 255, 0.22),
                menu: rgba(44, 44, 50, (0.92 * tl).min(1.0)),
                accent,
                accent_soft: accent.alpha(0.2),
                on_accent,
                danger: rgba(255, 69, 58, 1.0),
                success: rgba(48, 209, 88, 1.0),
                radius_k,
                tl,
            }
        } else {
            Palette {
                dark,
                oled: false,
                bg: rgba(243, 243, 246, 1.0),
                sidebar: rgba(236, 236, 242, (0.7 * tl).min(1.0)),
                bar: rgba(250, 250, 252, (0.74 * tl).min(1.0)),
                veil: rgba(243, 243, 246, 0.5),
                text: rgba(0, 0, 0, 0.86),
                text2: rgba(60, 60, 67, 0.68),
                text3: rgba(60, 60, 67, 0.42),
                panel: rgba(246, 246, 249, (0.76 * tl).min(1.0)),
                panel_solid: rgba(244, 244, 247, 1.0),
                panel_border: rgba(0, 0, 0, 0.1),
                group: rgba(255, 255, 255, 0.68),
                separator: rgba(0, 0, 0, 0.075),
                fill: rgba(0, 0, 0, 0.05),
                fill_hover: rgba(0, 0, 0, 0.08),
                fill_strong: rgba(0, 0, 0, 0.11),
                control: rgba(0, 0, 0, 0.11),
                seg_thumb: Color::WHITE,
                menu: rgba(248, 248, 250, (0.95 * tl).min(1.0)),
                accent,
                accent_soft: accent.alpha(0.18),
                on_accent,
                danger: rgba(255, 59, 48, 1.0),
                success: rgba(40, 183, 69, 1.0),
                radius_k,
                tl,
            }
        }
    }

    /// Un rayon de la maquette, ajusté au réglage d'arrondi.
    pub fn r(&self, base: f32) -> f32 {
        base * self.radius_k
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_follows_settings() {
        let mut s = Settings::default();
        assert!(Palette::from_settings(&s, true).dark);
        assert!(!Palette::from_settings(&s, false).dark);
        s.theme = "oled".into();
        let p = Palette::from_settings(&s, false);
        assert!(p.dark && p.oled);
        s.accent = "custom".into();
        s.custom_accent = "#ffd60a".into();
        let p = Palette::from_settings(&s, false);
        assert_eq!(p.on_accent, rgba(0, 0, 0, 0.88));
    }
}

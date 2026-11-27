//! Colour themes, fonts and the egui style that goes with them. A dashboard stores its theme
//! name in the layout, so every screen showing it (wall displays, kiosks) looks the same.

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, CornerRadius, Stroke, TextStyle};

/// Colours for one theme. Everything the page paints comes from here.
#[derive(Clone, Copy, PartialEq)]
pub struct Pal {
    pub dark: bool,
    /// Page background, top and bottom of its vertical gradient.
    pub bg: Color32,
    pub bg2: Color32,
    /// Tile / panel fill and the slightly raised variant (headers, inputs).
    pub panel: Color32,
    pub panel2: Color32,
    pub line: Color32,
    pub text: Color32,
    /// Secondary text: tile titles, units. Readable, but quieter than values.
    pub soft: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub alarm: Color32,
    /// Low-limit zones: blue for "warn below", purple for "alarm below".
    pub warn_lo: Color32,
    pub alarm_lo: Color32,
    pub shadow: Color32,
}

pub struct ThemeInfo {
    pub key: &'static str,
    pub name: &'static str,
}

/// Selectable themes, in menu order. `""` = follow the system light / dark setting.
pub const THEMES: [ThemeInfo; 8] = [
    ThemeInfo { key: "", name: "Auto (system)" },
    ThemeInfo { key: "midnight", name: "Midnight" },
    ThemeInfo { key: "graphite", name: "Graphite" },
    ThemeInfo { key: "aurora", name: "Aurora" },
    ThemeInfo { key: "ocean", name: "Ocean" },
    ThemeInfo { key: "ember", name: "Ember" },
    ThemeInfo { key: "daylight", name: "Daylight" },
    ThemeInfo { key: "paper", name: "Paper" },
];

pub fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

#[allow(clippy::too_many_arguments)]
fn dark(bg: u32, bg2: u32, panel: u32, panel2: u32, line: u32, text: u32, soft: u32, muted: u32, accent: u32) -> Pal {
    Pal {
        dark: true,
        bg: rgb(bg),
        bg2: rgb(bg2),
        panel: rgb(panel),
        panel2: rgb(panel2),
        line: rgb(line),
        text: rgb(text),
        soft: rgb(soft),
        muted: rgb(muted),
        accent: rgb(accent),
        ok: rgb(0x34d399),
        warn: rgb(0xfbbf24),
        alarm: rgb(0xf87171),
        warn_lo: rgb(0x5b8cff),
        alarm_lo: rgb(0xd946ef),
        shadow: Color32::from_black_alpha(110),
    }
}

#[allow(clippy::too_many_arguments)]
fn light(bg: u32, bg2: u32, panel: u32, panel2: u32, line: u32, text: u32, soft: u32, muted: u32, accent: u32) -> Pal {
    Pal {
        dark: false,
        bg: rgb(bg),
        bg2: rgb(bg2),
        panel: rgb(panel),
        panel2: rgb(panel2),
        line: rgb(line),
        text: rgb(text),
        soft: rgb(soft),
        muted: rgb(muted),
        accent: rgb(accent),
        ok: rgb(0x059669),
        warn: rgb(0xd97706),
        alarm: rgb(0xdc2626),
        warn_lo: rgb(0x1d4ed8),
        alarm_lo: rgb(0xa21caf),
        shadow: Color32::from_black_alpha(28),
    }
}

impl Pal {
    /// Palette for a theme key; unknown keys and `""` follow the system setting.
    pub fn of(key: &str, system_dark: bool) -> Pal {
        match key {
            "midnight" => dark(0x0b1120, 0x111a33, 0x151e33, 0x1c2741, 0x26324d, 0xf1f5f9, 0xb4c0d3, 0x7b8aa3, 0x38bdf8),
            "graphite" => dark(0x111214, 0x18191c, 0x1c1d21, 0x24262b, 0x2f3137, 0xf4f4f5, 0xc4c4cc, 0x8a8a94, 0xf59e0b),
            "aurora" => dark(0x0e0b1f, 0x16102e, 0x1a1533, 0x231c42, 0x302752, 0xf5f3ff, 0xc9c1e8, 0x8e84b3, 0xa78bfa),
            "ocean" => dark(0x041a22, 0x06242e, 0x0a2a35, 0x0f3542, 0x174452, 0xecfeff, 0xa9d6dc, 0x6f9fa8, 0x2dd4bf),
            "ember" => dark(0x170d0a, 0x21120d, 0x24160f, 0x2e1d14, 0x3d281c, 0xfff7ed, 0xe0c3ad, 0xa88a75, 0xfb923c),
            "daylight" => light(0xeef2f8, 0xe2e8f2, 0xffffff, 0xf5f7fb, 0xdde3ec, 0x0f172a, 0x475569, 0x7c8799, 0x2563eb),
            "paper" => light(0xf6f1e9, 0xece4d6, 0xfffdf8, 0xf8f3ea, 0xe4dccd, 0x292524, 0x57534e, 0x8a837a, 0x0f766e),
            _ if system_dark => Pal::of("midnight", true),
            _ => Pal::of("daylight", false),
        }
    }
}

/// The bold font family (values, headings).
pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn bold_id(size: f32) -> FontId {
    FontId::new(size, bold())
}

/// The app's fonts (egui's built-in set is switched off to save size):
/// * Noto Sans Medium for text and Noto Sans Bold for values and headings, both trimmed to Latin
///   letters plus common technical symbols (°, µ, Ω, ±, ≤, →, € …) — see `assets/`.
/// * egui's two emoji / symbol fonts as fallbacks for anything else.
pub fn install_fonts(ctx: &egui::Context) {
    use egui::epaint::text::FontTweak;
    use std::sync::Arc;
    let mut fonts = FontDefinitions::empty();
    let data = |bytes: &'static [u8], scale: f32| Arc::new(FontData::from_static(bytes).tweak(FontTweak { scale, ..Default::default() }));
    fonts.font_data.insert("noto".into(), data(include_bytes!("../assets/NotoSans-Medium.ttf"), 1.0));
    fonts.font_data.insert("noto-bold".into(), data(include_bytes!("../assets/NotoSans-Bold.ttf"), 1.0));
    // Same tweaks egui gives these two when it ships them itself.
    fonts.font_data.insert("emoji".into(), data(include_bytes!("../assets/NotoEmoji-Regular.ttf"), 0.81));
    fonts.font_data.insert("emoji-icons".into(), data(include_bytes!("../assets/emoji-icon-font.ttf"), 0.90));
    let chain = |first: &str| vec![first.to_string(), "emoji".to_string(), "emoji-icons".to_string()];
    fonts.families.insert(FontFamily::Proportional, chain("noto"));
    fonts.families.insert(FontFamily::Monospace, chain("noto"));
    fonts.families.insert(bold(), chain("noto-bold"));
    ctx.set_fonts(fonts);
}

/// egui widget style matching the palette (buttons, menus, inputs, windows).
pub fn apply_style(ctx: &egui::Context, pal: &Pal) {
    let mut v = if pal.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let r = CornerRadius::from(8.0);
    v.panel_fill = pal.panel;
    v.window_fill = pal.panel;
    v.window_stroke = Stroke::new(1.0, pal.line);
    v.window_corner_radius = CornerRadius::same(14);
    v.window_shadow = egui::epaint::Shadow { offset: [0, 12], blur: 32, spread: 0, color: pal.shadow };
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 8], blur: 20, spread: 0, color: pal.shadow };
    v.menu_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = pal.bg;
    v.faint_bg_color = pal.panel2;
    v.hyperlink_color = pal.accent;
    v.selection.bg_fill = pal.accent.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, pal.accent);
    v.override_text_color = None;
    for (w, fill, stroke) in [
        (&mut v.widgets.noninteractive, pal.panel, pal.line),
        (&mut v.widgets.inactive, pal.panel2, pal.line),
        (&mut v.widgets.hovered, pal.line, pal.accent.gamma_multiply(0.6)),
        (&mut v.widgets.active, pal.accent.gamma_multiply(0.3), pal.accent),
        (&mut v.widgets.open, pal.panel2, pal.accent.gamma_multiply(0.6)),
    ] {
        w.corner_radius = r;
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0, stroke);
    }
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, pal.soft);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, pal.text);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, pal.text);
    v.widgets.active.fg_stroke = Stroke::new(1.5, pal.text);
    v.widgets.open.fg_stroke = Stroke::new(1.0, pal.text);
    ctx.all_styles_mut(|s| {
        s.visuals = v.clone();
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.menu_margin = egui::Margin::same(8);
        s.spacing.interact_size.y = 30.0;
        s.text_styles.insert(TextStyle::Body, FontId::proportional(15.0));
        s.text_styles.insert(TextStyle::Button, FontId::proportional(15.0));
        s.text_styles.insert(TextStyle::Small, FontId::proportional(11.5));
        s.text_styles.insert(TextStyle::Heading, bold_id(22.0));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_resolves_and_auto_follows_the_system() {
        for t in &THEMES {
            let p = Pal::of(t.key, true);
            assert_ne!(p.bg, p.text, "{}", t.name);
        }
        assert!(Pal::of("", true).dark && !Pal::of("", false).dark);
        assert!(Pal::of("nonsense", true) == Pal::of("midnight", true));
        assert!(!Pal::of("paper", true).dark, "a light theme stays light on a dark system");
    }
}

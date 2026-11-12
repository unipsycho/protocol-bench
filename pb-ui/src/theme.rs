//! Protocol Bench palettes. Copper is the product theme; the others are alternates.

use egui::{Color32, FontFamily, FontId, Rounding, Stroke, TextStyle, Visuals};

pub const MIN_PANE: f32 = 168.0;
pub const SEP: f32 = 8.0;
pub const RADIUS: f32 = 8.0;
pub const HEADER_H: f32 = 42.0;
pub const BAR_H: f32 = 56.0;
pub const STATUS_H: f32 = 30.0;
pub const RAIL_W: f32 = 56.0;
/// Edge band used when the pointer is not on a dock pad.
pub const EDGE_BAND: f32 = 0.26;
pub const DROP_THRESHOLD: f32 = 5.0;

/// Protocol inks. Core colors sit on the dark scope; deep colors sit on light panes.
pub struct Proto;

impl Proto {
    pub const COPPER: Color32 = Color32::from_rgb(0xe0, 0x7a, 0x3d);
    pub const COPPER_DEEP: Color32 = Color32::from_rgb(0x9a, 0x43, 0x14);
    pub const TEAL: Color32 = Color32::from_rgb(0x3d, 0xce, 0xc0);
    pub const TEAL_DEEP: Color32 = Color32::from_rgb(0x0f, 0x6e, 0x68);
    pub const INDIGO: Color32 = Color32::from_rgb(0x9a, 0xa8, 0xff);
    pub const INDIGO_DEEP: Color32 = Color32::from_rgb(0x31, 0x40, 0xa8);
    pub const SAGE: Color32 = Color32::from_rgb(0x8f, 0xd0, 0xa4);
    pub const SAGE_DEEP: Color32 = Color32::from_rgb(0x1f, 0x6b, 0x45);
    pub const STREAMS: Color32 = Color32::from_rgb(0x79, 0xb8, 0xff);
    pub const STREAMS_DEEP: Color32 = Color32::from_rgb(0x1a, 0x5f, 0xad);
    pub const MODBUS: Color32 = Color32::from_rgb(0xf0, 0xb4, 0x29);
    pub const MODBUS_DEEP: Color32 = Color32::from_rgb(0x8a, 0x5a, 0x10);
    pub const MQTT: Color32 = Color32::from_rgb(0x2e, 0xc9, 0xb8);
    pub const MQTT_DEEP: Color32 = Color32::from_rgb(0x0e, 0x75, 0x6c);
    pub const OPC: Color32 = Color32::from_rgb(0xc4, 0xb5, 0xfd);
    pub const OPC_DEEP: Color32 = Color32::from_rgb(0x5b, 0x3d, 0x9e);
    pub const FILES: Color32 = Color32::from_rgb(0xe4, 0xc0, 0x7a);
    pub const FILES_DEEP: Color32 = Color32::from_rgb(0x8a, 0x5a, 0x20);
}

/// Dark instrument face used by the hero graphics on every theme.
pub mod scope {
    use super::Color32;
    pub const BG: Color32 = Color32::from_rgb(0x0c, 0x12, 0x18);
    pub fn grid() -> Color32 {
        Color32::from_rgba_unmultiplied(150, 180, 200, 24)
    }
    pub const TEXT: Color32 = Color32::from_rgb(0xc5, 0xd4, 0xe4);
}

/// Scope blue mixed halfway with the theme background. The well stays blue and follows the theme.
pub fn blend_scope(theme_bg: Color32) -> Color32 {
    mix(theme_bg, scope::BG, 0.5)
}

/// `t` is the share of `b`. 0 keeps `a`, 1 keeps `b`.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    Color32::from_rgb(
        (a.r() as f32 * inv + b.r() as f32 * t).round() as u8,
        (a.g() as f32 * inv + b.g() as f32 * t).round() as u8,
        (a.b() as f32 * inv + b.b() as f32 * t).round() as u8,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTheme {
    Copper,
    Ink,
    Daylight,
    Harbor,
    Ember,
    Signal,
    Frost,
}

impl AppTheme {
    pub const ALL: [AppTheme; 7] = [
        AppTheme::Copper,
        AppTheme::Ink,
        AppTheme::Daylight,
        AppTheme::Harbor,
        AppTheme::Ember,
        AppTheme::Signal,
        AppTheme::Frost,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Copper => "copper",
            Self::Ink => "ink",
            Self::Daylight => "daylight",
            Self::Harbor => "harbor",
            Self::Ember => "ember",
            Self::Signal => "signal",
            Self::Frost => "frost",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Copper => "Copper",
            Self::Ink => "Ink",
            Self::Daylight => "Daylight",
            Self::Harbor => "Harbor",
            Self::Ember => "Ember",
            Self::Signal => "Signal",
            Self::Frost => "Frost",
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "ink" => Self::Ink,
            "daylight" => Self::Daylight,
            "harbor" => Self::Harbor,
            "ember" => Self::Ember,
            "signal" => Self::Signal,
            "frost" => Self::Frost,
            _ => Self::Copper,
        }
    }

    pub fn palette(self) -> ThemePalette {
        match self {
            Self::Copper => copper(),
            Self::Ink => ink(),
            Self::Daylight => daylight(),
            Self::Harbor => harbor(),
            Self::Ember => ember(),
            Self::Signal => signal(),
            Self::Frost => frost(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ThemePalette {
    pub bg: Color32,
    pub bar: Color32,
    pub pane: Color32,
    pub header_top: Color32,
    pub header_bot: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub ok: Color32,
    pub live: Color32,
    pub warn: Color32,
    pub err: Color32,
    pub control: Color32,
    pub line: Color32,
    pub is_light: bool,
}

impl ThemePalette {
    pub fn swatch_colors(self) -> [Color32; 6] {
        [
            self.pane,
            self.accent,
            self.ok,
            self.live,
            self.warn,
            self.err,
        ]
    }
}

pub fn tint(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

pub fn font_ui(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn font_semi(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("pb_semi".into()))
}

pub fn font_mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

fn pack(
    bg: &str,
    bar: &str,
    pane: &str,
    header_top: &str,
    header_bot: &str,
    text: &str,
    muted: &str,
    accent: &str,
    ok: &str,
    live: &str,
    warn: &str,
    err: &str,
    control: &str,
    is_light: bool,
) -> ThemePalette {
    ThemePalette {
        bg: hex(bg),
        bar: hex(bar),
        pane: hex(pane),
        header_top: hex(header_top),
        header_bot: hex(header_bot),
        text: hex(text),
        muted: hex(muted),
        accent: hex(accent),
        ok: hex(ok),
        live: hex(live),
        warn: hex(warn),
        err: hex(err),
        control: hex(control),
        line: if is_light {
            Color32::from_rgba_unmultiplied(20, 24, 32, 36)
        } else {
            Color32::from_rgba_unmultiplied(255, 248, 240, 28)
        },
        is_light,
    }
}

fn hex(s: &str) -> Color32 {
    let s = s.trim().trim_start_matches('#');
    let n = u32::from_str_radix(s, 16).unwrap_or(0);
    Color32::from_rgb(
        ((n >> 16) & 0xff) as u8,
        ((n >> 8) & 0xff) as u8,
        (n & 0xff) as u8,
    )
}

fn copper() -> ThemePalette {
    pack(
        "#0e141c", "#151c27", "#1c2633", "#243244", "#1a2430", "#f4efe6", "#93a0b5", "#e07a3d",
        "#7dcea0", "#3dcec0", "#f0b429", "#ff6b6b", "#243044", false,
    )
}

fn ink() -> ThemePalette {
    pack(
        "#070b12", "#0e1522", "#141c2b", "#1c283c", "#121a28", "#e7eef8", "#8ea0b8", "#3dbeff",
        "#6dce9a", "#5eead4", "#f0b429", "#ff7a8a", "#1a2536", false,
    )
}

fn daylight() -> ThemePalette {
    pack(
        "#e7e1d6", "#f6f1e8", "#fbf7f1", "#efe4d4", "#f3ece3", "#1c1915", "#6f675c", "#c4622d",
        "#2f6b45", "#0f766e", "#a16207", "#b42318", "#fffdfb", true,
    )
}

fn harbor() -> ThemePalette {
    pack(
        "#0c1618", "#122124", "#182c30", "#1e383d", "#163034", "#e7f4f2", "#8eaaa8", "#e0a15a",
        "#7dcea0", "#5ee0c7", "#f0b429", "#ff7a7a", "#1c3338", false,
    )
}

fn ember() -> ThemePalette {
    pack(
        "#140e0c", "#1d1411", "#271c18", "#34241e", "#221814", "#f8efe8", "#b3a096", "#ff7a45",
        "#8dce8a", "#5ee0c7", "#ffb020", "#ff5d5d", "#2e221c", false,
    )
}

fn signal() -> ThemePalette {
    pack(
        "#0c0d0c", "#141514", "#1c1e1c", "#262826", "#1a1c1a", "#f5f6f2", "#a1a39c", "#ff8a3d",
        "#9ddeaf", "#7ee0c8", "#ffbf3c", "#ff5c5c", "#222422", false,
    )
}

fn frost() -> ThemePalette {
    pack(
        "#dfe6ee", "#f4f7fb", "#fbfdff", "#e5eef8", "#eef3f8", "#142033", "#5c6b80", "#1f6feb",
        "#1f7a4d", "#0f766e", "#9a6700", "#b42318", "#ffffff", true,
    )
}

/// Spacing and text styles. Call once; theme switches only replace visuals.
pub fn install_chrome_style(ctx: &egui::Context) {
    ctx.style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(10.0, 6.0);
        style.spacing.window_margin = egui::Margin::same(SEP);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.spacing.interact_size = egui::vec2(32.0, 28.0);
        style.text_styles.insert(TextStyle::Body, font_ui(13.5));
        style.text_styles.insert(TextStyle::Button, font_ui(13.0));
        style
            .text_styles
            .insert(TextStyle::Heading, font_semi(20.0));
        style
            .text_styles
            .insert(TextStyle::Monospace, font_mono(12.5));
        style.text_styles.insert(TextStyle::Small, font_ui(11.5));
    });
}

/// Push one palette into egui. Does not touch spacing.
pub fn apply_theme(ctx: &egui::Context, theme: AppTheme) {
    let p = theme.palette();
    let mut visuals = if p.is_light {
        Visuals::light()
    } else {
        Visuals::dark()
    };
    visuals.panel_fill = p.pane;
    visuals.window_fill = p.bar;
    visuals.extreme_bg_color = p.bg;
    visuals.faint_bg_color = p.header_bot;
    visuals.widgets.noninteractive.bg_fill = p.pane;
    visuals.widgets.inactive.bg_fill = p.control;
    visuals.widgets.hovered.bg_fill = p.header_top;
    visuals.widgets.active.bg_fill = p.header_bot;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, p.line);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.muted);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.4, p.text);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.4, p.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.6, p.accent);
    visuals.selection.bg_fill = tint(p.accent, 40);
    visuals.selection.stroke = Stroke::new(1.0, p.accent);
    visuals.hyperlink_color = p.accent;
    visuals.warn_fg_color = p.warn;
    visuals.error_fg_color = p.err;
    visuals.override_text_color = Some(p.text);
    visuals.window_rounding = Rounding::same(10.0);
    visuals.menu_rounding = Rounding::same(8.0);
    visuals.widgets.noninteractive.rounding = Rounding::same(6.0);
    visuals.widgets.inactive.rounding = Rounding::same(6.0);
    visuals.widgets.hovered.rounding = Rounding::same(6.0);
    visuals.widgets.active.rounding = Rounding::same(6.0);
    ctx.set_visuals(visuals);
}

/// Warm wash behind the dock. Pane fills cover most of it; the gutters show the rest.
pub fn paint_root_bg(ctx: &egui::Context, p: &ThemePalette) {
    let screen = ctx.screen_rect();
    let painter = ctx.layer_painter(egui::LayerId::background());
    painter.rect_filled(screen, Rounding::ZERO, p.bg);
    let (a0, a1, a2, a3) = if p.is_light {
        (10, 14, 18, 8)
    } else {
        (16, 22, 28, 14)
    };
    let copper_at = egui::pos2(screen.min.x + screen.width() * 0.16, screen.min.y + 10.0);
    for (radius, alpha) in [(420.0_f32, a0), (240.0, a1), (120.0, a2)] {
        painter.circle_filled(copper_at, radius, tint(p.accent, alpha));
    }
    let teal_at = egui::pos2(screen.max.x - 60.0, screen.max.y - 20.0);
    painter.circle_filled(teal_at, 280.0, tint(p.live, a3));
}

/// Inter + JetBrains Mono, SIL Open Font License, bundled so the face does not depend on the system.
pub fn install_fonts(ctx: &egui::Context) {
    const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
    const INTER_SEMI: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
    const JET: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");

    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("inter".into(), egui::FontData::from_static(INTER));
    fonts.font_data.insert(
        "inter_semibold".into(),
        egui::FontData::from_static(INTER_SEMI),
    );
    fonts
        .font_data
        .insert("jetbrains".into(), egui::FontData::from_static(JET));

    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "inter".into());
    fonts.families.insert(
        FontFamily::Name("pb_semi".into()),
        vec!["inter_semibold".into()],
    );
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "jetbrains".into());
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_ids_roundtrip() {
        for theme in AppTheme::ALL {
            assert_eq!(AppTheme::from_id(theme.id()), theme);
            assert_ne!(theme.palette().text, theme.palette().bg);
        }
        assert_eq!(AppTheme::from_id("nope"), AppTheme::Copper);
    }

    #[test]
    fn scope_blue_mixes_halfway_with_the_theme_background() {
        let theme = Color32::from_rgb(0xe7, 0xe1, 0xd6);
        let blended = blend_scope(theme);
        let mid = |a: u8, b: u8| ((a as f32 + b as f32) * 0.5).round() as u8;
        assert_eq!(blended.r(), mid(theme.r(), scope::BG.r()));
        assert_eq!(blended.g(), mid(theme.g(), scope::BG.g()));
        assert_eq!(blended.b(), mid(theme.b(), scope::BG.b()));
        assert_ne!(blended, scope::BG);
        assert_ne!(blended, theme);
    }
}

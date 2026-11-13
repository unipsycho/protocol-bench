//! Protocol Bench palettes. Copper is the product theme; the others are alternates.

use egui::{Color32, FontFamily, FontId, Rect, Rounding, Stroke, TextStyle, Vec2, Visuals};

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

/// A 2px bar centered in the gap, pulled in from both ends so it misses the rounded corners.
pub fn divider_bar(gap: Rect, vertical_bar: bool) -> Rect {
    const THICK: f32 = 2.0;
    const INSET: f32 = 24.0;
    if vertical_bar {
        let inset = INSET.min(((gap.height() - 16.0) * 0.5).max(0.0));
        let length = (gap.height() - inset * 2.0).max(THICK);
        Rect::from_center_size(gap.center(), Vec2::new(THICK, length))
    } else {
        let inset = INSET.min(((gap.width() - 16.0) * 0.5).max(0.0));
        let length = (gap.width() - inset * 2.0).max(THICK);
        Rect::from_center_size(gap.center(), Vec2::new(length, THICK))
    }
}

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
    /// Diagram / hero grid stroke. Light themes stay near the well so orange/green ink remains visible.
    pub fn grid(is_light: bool) -> Color32 {
        if is_light {
            // Soft dark lines, low alpha — closer to the blended background than the old pale grid.
            Color32::from_rgba_unmultiplied(18, 36, 52, 14)
        } else {
            Color32::from_rgba_unmultiplied(150, 180, 200, 24)
        }
    }
    pub const TEXT: Color32 = Color32::from_rgb(0xc5, 0xd4, 0xe4);
}

/// Scope blue mixed into the theme background for hub / protocol diagram wells.
/// Light themes stay mostly the theme bg (clearly lighter); dark themes keep the half-blend.
pub fn blend_scope(theme_bg: Color32, is_light: bool) -> Color32 {
    if is_light {
        // ~12% scope blue — cool tint without a dark slate well on Frost/Daylight.
        mix(theme_bg, scope::BG, 0.12)
    } else {
        mix(theme_bg, scope::BG, 0.5)
    }
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

/// How far each corner wash reaches. Under half of either side, so the two washes do not meet.
pub fn corner_glow_radius(screen: egui::Vec2) -> f32 {
    screen.x.min(screen.y) * 0.46
}

/// Warm wash behind the dock. Pane fills cover most of it; the gutters show the rest.
pub fn paint_root_bg(ctx: &egui::Context, p: &ThemePalette) {
    let screen = ctx.screen_rect();
    let painter = ctx.layer_painter(egui::LayerId::background());
    painter.rect_filled(screen, Rounding::ZERO, p.bg);
    let radius = corner_glow_radius(screen.size());
    let peak = tint(p.accent, if p.is_light { 40 } else { 72 });
    // Same accent wash in both corners. A vertex mesh interpolates it, so it does not band.
    paint_corner_glow(
        &painter,
        screen.left_top(),
        radius,
        peak,
        0.0,
        std::f32::consts::FRAC_PI_2,
    );
    paint_corner_glow(
        &painter,
        screen.right_bottom(),
        radius,
        peak,
        std::f32::consts::PI,
        std::f32::consts::FRAC_PI_2,
    );
}

/// Quarter-circle radial gradient. Rings sample a cosine falloff; the GPU fills between them.
fn paint_corner_glow(
    painter: &egui::Painter,
    origin: egui::Pos2,
    radius: f32,
    peak: Color32,
    start: f32,
    sweep: f32,
) {
    const RINGS: usize = 6;
    const STEPS: usize = 56;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(origin, peak);
    let mut inner = 0u32;
    let mut inner_is_point = true;
    for ring in 1..=RINGS {
        let d = ring as f32 / RINGS as f32;
        let fade = 0.5 * (1.0 + (std::f32::consts::PI * d).cos());
        let color = peak.gamma_multiply(fade);
        let reach = radius * d;
        let first = mesh.vertices.len() as u32;
        for step in 0..=STEPS {
            let ang = start + sweep * (step as f32 / STEPS as f32);
            mesh.colored_vertex(origin + egui::vec2(ang.cos(), ang.sin()) * reach, color);
        }
        if inner_is_point {
            for step in 0..STEPS as u32 {
                mesh.add_triangle(inner, first + step, first + step + 1);
            }
            inner_is_point = false;
        } else {
            for step in 0..STEPS as u32 {
                let a = inner + step;
                let b = first + step;
                mesh.add_triangle(a, b, b + 1);
                mesh.add_triangle(a, b + 1, a + 1);
            }
        }
        inner = first;
    }
    painter.add(mesh);
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
    fn scope_blue_mixes_halfway_on_dark_themes() {
        let theme = Color32::from_rgb(0x1a, 0x14, 0x10);
        let blended = blend_scope(theme, false);
        let mid = |a: u8, b: u8| ((a as f32 + b as f32) * 0.5).round() as u8;
        assert_eq!(blended.r(), mid(theme.r(), scope::BG.r()));
        assert_eq!(blended.g(), mid(theme.g(), scope::BG.g()));
        assert_eq!(blended.b(), mid(theme.b(), scope::BG.b()));
    }

    #[test]
    fn light_theme_wells_stay_near_the_theme_background() {
        let frost_bg = AppTheme::Frost.palette().bg;
        let well = blend_scope(frost_bg, true);
        // Clearly lighter than the old 50% scope mix.
        let heavy = mix(frost_bg, scope::BG, 0.5);
        let luma = |c: Color32| c.r() as u16 + c.g() as u16 + c.b() as u16;
        assert!(luma(well) > luma(heavy));
        assert!(luma(well) > luma(frost_bg) * 8 / 10);
        assert_ne!(well, frost_bg);
        assert_ne!(well, scope::BG);
    }

    #[test]
    fn light_theme_grid_is_quieter_than_dark() {
        let light = scope::grid(true);
        let dark = scope::grid(false);
        assert!(light.a() < dark.a());
        assert!(AppTheme::Frost.palette().is_light);
        assert!(AppTheme::Daylight.palette().is_light);
        assert!(!AppTheme::Copper.palette().is_light);
    }

    #[test]
    fn corner_glow_stays_under_half_the_window() {
        for screen in [
            egui::vec2(1280.0, 800.0),
            egui::vec2(1920.0, 1080.0),
            egui::vec2(800.0, 600.0),
        ] {
            let radius = corner_glow_radius(screen);
            assert!(radius < screen.x * 0.5);
            assert!(radius < screen.y * 0.5);
            assert!(radius > screen.x.min(screen.y) * 0.4);
        }
    }
}

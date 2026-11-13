//! Agile Automation logo: integer-scale paint (never fractional).
//!
//! Fits the UI slot by CPU box-downscaling (or integer upscaling) so one
//! texture pixel maps to one screen pixel, then samples with nearest-neighbor.

use egui::{ColorImage, TextureHandle, TextureOptions, Vec2};
use image::RgbaImage;
use std::collections::HashMap;

const LOGO_BYTES: &[u8] = include_bytes!("../assets/agile-automation-logo.png");

/// Cached logo textures keyed by slot size in physical pixels.
#[derive(Default)]
pub struct AaLogoCache {
    entries: HashMap<(u32, u32), (TextureHandle, Vec2)>,
}

impl AaLogoCache {
    /// Paint the AA mark inside `max_w`×`max_h` points at an integer scale.
    pub fn show(&mut self, ui: &mut egui::Ui, max_w: f32, max_h: f32) -> egui::Response {
        let ppp = ui.ctx().pixels_per_point().max(0.25);
        let max_w_px = (max_w * ppp).floor().max(1.0) as u32;
        let max_h_px = (max_h * ppp).floor().max(1.0) as u32;
        let key = (max_w_px, max_h_px);
        if !self.entries.contains_key(&key) {
            if let Some(entry) = build_logo(ui.ctx(), max_w_px, max_h_px, ppp) {
                self.entries.insert(key, entry);
            }
        }
        if let Some((tex, size)) = self.entries.get(&key) {
            ui.add(
                egui::Image::new(tex)
                    .fit_to_exact_size(*size)
                    .texture_options(nearest_opts()),
            )
        } else {
            ui.allocate_response(Vec2::ZERO, egui::Sense::hover())
        }
    }
}

fn nearest_opts() -> TextureOptions {
    TextureOptions {
        magnification: egui::TextureFilter::Nearest,
        minification: egui::TextureFilter::Nearest,
        ..TextureOptions::LINEAR
    }
}

fn build_logo(
    ctx: &egui::Context,
    max_w_px: u32,
    max_h_px: u32,
    ppp: f32,
) -> Option<(TextureHandle, Vec2)> {
    let raw = image::load_from_memory(LOGO_BYTES).ok()?.into_rgba8();
    let (nw, nh) = raw.dimensions();
    if nw == 0 || nh == 0 {
        return None;
    }

    let (tex_img, scale_up) = if nw <= max_w_px && nh <= max_h_px {
        // Largest integer upscale (1×, 2×, …) that still fits the slot.
        let ux = (max_w_px / nw).max(1);
        let uy = (max_h_px / nh).max(1);
        let up = ux.min(uy).max(1);
        (raw, up)
    } else {
        // Smallest integer downscale factor so both axes fit.
        let fx = ((nw + max_w_px - 1) / max_w_px).max(1);
        let fy = ((nh + max_h_px - 1) / max_h_px).max(1);
        let factor = fx.max(fy);
        let cw = (nw / factor) * factor;
        let ch = (nh / factor) * factor;
        let ox = (nw - cw) / 2;
        let oy = (nh - ch) / 2;
        let cropped = image::imageops::crop_imm(&raw, ox, oy, cw, ch).to_image();
        (integer_box_downscale(&cropped, factor), 1)
    };

    let (tw, th) = tex_img.dimensions();
    let color = ColorImage::from_rgba_unmultiplied([tw as usize, th as usize], tex_img.as_raw());
    let name = format!("aa-logo-{max_w_px}x{max_h_px}");
    let handle = ctx.load_texture(name, color, nearest_opts());
    // Paint size in points: integer tex pixels → integer physical pixels.
    let size = Vec2::new(
        (tw as f32 * scale_up as f32) / ppp,
        (th as f32 * scale_up as f32) / ppp,
    );
    Some((handle, size))
}

/// Average each `factor`×`factor` block into one pixel (integer box filter).
fn integer_box_downscale(src: &RgbaImage, factor: u32) -> RgbaImage {
    if factor <= 1 {
        return src.clone();
    }
    let w = src.width() / factor;
    let h = src.height() / factor;
    let mut out = RgbaImage::new(w, h);
    let area = (factor * factor) as u32;
    for y in 0..h {
        for x in 0..w {
            let mut r = 0u32;
            let mut g = 0u32;
            let mut b = 0u32;
            let mut a = 0u32;
            for dy in 0..factor {
                for dx in 0..factor {
                    let p = src.get_pixel(x * factor + dx, y * factor + dy).0;
                    r += u32::from(p[0]);
                    g += u32::from(p[1]);
                    b += u32::from(p[2]);
                    a += u32::from(p[3]);
                }
            }
            out.put_pixel(
                x,
                y,
                image::Rgba([
                    (r / area) as u8,
                    (g / area) as u8,
                    (b / area) as u8,
                    (a / area) as u8,
                ]),
            );
        }
    }
    out
}

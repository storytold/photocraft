//! Pen tool native cursor bitmaps (#9122).
//!
//! Standard Pen tool cursors must move with the hardware pointer without waiting for UI rendering
//! and presentation latency. This module rasterizes the standard Pen icons (Pen, Freeform Pen,
//! Curvature Pen, Add Anchor Point, Delete Anchor Point, and Convert Point) using continuous
//! subpixel resampling and hotspot quantization compensation to preserve exact fractional-scale
//! geometry matching the painted fallback.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use egui::{CustomCursorImage, Vec2, vec2};

use crate::state::Tool;

static RASTERIZATION_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Get the number of successful Pen cursor rasterizations performed.
pub fn pen_rasterization_count() -> usize {
    RASTERIZATION_COUNT.load(Ordering::Relaxed)
}

/// Reset the Pen cursor rasterization counter to zero.
pub fn reset_pen_rasterization_count() {
    RASTERIZATION_COUNT.store(0, Ordering::Relaxed);
}

/// The six Pen tool cursor variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PenVariant {
    Standard,
    Freeform,
    Curvature,
    AddAnchor,
    DeleteAnchor,
    ConvertPoint,
}

impl PenVariant {
    /// The Pen variant corresponding to `tool`, or `None` if `tool` is not a Pen tool.
    pub fn from_tool(tool: Tool) -> Option<Self> {
        match tool {
            Tool::Pen => Some(Self::Standard),
            Tool::FreeformPen => Some(Self::Freeform),
            Tool::CurvaturePen => Some(Self::Curvature),
            Tool::AddAnchorPoint => Some(Self::AddAnchor),
            Tool::DeleteAnchorPoint => Some(Self::DeleteAnchor),
            Tool::ConvertPoint => Some(Self::ConvertPoint),
            _ => None,
        }
    }

    /// The embedded icon name corresponding to this variant.
    pub fn icon_name(self) -> &'static str {
        match self {
            Self::Standard => "pen-tool",
            Self::Freeform => "pen-freeform",
            Self::Curvature => "pen-curvature",
            Self::AddAnchor => "pen-add-anchor",
            Self::DeleteAnchor => "pen-delete-anchor",
            Self::ConvertPoint => "convert-point",
        }
    }

    /// Where each icon's point is in its 24-unit box: the full-size pen's nib at (2.3, 2.3); the
    /// Freeform and Curvature pens draw it at 0.68 × from (7, 7); Convert Point's apex anchor at (12, 7).
    pub fn hotspot_in_box(self) -> Vec2 {
        match self {
            Self::Freeform | Self::Curvature => vec2(8.56, 8.56),
            Self::ConvertPoint => vec2(12.0, 7.0),
            _ => vec2(2.3, 2.3),
        }
    }

    /// All six variants for iteration and testing.
    pub const fn all() -> [Self; 6] {
        [Self::Standard, Self::Freeform, Self::Curvature, Self::AddAnchor, Self::DeleteAnchor, Self::ConvertPoint]
    }
}

/// The logical size of a standard tool icon cursor in egui points, matching [`crate::icons::cursor`].
const LOGICAL_SIZE: f32 = 22.0;

/// The dark outline's tint alpha in `icons::cursor` (`Color32::from_black_alpha(200)`).
const OUTLINE_TINT_ALPHA: f32 = 200.0 / 255.0;

/// The eight cardinal and diagonal offsets in logical points (`icons::cursor`).
const OUTLINE_OFFSETS: [(f32, f32); 8] = [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0)];

fn sample_alpha(pixels: &[egui::Color32], w: usize, h: usize, x: f32, y: f32) -> f32 {
    if x <= -1.0 || y <= -1.0 || x >= w as f32 || y >= h as f32 {
        return 0.0;
    }
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let x1 = x0 + 1;
    let y1 = y0 + 1;
    let fx = x - x.floor();
    let fy = y - y.floor();

    let get = |ix: i32, iy: i32| -> f32 {
        if ix >= 0 && (ix as usize) < w && iy >= 0 && (iy as usize) < h {
            pixels.get((iy as usize) * w + (ix as usize)).map_or(0.0, |p| f32::from(p.a()) / 255.0)
        } else {
            0.0
        }
    };

    let p00 = get(x0, y0);
    let p10 = get(x1, y0);
    let p01 = get(x0, y1);
    let p11 = get(x1, y1);

    let top = p00 * (1.0 - fx) + p10 * fx;
    let bottom = p01 * (1.0 - fx) + p11 * fx;
    top * (1.0 - fy) + bottom * fy
}

/// Rasterize a native cursor bitmap for `variant` at `scale` bitmap pixels per point.
pub fn rasterize(variant: PenVariant, scale: f32) -> Option<CustomCursorImage> {
    if !scale.is_finite() || scale <= 0.0 || scale > 16.0 {
        return None;
    }
    let icon_px_f = (LOGICAL_SIZE * scale).round();
    if icon_px_f < 1.0 || icon_px_f > 512.0 {
        return None;
    }
    let icon_px = icon_px_f as u32;

    let icon_image = crate::icons::rasterize_svg(variant.icon_name(), icon_px, icon_px)?;
    let [icon_w, icon_h] = icon_image.size;
    if icon_w == 0 || icon_h == 0 {
        return None;
    }

    let offset_px = 1.0 * scale;
    let pad = (offset_px + 2.0).ceil() as usize;
    let width = icon_w.checked_add(pad.checked_mul(2)?)?;
    let height = icon_h.checked_add(pad.checked_mul(2)?)?;
    if width > usize::from(super::MAX_SIDE) || height > usize::from(super::MAX_SIDE) {
        return None;
    }

    // Hotspot computation with OS integer quantization compensation
    let hot_box = variant.hotspot_in_box();
    let hot_pt_x = (hot_box.x / 24.0) * LOGICAL_SIZE;
    let hot_pt_y = (hot_box.y / 24.0) * LOGICAL_SIZE;
    let hot_px_x = hot_pt_x * scale;
    let hot_px_y = hot_pt_y * scale;
    let ideal_hx = pad as f32 + hot_px_x;
    let ideal_hy = pad as f32 + hot_px_y;
    let hx = ideal_hx.round() as u16;
    let hy = ideal_hy.round() as u16;

    // Subpixel shift to place the continuous nib exactly at the integer OS hotspot pixel
    let quant_dx = (hx as f32) - ideal_hx;
    let quant_dy = (hy as f32) - ideal_hy;
    let origin_x = (pad as f32) + quant_dx;
    let origin_y = (pad as f32) + quant_dy;

    let total_pixels = width.checked_mul(height)?;
    let byte_count = total_pixels.checked_mul(4)?;
    let mut rgba = vec![0u8; byte_count];

    for y in 0..height {
        let sy = (y as f32) - origin_y;
        let row_offset = y.checked_mul(width)?;
        for x in 0..width {
            let sx = (x as f32) - origin_x;

            // 1. Accumulate 8-offset dark outline using continuous subpixel resampling
            let mut a_dark = 0.0f32;
            for (odx, ody) in OUTLINE_OFFSETS {
                let sample_x = sx - odx * offset_px;
                let sample_y = sy - ody * offset_px;
                let a_svg = sample_alpha(&icon_image.pixels, icon_w, icon_h, sample_x, sample_y);
                if a_svg > 0.0 {
                    let a_layer = a_svg * OUTLINE_TINT_ALPHA;
                    a_dark = a_layer + a_dark * (1.0 - a_layer);
                }
            }

            // 2. Sample white foreground
            let a_fg = sample_alpha(&icon_image.pixels, icon_w, icon_h, sx, sy);

            // 3. Composite white foreground over dark outline into straight RGBA
            let a_out = a_fg + a_dark * (1.0 - a_fg);
            if a_out > 0.0 {
                let white = ((a_fg / a_out) * 255.0).round().clamp(0.0, 255.0) as u8;
                let alpha = (a_out * 255.0).round().clamp(0.0, 255.0) as u8;
                let dst_idx = row_offset.checked_add(x)?;
                let px = dst_idx.checked_mul(4)?;
                if let Some(dest) = rgba.get_mut(px..px.checked_add(4)?) {
                    dest[0] = white;
                    dest[1] = white;
                    dest[2] = white;
                    dest[3] = alpha;
                }
            }
        }
    }

    let hotspot = [hx.min(width.saturating_sub(1) as u16), hy.min(height.saturating_sub(1) as u16)];
    RASTERIZATION_COUNT.fetch_add(1, Ordering::Relaxed);

    Some(CustomCursorImage { rgba: Arc::from(rgba), size: [width as u16, height as u16], hotspot })
}

//! Canvas cursors must move with the OS pointer, independently of the canvas frame rate.
//! Windows' stock crosshair inverts grey into grey, so use our black/white bitmap there too.

use std::sync::Arc;

use egui::{CursorIcon, CustomCursorImage, Painter, Pos2, Stroke, vec2};

mod hand;
mod symbols;

/// winit's cursor limit. Keep allocations bounded even for hostile brush sizes or zooms.
const MAX_SIDE: u16 = 2048;

/// What a bitmap cursor shows. `Hand` is the Hand tool's open hand, or its fist while it drags.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    Circle { radius: f32, centre: bool },
    Crosshair { length: f32, gap: f32 },
    Hand { closed: bool },
    Zoom { out: bool },
    Pipette,
}

#[derive(Clone)]
struct Cached {
    shape: Shape,
    scale: f32,
    image: CustomCursorImage,
}

/// The image is sticky in egui's platform output. Clear it every pass, including when a
/// document closes or the pointer leaves the canvas; a later widget's cursor takes precedence.
pub(crate) struct CursorLifecycle;

impl egui::plugin::Plugin for CursorLifecycle {
    fn debug_name(&self) -> &'static str {
        "PhotoCraft tool cursor"
    }

    fn on_begin_pass(&mut self, ui: &mut egui::Ui) {
        ui.ctx().set_cursor_image(None);
    }

    fn on_end_pass(&mut self, ui: &mut egui::Ui) {
        ui.ctx().output_mut(|o| {
            if o.cursor_image.is_some() {
                if o.cursor_icon == CursorIcon::None {
                    // None marks our request until all widgets have had their say. A backend
                    // that cannot upload the bitmap must still leave a visible pointer.
                    o.cursor_icon = CursorIcon::Crosshair;
                } else {
                    o.cursor_image = None;
                }
            }
        });
    }
}

pub(crate) fn circle(painter: &Painter, at: Pos2, radius: f32, centre: bool) -> CursorIcon {
    show(painter, at, Shape::Circle { radius, centre })
}

pub(crate) fn crosshair(painter: &Painter, at: Pos2, length: f32, gap: f32) -> CursorIcon {
    show(painter, at, Shape::Crosshair { length, gap })
}

/// Windows has no stock zoom cursors. Other platforms retain their native magnifiers.
pub(crate) fn zoom(painter: &Painter, at: Pos2, out: bool) -> CursorIcon {
    if cfg!(target_os = "windows") {
        show(painter, at, Shape::Zoom { out })
    } else if out {
        CursorIcon::ZoomOut
    } else {
        CursorIcon::ZoomIn
    }
}

/// The tip stays on the sampled pixel. Native root viewports upload the bitmap; web and
/// immediate viewports draw the same geometry without an interactive overlay widget.
pub(crate) fn pipette(ctx: &egui::Context, at: Pos2) -> CursorIcon {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("pc-tool-pointer")));
    show(&painter, at, Shape::Pipette)
}

/// The Hand tool's pointer: an open hand, a fist while it drags (`closed`). winit maps `Grab` and
/// `Grabbing` to the four-arrow move cursor on Windows, so there it is a bitmap; macOS and the
/// Linux themes already draw hands, and the OS moves those without waiting for a frame.
pub(crate) fn hand(ctx: &egui::Context, closed: bool) -> CursorIcon {
    hand_with(ctx, closed, cfg!(target_os = "windows") && ctx.viewport_id() == egui::ViewportId::ROOT)
}

fn hand_with(ctx: &egui::Context, closed: bool, bitmap: bool) -> CursorIcon {
    if bitmap && let Some(image) = cached_image(ctx, Shape::Hand { closed }, ctx.pixels_per_point()) {
        ctx.set_cursor_image(Some(image));
        return CursorIcon::None;
    }
    if closed { CursorIcon::Grabbing } else { CursorIcon::Grab }
}

fn show(painter: &Painter, at: Pos2, shape: Shape) -> CursorIcon {
    let ctx = painter.ctx();
    // eframe's immediate viewports and web integration do not upload cursor images. Keep their
    // existing painter path. Other native platforms retain their existing cursor behaviour.
    let native = cfg!(target_os = "windows") || (!cfg!(target_arch = "wasm32") && matches!(shape, Shape::Pipette));
    if native && ctx.viewport_id() == egui::ViewportId::ROOT {
        let scale = ctx.pixels_per_point();
        if let Some(image) = cached_image(ctx, shape, scale) {
            ctx.set_cursor_image(Some(image));
            return CursorIcon::None;
        }
        // An enormous tip cannot fit an OS cursor. Preserve its full-size outline, with an OS
        // crosshair at the hotspot so the user still gets immediate position feedback.
        paint(painter, at, shape);
        if let Some(image) = cached_image(ctx, Shape::Crosshair { length: 6.0, gap: 2.0 }, scale) {
            ctx.set_cursor_image(Some(image));
        }
        return CursorIcon::None;
    }
    paint(painter, at, shape);
    CursorIcon::None
}

fn cached_image(ctx: &egui::Context, shape: Shape, scale: f32) -> Option<CustomCursorImage> {
    let id = egui::Id::new("photocraft-tool-cursor");
    if let Some(cached) = ctx.data(|d| d.get_temp::<Cached>(id))
        && cached.shape == shape
        && cached.scale == scale
    {
        return Some(cached.image);
    }
    let image = rasterize(shape, scale)?;
    ctx.data_mut(|d| d.insert_temp(id, Cached { shape, scale, image: image.clone() }));
    Some(image)
}

fn paint(painter: &Painter, at: Pos2, shape: Shape) {
    if extent(shape).is_none() {
        return;
    }
    let [dark, light] = crate::theme::Tokens::cursor_outline();
    for (width, color) in [(3.0, dark), (1.0, light)] {
        let stroke = Stroke::new(width, color);
        match shape {
            Shape::Circle { radius, centre } => {
                painter.circle_stroke(at, radius, stroke);
                if centre {
                    paint_crosshair(painter, at, 3.0, 0.0, stroke);
                }
            }
            Shape::Crosshair { length, gap } => paint_crosshair(painter, at, length, gap, stroke),
            // Only ever a bitmap: without one the OS draws its own grab cursors.
            Shape::Hand { .. } => {}
            Shape::Zoom { out } => symbols::paint_zoom(painter, at, out, stroke),
            Shape::Pipette => symbols::paint_pipette(painter, at, stroke),
        }
    }
}

fn paint_crosshair(painter: &Painter, at: Pos2, length: f32, gap: f32, stroke: Stroke) {
    for d in [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0)] {
        painter.line_segment([at + d * gap, at + d * length], stroke);
    }
}

/// Distance to the nearest arm (round caps), in physical pixels from the hotspot.
fn cross_distance(x: f32, y: f32, length: f32, gap: f32) -> f32 {
    let horizontal = (x - x.clamp(gap, length)).hypot(y);
    let vertical = x.hypot(y - y.clamp(gap, length));
    horizontal.min(vertical)
}

fn extent(shape: Shape) -> Option<f32> {
    Some(match shape {
        Shape::Circle { radius, centre } => {
            if !radius.is_finite() || radius <= 0.0 {
                return None;
            }
            if centre { radius.max(3.0) } else { radius }
        }
        Shape::Crosshair { length, gap } => {
            if !length.is_finite() || !gap.is_finite() || gap < 0.0 || length < gap {
                return None;
            }
            length
        }
        Shape::Hand { .. } => hand::HALF,
        Shape::Zoom { .. } => 14.0,
        Shape::Pipette => 23.0,
    })
}

/// How much of the pixel at `(dx, dy)` physical pixels from the hotspot the light fill covers and
/// how much the dark outline around it does, each 0 to 1. A dark crease inside the hand takes
/// from the fill, so the fill never covers it.
fn coverage(shape: Shape, dx: f32, dy: f32, scale: f32) -> (f32, f32) {
    let (x, y) = (dx.abs(), dy.abs());
    match shape {
        Shape::Circle { radius, centre } => {
            let ring = (x.hypot(y) - radius * scale).abs();
            let distance = if centre { ring.min(cross_distance(x, y, 3.0 * scale, 0.0)) } else { ring };
            stroke_coverage(distance, scale)
        }
        Shape::Crosshair { length, gap } => stroke_coverage(cross_distance(x, y, length * scale, gap * scale), scale),
        Shape::Hand { closed } => {
            let (lx, ly) = (dx / scale, dy / scale);
            let edge = hand::shape(closed, lx, ly) * scale;
            let inside = (0.5 - edge).clamp(0.0, 1.0);
            let outline = (scale + 0.5 - edge).clamp(0.0, 1.0);
            let crease = (0.45 * scale + 0.5 - hand::crease(closed, lx, ly) * scale).clamp(0.0, 1.0);
            (inside * (1.0 - crease), outline.max(inside * crease))
        }
        Shape::Zoom { out } => stroke_coverage(symbols::zoom_distance(dx / scale, dy / scale, out) * scale, scale),
        Shape::Pipette => stroke_coverage(symbols::pipette_distance(dx / scale, dy / scale) * scale, scale),
    }
}

/// A 1 px light line with a 1 px dark one on each side, `distance` physical pixels from its centre.
fn stroke_coverage(distance: f32, scale: f32) -> (f32, f32) {
    ((0.5 * scale + 0.5 - distance).clamp(0.0, 1.0), (1.5 * scale + 0.5 - distance).clamp(0.0, 1.0))
}

fn rasterize(shape: Shape, scale: f32) -> Option<CustomCursorImage> {
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let extent = extent(shape)?;
    // Odd dimensions put the hotspot on the centre pixel at every DPI. Include the outline
    // and its antialiasing fringe, then check before converting or allocating.
    let half = ((extent + 1.5) * scale + 1.0).ceil();
    if !half.is_finite() || half > f32::from((MAX_SIDE - 1) / 2) {
        return None;
    }
    let hot = half as u16;
    let side = hot * 2 + 1;
    let bytes = usize::from(side).checked_mul(usize::from(side))?.checked_mul(4)?;
    let mut rgba = Vec::with_capacity(bytes);
    let [dark, light] = crate::theme::Tokens::cursor_outline();
    for row in 0..side {
        let dy = f32::from(row) - f32::from(hot);
        for col in 0..side {
            let dx = f32::from(col) - f32::from(hot);
            let (fill, outline) = coverage(shape, dx, dy, scale);
            let a_dark = outline * f32::from(dark.a()) / 255.0;
            let a_light = fill * f32::from(light.a()) / 255.0;
            let alpha = a_light + a_dark * (1.0 - a_light);
            // Straight RGBA, as required by winit, not egui's premultiplied representation.
            let white = if alpha > 0.0 { (255.0 * a_light / alpha).round() as u8 } else { 0 };
            rgba.extend_from_slice(&[white, white, white, (255.0 * alpha).round() as u8]);
        }
    }
    Some(CustomCursorImage { rgba: Arc::from(rgba), size: [side, side], hotspot: [hot, hot] })
}

#[cfg(test)]
mod tests;

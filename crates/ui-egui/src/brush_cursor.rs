//! Cached OS brush outline. Large tips and web keep the canvas-drawn footprint.

use egui::{Context, CustomCursorImage};

#[derive(Default)]
pub struct BrushCursor {
    cached: Option<((u32, u32, bool), CustomCursorImage)>,
}

impl BrushCursor {
    pub fn show(&mut self, ctx: &Context, radius: f32, crosshair: bool) -> bool {
        if cfg!(target_arch = "wasm32") {
            return false;
        }
        // winit 0.30 builds macOS NSImages with size == bitmap dimensions in logical points.
        // Other native backends consume physical cursor pixels. Scaling twice breaks Retina tips.
        let scale = if cfg!(target_os = "macos") { ctx.zoom_factor() } else { ctx.pixels_per_point() };
        let key = (radius.to_bits(), scale.to_bits(), crosshair);
        if self.cached.as_ref().is_none_or(|(k, _)| *k != key) {
            self.cached = image(radius, scale, crosshair).map(|image| (key, image));
        }
        if let Some((_, image)) = &self.cached {
            ctx.set_cursor_image(Some(image.clone()));
            true
        } else {
            false
        }
    }
}

fn image(radius: f32, scale: f32, crosshair: bool) -> Option<CustomCursorImage> {
    if !radius.is_finite() || radius <= 0.0 || !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let r = radius * scale;
    let half = ((r + 2.0 * scale).max(5.0 * scale)).ceil();
    // Stay within a modest OS bitmap size; never allocate based on an unbounded brush size.
    if half > 127.0 {
        return None;
    }
    let side = 2 * half as u16 + 1;
    let mut rgba = Vec::with_capacity(usize::from(side).pow(2) * 4);
    let coverage = |distance: f32, width: f32| (width / 2.0 + 0.5 - distance).clamp(0.0, 1.0);
    for y in 0..side {
        for x in 0..side {
            let (dx, dy) = (f32::from(x) - half, f32::from(y) - half);
            let distance = dx.hypot(dy);
            let mut black = coverage((distance - r - 0.5 * scale).abs(), scale) * 140.0 / 255.0;
            let mut white = coverage((distance - r).abs(), scale) * 220.0 / 255.0;
            if crosshair {
                let cross = dx.abs().min(dy.abs());
                let ends = (3.0 * scale + 0.5 - dx.abs().max(dy.abs())).clamp(0.0, 1.0);
                black = black.max(coverage(cross, 2.5 * scale) * ends * 140.0 / 255.0);
                white = white.max(coverage(cross, scale) * ends * 220.0 / 255.0);
            }
            let alpha = white + black * (1.0 - white);
            let value = if alpha > 0.0 { (255.0 * white / alpha).round() as u8 } else { 0 };
            rgba.extend_from_slice(&[value, value, value, (255.0 * alpha).round() as u8]);
        }
    }
    Some(CustomCursorImage { rgba: rgba.into(), size: [side; 2], hotspot: [half as u16; 2] })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_hotspot_and_bounded_fallback() {
        for scale in [1.0, 1.5, 2.0] {
            let image = image(20.0, scale, true).unwrap();
            assert_eq!(image.rgba.len(), usize::from(image.size[0]).pow(2) * 4);
            assert_eq!(image.hotspot[0] * 2 + 1, image.size[0]);
            assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
        }
        for radius in [f32::NAN, f32::INFINITY, -1.0, 0.0, 10000.0] {
            assert!(image(radius, 2.0, false).is_none());
        }
        assert!(image(20.0, f32::NAN, false).is_none());
    }

    #[test]
    fn reuses_bitmap_until_shape_changes() {
        let ctx = Context::default();
        let mut cache = BrushCursor::default();
        assert!(cache.show(&ctx, 10.0, false));
        let first = cache.cached.as_ref().unwrap().1.rgba.clone();
        assert!(cache.show(&ctx, 10.0, false));
        assert!(std::sync::Arc::ptr_eq(&first, &cache.cached.as_ref().unwrap().1.rgba));
        assert!(cache.show(&ctx, 10.0, true));
        assert!(!std::sync::Arc::ptr_eq(&first, &cache.cached.as_ref().unwrap().1.rgba));
    }

    #[test]
    fn canvas_preferences_tools_and_leaving_canvas() {
        use crate::{PhotocraftApp, Services, Tool};
        use egui_kittest::Harness;
        use serde_json::json;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        app.run("file.new", json!({"width": 128, "height": 128})).unwrap();
        app.ui.tool = Tool::Brush;
        app.session.tools.brush.size = 4.0;
        let mut h = Harness::builder().with_size(egui::vec2(1000.0, 800.0)).build_eframe(|_| app);
        h.run_steps(3);
        for dpi in [1.0, 2.0] {
            h.set_pixels_per_point(dpi);
            h.run_steps(3);
            for preference in ["normalTip", "fullSizeTip", "precise", "standard"] {
                h.state_mut().run("prefs.set", json!({"path": "cursors.painting", "value": preference})).unwrap();
                let pos = h.state().last_canvas_rect.center();
                h.event(egui::Event::PointerMoved(pos));
                h.run_steps(2);
                let out = &h.output().platform_output;
                assert_eq!(out.cursor_image.is_some(), matches!(preference, "normalTip" | "fullSizeTip"), "{preference}, {dpi}");
                if cfg!(target_os = "macos")
                    && let Some(image) = &out.cursor_image
                {
                    let brush = &h.state().session.tools.brush;
                    let full = brush.size / 2.0 * h.state().current_zoom();
                    let radius = if preference == "normalTip" { (full * (0.5 + 0.5 * brush.hardness)).max(1.0) } else { full.max(1.0) };
                    let scale = h.ctx.zoom_factor();
                    assert_eq!(image.hotspot[0], (((radius + 2.0).max(5.0)) * scale).ceil() as u16, "NSImage uses logical points even on Retina");
                }
                assert_ne!(out.cursor_icon, egui::CursorIcon::None);
            }
        }
        h.state_mut().run("prefs.set", json!({"path": "cursors.painting", "value": "normalTip"})).unwrap();
        for tool in [Tool::Eraser, Tool::BackgroundEraser, Tool::QuickSelection] {
            h.state_mut().ui.tool = tool;
            h.run_steps(2);
            assert!(h.output().platform_output.cursor_image.is_some(), "{tool:?}");
        }
        h.state_mut().ui.tool = Tool::Pencil;
        h.run_steps(2);
        assert!(h.output().platform_output.cursor_image.is_none(), "pixel-snapped pencil retains its footprint");
        h.state_mut().ui.tool = Tool::Brush;
        h.state_mut().session.tools.brush.size = 2000.0;
        h.run_steps(2);
        assert!(h.output().platform_output.cursor_image.is_none(), "large outlines stay on canvas");
        h.event(egui::Event::PointerMoved(egui::pos2(10.0, 10.0)));
        h.run_steps(2);
        assert!(h.output().platform_output.cursor_image.is_none());
    }
}

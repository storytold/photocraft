//! Small brush outlines and pipettes belong to the OS cursor plane: they follow the mouse even while the
//! canvas is busy. Large outlines and the pixel-snapped Pencil still use the canvas overlay.

use egui::{Context, CustomCursorImage};

// Stay within conservative desktop cursor dimensions. This also bounds every allocation.
const MAX_SIDE: u16 = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    radius: u16, // bitmap pixels, quantized to 1/8 pixel
    scale: u16,  // bitmap pixels per egui point, quantized to 1/8
    crosshair: bool,
}

#[derive(Clone)]
struct Cached {
    key: Key,
    image: CustomCursorImage,
}

fn key(radius: f32, scale: f32, crosshair: bool) -> Option<Key> {
    if !radius.is_finite() || !scale.is_finite() || radius <= 0.0 || !(0.25..=8.0).contains(&scale) {
        return None;
    }
    let extent = radius.max(if crosshair { 3.0 } else { 0.0 }) * scale + 2.0 * scale + 1.0;
    if extent.ceil() * 2.0 + 1.0 > f32::from(MAX_SIDE) {
        return None;
    }
    Some(Key { radius: (radius * scale * 8.0).round() as u16, scale: (scale * 8.0).round() as u16, crosshair })
}

fn bitmap(key: Key) -> CustomCursorImage {
    let radius = f32::from(key.radius) / 8.0;
    let scale = f32::from(key.scale) / 8.0;
    let extent = (radius.max(if key.crosshair { 3.0 * scale } else { 0.0 }) + 2.0 * scale + 1.0).ceil();
    let half = (extent as u16).min((MAX_SIDE - 1) / 2);
    let side = half * 2 + 1;
    let mut rgba = Vec::with_capacity(usize::from(side) * usize::from(side) * 4);
    for y in 0..side {
        for x in 0..side {
            let dx = f32::from(x) - f32::from(half);
            let dy = f32::from(y) - f32::from(half);
            let distance = dx.hypot(dy);
            // White contour with a dark outside rim, as in the existing canvas cursor.
            let coverage = |d: f32, width: f32| (width * 0.5 + 0.5 - d.abs()).clamp(0.0, 1.0);
            let mut white = coverage(distance - radius, scale) * (220.0 / 255.0);
            let mut black = coverage(distance - radius - 0.5 * scale, scale) * (140.0 / 255.0);
            if key.crosshair {
                let cross = |width| {
                    let horizontal = coverage(dy, width) * (3.0 * scale + 0.5 - dx.abs()).clamp(0.0, 1.0);
                    let vertical = coverage(dx, width) * (3.0 * scale + 0.5 - dy.abs()).clamp(0.0, 1.0);
                    horizontal.max(vertical)
                };
                white = white.max(cross(scale) * (220.0 / 255.0));
                black = black.max(cross(2.5 * scale) * (140.0 / 255.0));
            }
            let alpha = white + black * (1.0 - white);
            // egui/winit require straight RGBA, not premultiplied colour.
            let gray = if alpha > 0.0 { (255.0 * white / alpha).round() as u8 } else { 0 };
            rgba.extend_from_slice(&[gray, gray, gray, (255.0 * alpha).round() as u8]);
        }
    }
    CustomCursorImage { rgba: rgba.into(), size: [side, side], hotspot: [half, half] }
}

fn native_scale(ctx: &Context) -> f32 {
    // winit gives NSCursor images their bitmap dimensions as logical points. Including the
    // monitor scale here would enlarge macOS cursors a second time on Retina displays.
    // Other desktop backends use physical bitmap pixels. Both paths include egui UI zoom.
    if cfg!(target_os = "macos") { ctx.zoom_factor() } else { ctx.pixels_per_point() }
}

/// Returns false for web integrations and oversized/invalid tips; callers retain their overlay.
pub(crate) fn show(ctx: &Context, radius: f32, crosshair: bool) -> bool {
    if cfg!(target_arch = "wasm32") {
        return false; // eframe's web integration currently ignores bitmap cursors.
    }
    let Some(key) = key(radius, native_scale(ctx), crosshair) else { return false };
    let id = egui::Id::new("photocraft-brush-cursor");
    let image = ctx.data_mut(|data| {
        if let Some(cached) = data.get_temp::<Cached>(id)
            && cached.key == key
        {
            return cached.image;
        }
        let image = bitmap(key);
        data.insert_temp(id, Cached { key, image: image.clone() });
        image
    });
    ctx.set_cursor_image(Some(image));
    true
}

#[derive(Clone)]
struct Pipette {
    side: u32,
    image: Option<CustomCursorImage>,
}

fn pipette_bitmap(side: u32) -> Option<CustomCursorImage> {
    if !(6..=u32::from(MAX_SIDE)).contains(&side) {
        return None;
    }
    // Reuse the attributed Lucide icon, with padding and a dark halo on light images.
    let source = include_str!("../../../assets/icons/pipette.svg");
    let white = source.replace("<svg", "<svg x=\"2\" y=\"2\"").replace("currentColor", "#ffffff").replace("stroke-width=\"2\"", "stroke-width=\"1.75\"");
    let black = source
        .replace("<svg", "<svg x=\"2\" y=\"2\" opacity=\"0.8\"")
        .replace("currentColor", "#000000")
        .replace("stroke-width=\"2\"", "stroke-width=\"3.75\"");
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 28 28">{black}{white}</svg>"#);
    let image = egui_extras::image::load_svg_bytes_with_size(svg.as_bytes(), egui::SizeHint::Width(side), &Default::default()).ok()?;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect();
    // The actual sampling point is the lower-left tip (2,22), plus the two-unit padding.
    let hot = |v: f32| ((v / 28.0 * side as f32).round() as u16).min(side as u16 - 1);
    Some(CustomCursorImage { rgba: rgba.into(), size: [side as u16; 2], hotspot: [hot(4.0), hot(24.0)] })
}

/// Alt sampling and the Eyedropper share a cached native bitmap; web keeps the icon overlay.
pub(crate) fn eyedropper(ctx: &Context, pos: egui::Pos2) -> egui::CursorIcon {
    let scale = native_scale(ctx);
    let side = (28.0 * 20.0 / 24.0 * scale).round();
    if !cfg!(target_arch = "wasm32") && side.is_finite() && (6.0..=f32::from(MAX_SIDE)).contains(&side) {
        let side = side as u32;
        let id = egui::Id::new("photocraft-pipette-cursor");
        let image = ctx.data_mut(|data| {
            if let Some(cached) = data.get_temp::<Pipette>(id)
                && cached.side == side
            {
                return cached.image;
            }
            let image = pipette_bitmap(side);
            data.insert_temp(id, Pipette { side, image: image.clone() });
            image
        });
        if let Some(image) = image {
            ctx.set_cursor_image(Some(image));
            return egui::CursorIcon::Crosshair; // visible fallback for non-winit integrations
        }
    }
    crate::icons::cursor(ctx, "pipette", pos, egui::vec2(2.0, 22.0) / 24.0, 20.0);
    egui::CursorIcon::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_scale_accounts_for_backend_units_and_ui_zoom() {
        let ctx = Context::default();
        ctx.set_zoom_factor(1.25);
        let mut input = egui::RawInput::default();
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(2.0);
        ctx.run_ui(input, |_| {}).textures_delta.clear();
        assert_eq!(ctx.pixels_per_point(), 2.5);
        assert_eq!(native_scale(&ctx), if cfg!(target_os = "macos") { 1.25 } else { 2.5 });
    }

    #[test]
    fn pipette_bitmap_has_a_tip_hotspot_and_is_cached_across_pointer_moves() {
        assert!(pipette_bitmap(0).is_none());
        assert!(pipette_bitmap(u32::MAX).is_none());
        for side in [23, 29, 35, 47, 70] {
            let image = pipette_bitmap(side).unwrap();
            assert_eq!(image.rgba.len(), side as usize * side as usize * 4);
            assert!(image.hotspot[0] < image.size[0] / 4);
            assert!(image.hotspot[1] > image.size[1] * 3 / 4 && image.hotspot[1] < image.size[1]);
            assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[0] > 200 && p[3] > 200));
            assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[0] < 80 && p[3] > 100));
        }
        let ctx = Context::default();
        eyedropper(&ctx, egui::pos2(10.0, 10.0));
        let first = ctx.output(|o| o.cursor_image.clone()).unwrap();
        eyedropper(&ctx, egui::pos2(80.0, 60.0));
        let moved = ctx.output(|o| o.cursor_image.clone()).unwrap();
        assert!(std::sync::Arc::ptr_eq(&first.rgba, &moved.rgba));
        if let Some(dir) = std::env::var_os("PHOTOCRAFT_CURSOR_EVIDENCE") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            let image = pipette_bitmap(70).unwrap();
            let image =
                photocraft_codecs::Image::from_raw(70, 70, photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8, image.rgba.to_vec())
                    .unwrap();
            let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
            std::fs::write(dir.join("pipette-cursor.png"), png).unwrap();
        }
    }

    #[test]
    fn cursor_is_bounded_and_rejects_invalid_or_oversized_input() {
        for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
            for radius in [1.0, 3.0, 8.0, 16.0] {
                let image = bitmap(key(radius, scale, true).unwrap());
                assert!(image.size[0] <= MAX_SIDE);
                assert_eq!(image.rgba.len(), usize::from(image.size[0]) * usize::from(image.size[1]) * 4);
                assert_eq!(image.hotspot, [image.size[0] / 2, image.size[1] / 2]);
                assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
                assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[3] == 0));
            }
        }
        for radius in [f32::NAN, f32::INFINITY, -1.0, 0.0, 10000.0] {
            assert!(key(radius, 1.0, false).is_none());
        }
        for scale in [f32::NAN, f32::INFINITY, -1.0, 0.0, 10000.0] {
            assert!(key(10.0, scale, false).is_none());
        }
    }

    #[test]
    fn outline_scales_with_dpi_and_leaves_the_centre_clear_unless_requested() {
        let plain = bitmap(key(16.0, 1.0, false).unwrap());
        let centre = (usize::from(plain.hotspot[1]) * usize::from(plain.size[0]) + usize::from(plain.hotspot[0])) * 4;
        assert_eq!(plain.rgba[centre + 3], 0);
        let cross = bitmap(key(16.0, 1.0, true).unwrap());
        assert!(cross.rgba[centre + 3] > 200);
        let hidpi = bitmap(key(16.0, 2.0, false).unwrap());
        assert!(hidpi.size[0] > plain.size[0]);
        // Contrast on either a white or black canvas requires both light and dark texels.
        assert!(plain.rgba.as_chunks::<4>().0.iter().any(|p| p[0] > 180 && p[3] > 100));
        assert!(plain.rgba.as_chunks::<4>().0.iter().any(|p| p[0] < 100 && p[3] > 20));
        // Optional visual evidence of the exact bitmap sent to the OS. Window screenshots
        // cannot capture an OS cursor, so inspect this alongside the offscreen canvas.
        if let Some(dir) = std::env::var_os("PHOTOCRAFT_CURSOR_EVIDENCE") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            for (name, background) in [("dark", 32u8), ("light", 255u8)] {
                let mut rgba = hidpi.rgba.to_vec();
                for p in rgba.as_chunks_mut::<4>().0 {
                    let a = f32::from(p[3]) / 255.0;
                    let v = (f32::from(p[0]) * a + f32::from(background) * (1.0 - a)).round() as u8;
                    p.copy_from_slice(&[v, v, v, 255]);
                }
                let image = photocraft_codecs::Image::from_raw(
                    u32::from(hidpi.size[0]),
                    u32::from(hidpi.size[1]),
                    photocraft_codecs::ChannelLayout::Rgba,
                    photocraft_codecs::SampleType::U8,
                    rgba,
                )
                .unwrap();
                let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
                std::fs::write(dir.join(format!("brush-cursor-{name}.png")), png).unwrap();
            }
        }
    }

    #[test]
    fn hover_reuses_the_same_native_bitmap() {
        let ctx = Context::default();
        assert!(show(&ctx, 16.0, true));
        let first = ctx.output(|o| o.cursor_image.clone()).unwrap();
        assert!(show(&ctx, 16.0, true));
        let next = ctx.output(|o| o.cursor_image.clone()).unwrap();
        assert!(std::sync::Arc::ptr_eq(&first.rgba, &next.rgba));
        assert!(show(&ctx, 20.0, true));
        let resized = ctx.output(|o| o.cursor_image.clone()).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&first.rgba, &resized.rgba));
    }
}

//! Small font-menu samples, rendered by the same font database as document text.

use std::collections::VecDeque;
use std::sync::PoisonError;

use egui::{ColorImage, Rect, TextureHandle, TextureOptions, pos2, vec2};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::TextLayer;
use photocraft_geom::Affine;

const SAMPLE: &str = "AaBbCc";
const WIDTH: f32 = 100.0;
const HEIGHT: f32 = 24.0;
const CACHE_LIMIT: usize = 128;

/// Family, display scale, and the font database generations, so a family installed or fetched
/// (served fonts) after its first miss gets a sample once it is available.
type Key = (String, u32, (u64, u64));
#[derive(Clone, Default)]
struct Cache(VecDeque<(Key, Option<TextureHandle>)>);

fn image(engine: &mut photocraft_text::TextEngine, family: &str, scale: u32) -> Option<ColorImage> {
    // Unknown/unreadable families keep their plain menu label, rather than a misleading fallback.
    if !engine.fonts.has_family(family) {
        return None;
    }
    let chars = engine.fonts.charmap(family, 400, false);
    let text = if SAMPLE.chars().all(|c| chars.binary_search(&c).is_ok()) {
        SAMPLE.into()
    } else {
        // Symbol/script-only faces must show their own glyphs, not the Latin fallback font.
        chars.into_iter().filter(|c| !c.is_whitespace()).take(6).collect::<String>()
    };
    if text.is_empty() {
        return None;
    }
    let layer = TextLayer { text, font_family: family.into(), size_pt: 18.0 * scale as f32, ..Default::default() };
    let layout = engine.layout(&layer, 72.0);
    let bounds = layout.bounds()?;
    let width = (WIDTH as u32 * scale) as usize;
    let height = (HEIGHT as u32 * scale) as usize;
    let transform = Affine::translate(f64::from(-bounds[0]), f64::from((height as f32 - (bounds[3] - bounds[1])) / 2.0 - bounds[1]));
    // Reject oversized ink before the rasterizer allocates a surface (malformed installed fonts).
    let ink = photocraft_text::render::ink_rect(&layout, &transform);
    if ink.is_empty() || ink.width() > width as u32 * 4 || ink.height() > height as u32 * 4 {
        return None;
    }
    let rendered = photocraft_text::render::rasterize(&layout, &transform, PixelFormat::new(ColorMode::Rgb, SampleType::U8, true), layer.antialias);
    let mut rgba = vec![[0u8; 4]; width * height];
    rendered.surface.read_rgba8_into(photocraft_geom::Rect::new(0, 0, width as i32, height as i32), &mut rgba);
    Some(ColorImage::from_rgba_unmultiplied([width, height], &rgba.into_iter().flat_map(|p| [255, 255, 255, p[3]]).collect::<Vec<_>>()))
}

pub(crate) fn paint(ui: &egui::Ui, family: &str, row: Rect) {
    let ctx = ui.ctx();
    let scale = ctx.pixels_per_point().ceil().clamp(1.0, 4.0) as u32;
    let key = (family.to_owned(), scale, (photocraft_text::fonts::generation(), photocraft_text::served::generation()));
    let id = egui::Id::new("font-menu-previews");
    let cached = ctx.data_mut(|d| {
        let cache = d.get_temp_mut_or_default::<Cache>(id);
        let index = cache.0.iter().position(|(k, _)| k == &key)?;
        let entry = cache.0.remove(index)?;
        let texture = entry.1.clone();
        cache.0.push_back(entry);
        Some(texture)
    });
    let texture = cached.unwrap_or_else(|| {
        let texture = image(&mut photocraft_text::shared().lock().unwrap_or_else(PoisonError::into_inner), family, scale)
            .map(|img| ctx.load_texture(format!("font-sample-{family}"), img, TextureOptions::LINEAR));
        ctx.data_mut(|d| {
            let cache = d.get_temp_mut_or_default::<Cache>(id);
            if cache.0.len() >= CACHE_LIMIT {
                cache.0.pop_front();
            }
            cache.0.push_back((key, texture.clone()));
        });
        texture
    });
    if let Some(texture) = texture {
        let rect = Rect::from_min_size(pos2(row.right() - WIDTH - 8.0, row.center().y - HEIGHT / 2.0), vec2(WIDTH, HEIGHT));
        ui.painter().image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), crate::theme::Tokens::get(ctx).text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_use_the_requested_face_and_scale() {
        let mut engine = photocraft_text::TextEngine::new();
        let sans = image(&mut engine, "Inter", 1).expect("bundled sans");
        let mono = image(&mut engine, "JetBrains Mono", 1).expect("bundled mono");
        assert_eq!(sans.size, [100, 24]);
        assert!(sans.pixels.iter().any(|p| p.a() > 0));
        assert_ne!(sans.pixels, mono.pixels, "samples use different font outlines");
        assert_eq!(image(&mut engine, "Inter", 2).unwrap().size, [200, 48]);
        assert!(image(&mut engine, "Missing font", 1).is_none());
    }
}

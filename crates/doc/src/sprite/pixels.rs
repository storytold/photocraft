use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

use super::{MAX_BYTES, PaletteEntry, SpriteCel};

pub fn checked_rect(rect: Rect, bpp: usize) -> Result<(), String> {
    let cost = rect.tile_count().checked_mul((256 * 256 * bpp) as u64).ok_or("sprite cel rectangle overflow")?;
    if rect.x1 < rect.x0 || rect.y1 < rect.y0 || rect.width() > 65_535 || rect.height() > 65_535 || cost > MAX_BYTES as u64 {
        return Err("sprite cel rectangle exceeds the decoded tile limit".into());
    }
    Ok(())
}

pub fn render_indices(indices: &Surface, rect: Rect, palette: &[PaletteEntry], transparent: Option<u8>) -> Result<Surface, String> {
    checked_rect(rect, 5)?;
    let mut out = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    if rect.is_empty() {
        return Ok(out);
    }
    let mut row = vec![0; rect.width() as usize * 4];
    for y in rect.y0..rect.y1 {
        let values = indices.to_interleaved(Rect::from_xywh(rect.x0, y, rect.width(), 1));
        for (index, rgba) in values.iter().zip(row.chunks_exact_mut(4)) {
            let mut value = palette.get(usize::from(*index)).ok_or("sprite pixel refers to a missing palette entry")?.rgba;
            if transparent == Some(*index) {
                value[3] = 0;
            }
            rgba.copy_from_slice(&value);
        }
        out.try_write_interleaved(Rect::from_xywh(rect.x0, y, rect.width(), 1), &row).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// Quantize only changed tiles. For an unchanged color, retain the previous index even when
/// another palette entry has the very same RGBA value. Index identity survives ordinary edits.
pub fn capture_indices(cel: &mut SpriteCel, edited: &Surface, palette: &[PaletteEntry], transparent: Option<u8>) -> Result<(), String> {
    let rect = cel.rect.union(&edited.content_bounds());
    checked_rect(rect, 5)?;
    let index_format = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    let mut indices = if rect == cel.rect {
        cel.indices.clone().unwrap_or_else(|| Surface::with_default(index_format, &[f32::from(transparent.unwrap_or(0)) / 255.0]))
    } else {
        let mut expanded = Surface::with_default(index_format, &[f32::from(transparent.unwrap_or(0)) / 255.0]);
        if let Some(original) = &cel.indices {
            for y in cel.rect.y0..cel.rect.y1 {
                let row = Rect::from_xywh(cel.rect.x0, y, cel.rect.width(), 1);
                expanded.try_write_interleaved(row, &original.to_interleaved(row)).map_err(|e| e.to_string())?;
            }
        }
        expanded
    };
    let mut display = cel.pixels.clone();
    let tiles: std::collections::BTreeSet<_> = cel.pixels.tiles().chain(edited.tiles()).map(|(tc, _)| *tc).collect();
    for tc in tiles {
        if cel.pixels.tile(tc) == edited.tile(tc) {
            continue;
        }
        let r = rect.intersect(&tc.rect());
        for y in r.y0..r.y1 {
            let mut row = Vec::with_capacity(r.width() as usize);
            let mut colors = Vec::with_capacity(r.width() as usize * 4);
            for x in r.x0..r.x1 {
                let rgba = rgba8(edited, x, y);
                let old = (indices.pixel(x, y).first().copied().unwrap_or(0.0) * 255.0).round() as u8;
                let matches = |i: u8| palette.get(usize::from(i)).is_some_and(|e| color(e, i, transparent) == rgba);
                let index = if matches(old) {
                    old
                } else if rgba[3] == 0
                    && let Some(index) = transparent
                {
                    index
                } else {
                    palette
                        .iter()
                        .enumerate()
                        .min_by_key(|(i, e)| {
                            let c = color(e, *i as u8, transparent);
                            let premult = |p: [u8; 4]| {
                                [
                                    u32::from(p[0]) * u32::from(p[3]) / 255,
                                    u32::from(p[1]) * u32::from(p[3]) / 255,
                                    u32::from(p[2]) * u32::from(p[3]) / 255,
                                    u32::from(p[3]),
                                ]
                            };
                            premult(c).into_iter().zip(premult(rgba)).map(|(a, b)| a.abs_diff(b).pow(2)).sum::<u32>()
                        })
                        .map(|(i, _)| i as u8)
                        .ok_or("sprite palette is empty")?
                };
                row.push(index);
                colors.extend(color(palette.get(usize::from(index)).ok_or("sprite palette entry is missing")?, index, transparent));
            }
            indices.try_write_interleaved(Rect::from_xywh(r.x0, y, r.width(), 1), &row).map_err(|e| e.to_string())?;
            display.try_write_interleaved(Rect::from_xywh(r.x0, y, r.width(), 1), &colors).map_err(|e| e.to_string())?;
        }
    }
    cel.rect = rect;
    cel.pixels = display;
    cel.indices = Some(indices);
    Ok(())
}

fn color(entry: &PaletteEntry, index: u8, transparent: Option<u8>) -> [u8; 4] {
    let mut value = entry.rgba;
    if transparent == Some(index) {
        value[3] = 0;
    }
    value
}

fn rgba8(surface: &Surface, x: i32, y: i32) -> [u8; 4] {
    let p = surface.pixel(x, y);
    let byte = |i: usize, default| (p.get(i).copied().unwrap_or(default).clamp(0.0, 1.0) * 255.0).round() as u8;
    [byte(0, 0.0), byte(1, 0.0), byte(2, 0.0), byte(3, 1.0)]
}

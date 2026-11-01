//! Recorded Replace Color samples are a Lab box, not a single RGB colour.
use photocraft_algo::{selection, tone};
use photocraft_cms::{Builtin, TransformOptions};
use photocraft_doc::SampleType;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::Value;

use super::{bad, num};
use crate::{EngineError, Result, Session};

const CMD: &str = "image.adjustments.replaceColor";

/// Unstored pixels can still be selected when the mask has a nonzero default.
fn edit_bounds(content: Rect, selected: Option<&Surface>) -> Rect {
    match selected {
        Some(mask) if mask.default_pixel().first() == Some(&0.0) => content.intersect(&mask.content_bounds()),
        _ => content,
    }
}

/// Validate original precision before checking order and converting to native f32.
fn bound(p: &Value, key: &str) -> Result<[f64; 3]> {
    let values = p.get(key).and_then(Value::as_array).filter(|a| a.len() == 3).ok_or_else(|| bad(CMD, format!("`{key}` must be three finite Lab numbers")))?;
    let mut result = [0.0; 3];
    for (out, value) in result.iter_mut().zip(values) {
        let n = value.as_f64().filter(|n| n.is_finite() && (*n as f32).is_finite()).ok_or_else(|| bad(CMD, format!("invalid `{key}` Lab number")))?;
        *out = n;
    }
    Ok(result)
}

/// Apply explicit recorded bounds; `None` leaves the legacy RGB/sample path alone.
/// Matching converts original surface channels through their document profile,
/// before the existing RGBA tone shift can clip or change their colour space.
pub(super) fn apply(s: &mut Session, p: &Value) -> Result<Option<Value>> {
    if p.get("labMinimum").is_none() && p.get("labMaximum").is_none() {
        return Ok(None);
    }
    if ["color", "colors", "points", "subtractPoints", "order", "localized", "range", "channel", "targetChannel"].iter().any(|key| p.get(*key).is_some()) {
        return Err(bad(CMD, "Lab bounds cannot be combined with RGB samples, localized settings or channel targets"));
    }
    if p.get("target").is_some_and(|t| t.as_str() != Some("pixels")) || crate::channel_cmds::edits_channel(s) || crate::color_channels::target(s, p)?.is_some()
    {
        return Err(bad(CMD, "Lab bounds require the composite pixel-layer target"));
    }
    let (lo, hi) = (bound(p, "labMinimum")?, bound(p, "labMaximum")?);
    if lo.iter().zip(hi).any(|(lo, hi)| *lo > hi) {
        return Err(bad(CMD, "Lab minimum must not exceed maximum"));
    }
    let mut range = selection::LabRange { lo: lo.map(|v| v as f32), hi: hi.map(|v| v as f32) };
    // Validate supplied shifts before the edit rather than silently defaulting.
    for key in ["fuzziness", "hue", "saturation", "lightness"] {
        if p.get(key).is_some_and(|v| !v.as_f64().is_some_and(f64::is_finite)) {
            return Err(bad(CMD, format!("`{key}` must be a finite number")));
        }
    }
    let fuzz = num(p, "fuzziness", 40.0).clamp(0.0, 200.0);
    let (h, sat, l) = (num(p, "hue", 0.0).clamp(-180.0, 180.0), num(p, "saturation", 0.0).clamp(-100.0, 100.0), num(p, "lightness", 0.0).clamp(-100.0, 100.0));
    let id = crate::commands::layer_param(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    super::super::enabled::check_pixels_unlocked(&st.doc, id)?;
    let surface = st.doc.layer(id).and_then(|l| l.surface()).ok_or_else(|| bad(CMD, "Lab bounds require a pixel layer"))?;
    let format = surface.format();
    if format.mode != st.doc.mode {
        return Err(bad(CMD, "pixel layout does not match the document profile"));
    }
    let transform = photocraft_cms::cached(&crate::color_cmds::document_profile(&st.doc), Builtin::LabD50.profile(), TransformOptions::default())
        .map_err(|e| bad(CMD, format!("Lab colour transform: {e}")))?;
    if transform.inputs() != format.mode.color_channels() || transform.outputs() != 3 || format.channels() != transform.inputs() + usize::from(format.alpha) {
        return Err(bad(CMD, "unsupported Lab colour-transform layout"));
    }
    let grid = format.sample == SampleType::U8;
    if grid {
        range.lo = selection::quantize_lab8(range.lo);
        range.hi = selection::quantize_lab8(range.hi);
    }
    if h == 0.0 && sat == 0.0 && l == 0.0 {
        // Validation still runs, but a zero shift must not re-separate native inks.
        return Ok(Some(Value::Null));
    }
    s.edit("Replace Color", |doc, _| {
        let canvas = doc.bounds();
        let selected = doc.selection.clone();
        let surface = crate::commands::paint_surface(doc, id, &Value::Null)?;
        let transparent_default = format.alpha && surface.default_pixel().last().is_some_and(|a| a.is_nan() || *a <= 0.0);
        let content = surface.content_bounds();
        // A nontransparent default is visible across the canvas even without stored tiles.
        let extent = if transparent_default { content } else { content.union(&canvas) };
        let bounds = edit_bounds(extent, selected.as_ref());
        // Transparent gaps cannot match. Visit stored tiles instead of the possibly
        // enormous rectangle between two distant pixels.
        let stored = if transparent_default {
            let mut tiles = crate::allocation::capacity(surface.tiles().count(), "Replace Color tile coordinates")?;
            tiles.extend(surface.tiles().map(|(tile, _)| *tile));
            Some(tiles)
        } else {
            None
        };
        // Matching and tone changes are pointwise. Tile-sized work also bounds buffers
        // for full-layer edits, without imposing a new image-size limit.
        let mut process_tile = |tile: photocraft_geom::TileCoord| -> Result<()> {
            let rect = tile.rect().intersect(&bounds);
            if rect.is_empty() {
                return Ok(());
            }
            let mut raw = crate::allocation::read_region(surface, rect, "Replace Color")?;
            let mut lab = crate::allocation::filled(raw.len() / format.channels(), [0.0, 0.0, 0.0, 1.0], "Replace Color")?;
            transform.convert_f32(&raw, format.channels(), lab.as_flattened_mut(), 4, true);
            let width = rect.width() as usize;
            let mut selection_row = crate::allocation::filled(width, 1.0, "Replace Color")?;
            let mut changed = false;
            for (y, (row, lab_row)) in raw.chunks_exact_mut(width * format.channels()).zip(lab.chunks_exact(width)).enumerate() {
                if let Some(selected) = selected.as_ref() {
                    selected.sample_row_strided(rect.y0.saturating_add(y as i32), rect.x0, 1, 0, &mut selection_row);
                }
                for ((pixel, sample), selected) in row.chunks_exact_mut(format.channels()).zip(lab_row).zip(&selection_row) {
                    if sample[3].is_nan() || sample[3] <= 0.0 || *selected <= 0.0 {
                        continue;
                    }
                    let value = [sample[0] * 100.0, sample[1] * 255.0 - 128.0, sample[2] * 255.0 - 128.0];
                    let coverage = if grid {
                        selection::lab_range_coverage_8bit(selection::quantize_lab8(value), &range, fuzz)
                    } else {
                        selection::lab_range_coverage(value, &range, fuzz)
                    };
                    if coverage <= 0.0 {
                        continue;
                    }
                    let original = photocraft_raster::to_rgba(&format, pixel);
                    let mut edited = [original];
                    tone::replace_color_masked(&mut edited, &[coverage], h, sat, l);
                    // Unchanged native samples must not take an incidental RGB roundtrip.
                    if edited[0] == original {
                        continue;
                    }
                    let mixed = std::array::from_fn(|c| original[c] * (1.0 - selected) + edited[0][c] * selected);
                    photocraft_raster::from_rgba_into(&format, mixed, pixel);
                    changed = true;
                }
            }
            if changed {
                surface.try_write_region(rect, &raw).map_err(|e| crate::allocation::error("Replace Color", &e.to_string()))?;
            }
            Ok(())
        };
        if let Some(tiles) = stored {
            for tile in tiles {
                process_tile(tile)?;
            }
        } else {
            for tile in bounds.tiles() {
                process_tile(tile)?;
            }
        }
        Ok(Value::Null)
    })
    .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;

    /// Work buffers follow explicit selected pixels without cropping implicitly selected coverage.
    #[test]
    fn selected_work_bounds_preserve_implicit_selection_and_clip_explicit_pixels() {
        let content = Rect::new(-100, -100, 100, 100);
        let mut mask = Surface::new(PixelFormat::GRAY8);
        assert_eq!(edit_bounds(content, Some(&mask)), Rect::EMPTY);
        mask.fill_rect(Rect::new(98, 96, 110, 105), &[0.5]);
        assert_eq!(edit_bounds(content, Some(&mask)), Rect::new(98, 96, 100, 100));
        assert_eq!(edit_bounds(content, None), content);
        for coverage in [0.5, 1.0] {
            let mut mask = Surface::with_default(PixelFormat::GRAY8, &[coverage]);
            mask.fill_rect(Rect::new(2, 2, 3, 3), &[0.0]);
            assert_eq!(edit_bounds(content, Some(&mask)), content, "stored holes do not bound selected pixels");
        }
    }
}

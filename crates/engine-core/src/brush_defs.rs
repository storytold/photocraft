//! Brush settings checks and sampled tips, shared by the painting tools, Stroke Path and brush
//! preset imports.

use photocraft_paint::{BrushSettings, GrayTile, TipShape};

use crate::{EngineError, Result};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// Rejects brush sizes and Scatter amounts outside their documented ranges, before a brush is
/// stored or painted with.
pub fn validate_brush(brush: &BrushSettings, cmd: &str) -> Result<()> {
    let max = photocraft_paint::MAX_BRUSH_SIZE;
    if !brush.size.is_finite() || brush.size > max {
        return Err(bad(cmd, format!("brush size must be finite and at most {max} px")));
    }
    if brush.dual_brush.enabled && (!brush.dual_brush.size.is_finite() || brush.dual_brush.size > max) {
        return Err(bad(cmd, format!("dual brush size must be finite and at most {max} px")));
    }
    // Scatter may exceed 1 (100 %) but not the 1000 % maximum: a huge amount pushed dab centres to
    // infinity (#977). `{:?}` keeps 1e38 short and shows infinity as `inf`.
    let scatter_max = photocraft_paint::MAX_SCATTER;
    let in_range = |v: f32| (0.0..=scatter_max).contains(&v);
    let sc = brush.scattering.scatter.jitter;
    if brush.scattering.enabled && !in_range(sc) {
        return Err(bad(cmd, format!("scatter amount {sc:?} is out of range: it must be between 0 and {scatter_max} (1000 %)")));
    }
    let ds = brush.dual_brush.scatter;
    if brush.dual_brush.enabled && !in_range(ds) {
        return Err(bad(cmd, format!("dual brush scatter amount {ds:?} is out of range: it must be between 0 and {scatter_max} (1000 %)")));
    }
    Ok(())
}

/// Largest sampled tip side (like Photoshop's 5000 px limit, kept smaller for preset size).
pub const MAX_TIP: u32 = 2500;

/// A sampled-tip brush from a paint-amount map (`w`×`h`, row-major, 0 = no paint, 1 = full):
/// cropped to the painted extent, at most [`MAX_TIP`] px a side. Shared by Define Brush Preset
/// and importing an image file as a brush (`brush.presets.importImage`).
pub fn sampled_brush(cmd: &str, v: &[f32], w: usize, h: usize) -> Result<(BrushSettings, u32, u32)> {
    let at = |x: usize, y: usize| v.get(y * w + x).copied().unwrap_or(0.0);
    // Crop to the painted extent.
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if at(x, y) > 1e-4 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return Err(bad(cmd, "the pixels contain no paint (all white or transparent)"));
    }
    let (tw, th) = ((x1 - x0) as u32, (y1 - y0) as u32);
    if tw.max(th) > MAX_TIP {
        return Err(bad(cmd, format!("too large for a brush tip ({tw}×{th}; max {MAX_TIP} px)")));
    }
    let mut data = Vec::with_capacity((tw * th) as usize);
    for y in y0..y1 {
        data.extend((x0..x1).map(|x| at(x, y)));
    }
    let tip = GrayTile::from_f32(tw, th, &data);
    let brush = BrushSettings { tip: TipShape::Sampled(tip).into(), size: tw.max(th) as f32, spacing: 0.25, pressure_size: false, ..BrushSettings::default() };
    Ok((brush, tw, th))
}

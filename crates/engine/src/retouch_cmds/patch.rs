//! The Patch Tool as a command: heal the selected area with texture from another place.
//!
//! The selection is the patch, `offset` is how far the user dragged it. In `source` mode (the
//! default, Photoshop's "Source") the selected area is repaired with the pixels under the dragged
//! outline; in `destination` mode the selected pixels are copied to the dragged outline and repair
//! it. Either way the copied texture is seamless-cloned (`heal_region`, as the Healing Brush), so it
//! takes the colour and lighting around the repaired area. A feathered selection blends the result
//! in by its coverage. Only the targeted surface is read and written, as Photoshop's Patch Tool in
//! Normal mode samples the current layer.

use super::*;

const CMD: &str = "paint.patch";

/// Largest patch, in pixels of the selection's bounding box: the solve holds a few float planes of
/// it, so this keeps memory to a few GB at worst.
const MAX_PIXELS: u64 = 64 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PatchMode {
    /// The selection is repaired with texture from the dragged-to place.
    Source,
    /// The dragged-to place is repaired with texture from the selection.
    Destination,
}

/// A pixel target and a selection to patch with.
pub(super) fn enabled(s: &Session) -> std::result::Result<(), String> {
    has_pixel_layer(s)?;
    let d = s.active().ok_or("no document open")?;
    if d.doc.selection.is_some() { Ok(()) } else { Err("select the area to patch first".into()) }
}

/// The layer `p` targets (`None` for an alpha channel or the Quick Mask), as `parse_brush`.
fn target_layer(s: &Session, p: &Value) -> Result<Option<LayerId>> {
    if crate::channel_cmds::is_channel_target(p) {
        return Ok(None);
    }
    match p.get("layer").and_then(Value::as_u64) {
        Some(id) => Ok(Some(LayerId(id))),
        None => Ok(Some(s.active().and_then(|d| d.active_layer).ok_or(EngineError::Other("no active layer".into()))?)),
    }
}

/// Integer drag offset, rejecting non-finite and absurd values.
fn offset(p: &Value) -> Result<(i32, i32)> {
    let (dx, dy) = point(p, "offset").ok_or_else(|| bad(CMD, "missing `offset` ([dx, dy])"))?;
    let ok = |v: f64| v.is_finite() && v.abs() < 1e7;
    if !ok(dx) || !ok(dy) {
        return Err(bad(CMD, "`offset` must be two finite numbers"));
    }
    Ok((dx.round() as i32, dy.round() as i32))
}

/// Selected area within the canvas: the selection's content bounds, or the whole canvas when the
/// selection covers everything outside its tiles (an inverted selection).
fn selection_area(sel: &Surface, canvas: Rect) -> Rect {
    let outside = sel.default_pixel().first().copied().unwrap_or(0.0);
    let b = if outside > 0.0 { canvas } else { sel.content_bounds() };
    b.intersect(&canvas)
}

/// Selection coverage over `rect` (row-major, 0..1).
fn coverage(sel: &Surface, rect: Rect) -> Vec<f32> {
    (rect.y0..rect.y1).flat_map(|y| (rect.x0..rect.x1).map(move |x| sel.sample_channel(x, y, 0).clamp(0.0, 1.0))).collect()
}

/// Heal `surf` at the selection (shifted by `dst_shift`) with texture read `src_shift` away.
/// Returns the damaged rectangle.
fn patch_surface(surf: &mut Surface, sel: &Surface, canvas: Rect, area: Rect, dst_shift: (i32, i32), src_shift: (i32, i32), lock: bool) -> Rect {
    // The repaired area plus a two-pixel ring: the ring is the Dirichlet boundary of the solve. It is
    // clipped to where both the repaired and the sampled pixels lie on the canvas, so a patch at the
    // canvas edge sees a free (Neumann) boundary there instead of the empty pixels beyond it.
    let dst_area = area.translate(dst_shift.0, dst_shift.1);
    let g = dst_area.inflate(2).intersect(&canvas).intersect(&canvas.translate(-src_shift.0, -src_shift.1));
    if g.is_empty() {
        return Rect::EMPTY;
    }
    let fmt = surf.format();
    let dst = Region::read(surf, g);
    let mut src = Region::read(surf, g.translate(src_shift.0, src_shift.1));
    src.rect = g;
    // Selection coverage moved to the repaired place.
    let cov = coverage(sel, g.translate(-dst_shift.0, -dst_shift.1));
    let mask: Vec<bool> = cov.iter().map(|c| *c > 0.0).collect();
    let healed = heal_region(&fmt, &src, &dst, &mask);
    apply_coverage(surf, g, &cov, 1.0, None, lock, &healed, BlendMode::Normal)
}

pub(super) fn patch(s: &mut Session, p: &Value) -> Result<Value> {
    let mode = match string(p, "mode", "source") {
        "source" => PatchMode::Source,
        "destination" => PatchMode::Destination,
        o => return Err(bad(CMD, format!("unknown mode `{o}` (source|destination)"))),
    };
    let (dx, dy) = offset(p)?;
    let id = target_layer(s, p)?;
    let d = s.active().ok_or(EngineError::Other("no document open".into()))?;
    let canvas = d.doc.bounds();
    let sel = d.doc.selection.as_ref().ok_or_else(|| bad(CMD, "select the area to patch first"))?;
    let area = selection_area(sel, canvas);
    if area.is_empty() {
        return Err(bad(CMD, "the selection is empty"));
    }
    let pixels = u64::from(area.width()) * u64::from(area.height());
    if pixels > MAX_PIXELS {
        return Err(bad(CMD, format!("the selection is too large to patch ({pixels} pixels, at most {MAX_PIXELS})")));
    }
    if (dx, dy) == (0, 0) {
        return Err(bad(CMD, "drag the selection to the area to sample from"));
    }
    // Both the texture and the repaired area must lie on the canvas: off-canvas pixels are empty.
    let (dst_shift, src_shift) = match mode {
        PatchMode::Source => ((0, 0), (dx, dy)),
        PatchMode::Destination => ((dx, dy), (-dx, -dy)),
    };
    let moved = area.translate(dx, dy);
    if !canvas.contains_rect(&moved) {
        return Err(bad(CMD, "the dragged patch must stay inside the canvas"));
    }
    let label = if mode == PatchMode::Source { "Patch" } else { "Patch (Destination)" };
    let dmg = run_stroke(s, label, id, p, |pre, surf, _, lock| {
        let sel = pre.selection.as_ref().ok_or_else(|| bad(CMD, "select the area to patch first"))?;
        Ok(patch_surface(surf, sel, canvas, area, dst_shift, src_shift, lock))
    })?;
    Ok(json!({ "damage": damage_json(dmg), "offset": [dx, dy] }))
}

//! Read-only comparison of the active document's rendered pixels with a History snapshot.
//!
//! Both sides use the CPU compositor's straight-alpha RGBA f32 output. This does not compare
//! native layer channels, exported bytes, or colour-managed display pixels. Masks, Smart Filters
//! and earlier deformations are included. The active selection does not restrict the requested
//! regions; uncommitted floating pieces and GUI preview overlays are excluded.

use photocraft_compose::Buffer;
use photocraft_doc::Document;
use photocraft_geom::Rect;
use serde::{Deserialize, de::IgnoredAny};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const COMMAND: &str = "document.comparePixels";
const MAX_REGIONS: usize = 32;
/// Count overlaps again: each requested region is rendered and reported independently.
const MAX_COMPARED_PIXELS: u64 = 1_048_576;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    snapshot: u64,
    regions: Vec<Region>,
    #[serde(default)]
    tolerance: f64,
    // Every engine command accepts these universal parameters. A read-only comparison
    // does not coalesce an edit or route the inspection to a targeted pixel channel.
    #[serde(default, rename = "coalesce")]
    _coalesce: Option<IgnoredAny>,
    #[serde(default, rename = "target")]
    _target: Option<IgnoredAny>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Region {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Default)]
struct Difference {
    compared_pixels: u64,
    changed_pixels: u64,
    max_channel_difference: f64,
}

/// Attach this command's id to an actionable parameter error.
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: COMMAND.into(), msg: message.into() }
}

/// Read the strict request schema after bounding the number of regions to allocate.
fn params(value: &Value) -> Result<Params> {
    // Bound the array before deserializing it, including callers outside the RPC reply budget.
    let regions = value.get("regions").and_then(Value::as_array).ok_or_else(|| bad("`regions` must be a nonempty array"))?;
    if regions.is_empty() || regions.len() > MAX_REGIONS {
        return Err(bad("`regions` must contain 1..32 rectangles"));
    }
    let params = Params::deserialize(value).map_err(|e| bad(format!("invalid comparison parameters: {e}")))?;
    if !params.tolerance.is_finite() || params.tolerance < 0.0 {
        return Err(bad("`tolerance` must be finite and nonnegative"));
    }
    Ok(params)
}

/// Reject rather than clip invalid/outside rectangles: silently comparing a smaller area could
/// incorrectly reassure a caller that every requested protected pixel was checked.
fn regions(requested: &[Region], bounds: Rect) -> Result<Vec<Rect>> {
    let mut total = 0u64;
    requested
        .iter()
        .map(|region| {
            if region.width == 0 || region.height == 0 {
                return Err(bad("region width and height must be positive"));
            }
            let x1 = region.x.checked_add_unsigned(region.width).ok_or_else(|| bad("region x + width overflows"))?;
            let y1 = region.y.checked_add_unsigned(region.height).ok_or_else(|| bad("region y + height overflows"))?;
            let rect = Rect::new(region.x, region.y, x1, y1);
            if !bounds.contains_rect(&rect) {
                return Err(bad("every region must be entirely inside the document and snapshot bounds"));
            }
            let area = u64::from(region.width) * u64::from(region.height);
            total = total
                .checked_add(area)
                .filter(|&n| n <= MAX_COMPARED_PIXELS)
                .ok_or_else(|| bad("regions exceed 1048576 compared pixels (overlaps count again)"))?;
            Ok(rect)
        })
        .collect()
}

/// Compare all four rendered channels, including alpha, without quantizing or clamping HDR.
/// The maximum records the raw difference even when it is below the counting tolerance.
fn accumulate(before: &Buffer, after: &Buffer, rect: Rect, tolerance: f64, difference: &mut Difference) -> Result<()> {
    let expected = usize::try_from(rect.size().area()).map_err(|_| EngineError::Other("comparison tile is too large".into()))?;
    if before.rect != rect || after.rect != rect || before.px.len() != expected || after.px.len() != expected {
        return Err(EngineError::Other("the compositor returned an incomplete comparison tile".into()));
    }
    for (offset, (a, b)) in before.px.iter().zip(&after.px).enumerate() {
        let mut maximum = 0.0f64;
        for (&a, &b) in a.iter().zip(b) {
            if !a.is_finite() || !b.is_finite() {
                let width = rect.width() as usize;
                let x = i64::from(rect.x0) + (offset % width) as i64;
                let y = i64::from(rect.y0) + (offset / width) as i64;
                return Err(EngineError::Other(format!("non-finite rendered channel at ({x}, {y}); pixel comparison cannot continue")));
            }
            maximum = maximum.max((f64::from(a) - f64::from(b)).abs());
        }
        difference.compared_pixels += 1;
        difference.changed_pixels += u64::from(maximum > tolerance);
        difference.max_channel_difference = difference.max_channel_difference.max(maximum);
    }
    Ok(())
}

/// Compare one validated region in bounded tiles through the canonical CPU compositor.
fn compare_region(before: &Document, after: &Document, rect: Rect, tolerance: f64) -> Result<Difference> {
    let mut difference = Difference::default();
    // Two 256² RGBA f32 buffers use at most 2 MiB. The compositor retains its existing
    // effect/Smart Object cache budgets; this is not a cap on its internal working memory.
    for tile in rect.tiles() {
        let tile = tile.rect().intersect(&rect);
        let original = photocraft_compose::render(before, tile);
        let current = photocraft_compose::render(after, tile);
        accumulate(&original, &current, tile, tolerance, &mut difference)?;
    }
    Ok(difference)
}

/// Compare committed composites without selecting a snapshot or editing either document.
fn compare(s: &mut Session, value: &Value) -> Result<Value> {
    let params = params(value)?;
    let state = s.active().ok_or(EngineError::NoDocument)?;
    let snapshot = state
        .snapshots
        .iter()
        .find(|snapshot| snapshot.id == params.snapshot)
        .ok_or_else(|| bad(format!("no snapshot {} in the active document", params.snapshot)))?;
    let (before, after) = (&snapshot.doc, &state.doc);
    if before.bounds() != after.bounds() || before.mode != after.mode || before.depth != after.depth || before.icc_profile != after.icc_profile {
        return Err(bad("document and snapshot must have identical bounds, colour mode, depth and ICC profile"));
    }
    let requested = regions(&params.regions, after.bounds())?;
    let mut results = Vec::with_capacity(requested.len());
    for rect in requested {
        let difference = compare_region(before, after, rect, params.tolerance)?;
        results.push(json!({
            "rect": [rect.x0, rect.y0, rect.x1, rect.y1],
            "comparedPixels": difference.compared_pixels,
            "changedPixels": difference.changed_pixels,
            "maxChannelDifference": difference.max_channel_difference,
        }));
    }
    Ok(json!({"snapshot": params.snapshot, "space": "composited-rgba-f32", "tolerance": params.tolerance, "regions": results}))
}

/// Register the read-only snapshot comparison for the engine, CLI and MCP command routes.
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: COMMAND,
        label: "Compare Document Pixels",
        menu: &[],
        shortcut: None,
        params: r##"{"snapshot":id,"regions":[{"x":integer,"y":integer,"width":positive integer,"height":positive integer}…],"tolerance":finite nonnegative number=0} → {snapshot,space:"composited-rgba-f32",tolerance,regions:[{rect:[x0,y0,x1,y1],comparedPixels,changedPixels,maxChannelDifference}]}: read-only CPU straight-alpha RGBA f32 comparison with a History snapshot; 1..32 regions, fully inside matching bounds/mode/depth/ICC profile, total at most 1048576 pixels including overlaps. A pixel counts when any channel difference exceeds tolerance; maximum includes differences below tolerance. Universal coalesce and target parameters are accepted and ignored. Native hidden channels, export bytes and display colour are not compared. Non-finite rendered channels are errors."##,
        enabled: |s| s.active().map(|_| ()).ok_or_else(|| "no document open".into()),
        run: compare,
        journal: false,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_hdr_differences_are_not_lost_to_float_overflow() {
        let rect = Rect::new(0, 0, 1, 1);
        let before = Buffer::filled(rect, [-f32::MAX, 0.0, 0.0, 1.0]);
        let after = Buffer::filled(rect, [f32::MAX, 0.0, 0.0, 1.0]);
        let mut difference = Difference::default();
        accumulate(&before, &after, rect, 0.0, &mut difference).unwrap();
        assert_eq!(difference.changed_pixels, 1);
        assert_eq!(difference.max_channel_difference, 2.0 * f64::from(f32::MAX));
        assert!(difference.max_channel_difference.is_finite());
    }

    #[test]
    fn non_finite_rendered_samples_are_errors_not_unchanged_pixels() {
        let rect = Rect::new(0, 0, 1, 1);
        let before = Buffer::filled(rect, [0.0, 0.0, 0.0, 0.0]);
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let after = Buffer::filled(rect, [invalid, 0.0, 0.0, 0.0]);
            assert!(accumulate(&before, &after, rect, 0.0, &mut Difference::default()).is_err());
            assert!(accumulate(&after, &before, rect, 0.0, &mut Difference::default()).is_err());
        }
    }
}

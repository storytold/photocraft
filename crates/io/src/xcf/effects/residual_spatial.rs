//! Shared native-conversion helpers; mapping registration follows separately.

#[allow(unused_imports)]
use super::{ArgumentValue, MappingContext, MappingFailure, MappingResult, NativeBranch, NativeMapping, NativeStructure, StoredEffect, native_filter};

#[allow(unused_imports)]
use photocraft_color::{BlendMode, Color};

#[allow(unused_imports)]
use photocraft_doc::effects::{Contour, Effect, FxCommon, FxPaint, Glow, GlowSource, GlowTechnique, Shadow};

#[allow(unused_imports)]
use photocraft_doc::{Fill, Group, Layer, LayerContent, Path, ShapeLayer, Subpath, VectorMask};

#[allow(unused_imports)]
use photocraft_geom::{Affine, Rect};

#[allow(unused_imports)]
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, MappingFailure>;

const MAX_GEOMETRY: usize = 65_536;

/// Classifies malformed optional spatial data without confusing it with an unavailable native resource.
pub(super) fn invalid(message: impl Into<String>) -> MappingFailure {
    MappingFailure { reason: photocraft_doc::import_conversion::ConversionReason::InvalidOptionalData, message: message.into() }
}

/// Observes import cancellation before constructing native geometry or generated resources.
pub(super) fn generation_check(c: &MappingContext) -> Result<()> {
    c.interrupt.check().map_err(|_| MappingFailure::resource("native generation was cancelled"))
}

/// Rejects generated working data beyond the remaining allowance before allocation, with cancellation taking precedence.
pub(super) fn generation_bytes(c: &MappingContext, bytes: usize) -> Result<()> {
    generation_check(c)?;
    if bytes > c.remaining_generation_bytes {
        return Err(MappingFailure::resource("native generated data exceeds the actual remaining import byte allowance"));
    }
    Ok(())
}

/// Checks overflow and conservative geometry storage against both remaining metadata and generation budgets.
pub(super) fn geometry_bytes(c: &MappingContext, paths: usize, knots: usize) -> Result<()> {
    let bytes = paths
        .checked_mul(std::mem::size_of::<Subpath>())
        .and_then(|v| knots.checked_mul(std::mem::size_of::<photocraft_doc::Knot>()).and_then(|k| v.checked_add(k)))
        .and_then(|v| v.checked_mul(2))
        .and_then(|v| v.checked_add(4096))
        .ok_or_else(|| MappingFailure::resource("native geometry allocation size overflows"))?;
    if bytes > c.remaining_metadata_bytes {
        return Err(MappingFailure::resource("native geometry exceeds the actual remaining metadata allowance"));
    }
    generation_bytes(c, bytes)
}

/// Reads a finite source scalar using representable stored endpoints; rejects out-of-domain values rather than clamping them.
pub(super) fn num(e: &StoredEffect<'_>, key: &str, default: f64, lo: f64, hi: f64) -> Result<f64> {
    let v = e.number(key).map_err(invalid)?.unwrap_or(default);
    if !v.is_finite() || v < f64::from(lo as f32) || v > f64::from(hi as f32) {
        return Err(invalid(format!("{}: {key} outside source domain", e.operation)));
    }
    Ok(v)
}

/// Preserves exact gint endpoints while rejecting fractional, nonfinite or out-of-range geometry controls.
pub(super) fn integer(e: &StoredEffect<'_>, key: &str, default: i32, lo: i32, hi: i32) -> Result<i32> {
    // Stored gint must stay integer; do not pass i32 endpoints through rounded F32.
    let v = e.number(key).map_err(invalid)?.unwrap_or(f64::from(default));
    if !v.is_finite() || v.fract() != 0. || v < f64::from(lo) || v > f64::from(hi) {
        return Err(invalid(format!("{key} must be an in-range source integer")));
    }
    Ok(v as i32)
}

/// Accepts only the declared numeric enum values, preserving unknown choices as conversion failures.
pub(super) fn choice(e: &StoredEffect<'_>, key: &str, default: i32, values: &[i32]) -> Result<i32> {
    let v = e.enum_value(key).map_err(invalid)?.unwrap_or(default);
    if !values.contains(&v) {
        return Err(invalid(format!("{key}: unknown enum {v}")));
    }
    Ok(v)
}

/// Reads a typed optional boolean, using the declared default only when the argument is absent.
pub(super) fn boolean(e: &StoredEffect<'_>, key: &str, default: bool) -> Result<bool> {
    Ok(e.boolean(key).map_err(invalid)?.unwrap_or(default))
}

/// Requires the stored unsigned seed representation so signed or unrelated argument types cannot become random state.
pub(super) fn seed(e: &StoredEffect<'_>) -> Result<u32> {
    match e.argument("seed").map_err(invalid)? {
        None => Ok(0),
        Some(ArgumentValue::UInt(v)) => Ok(*v),
        _ => Err(invalid("seed must be stored guint")),
    }
}

/// Converts typed source paint through native CMS and distinguishes missing color resources from malformed values.
pub(super) fn color(e: &StoredEffect<'_>, key: &str, default: [f32; 4], c: &MappingContext) -> Result<Color> {
    let result =
        if e.argument(key).map_err(invalid)?.is_some() { super::resources::native_color(e, key, c) } else { super::resources::native_rgba(default, c) };
    result.map_err(|s| if s.starts_with("resource unavailable:") { MappingFailure::resource(s) } else { invalid(s) })
}

/// Packages ordinary native filters with the explanatory policy retained in the import report.
pub(super) fn mapped(filters: Vec<photocraft_doc::SmartFilter>, note: impl Into<String>) -> NativeMapping {
    NativeMapping { filters, notes: vec![note.into()], ..Default::default() }
}

/// Constructs a single ordinary native filter without carrying a source operation into replay data.
pub(super) fn one(command: &str, params: Value, note: impl Into<String>) -> NativeMapping {
    mapped(vec![native_filter(command, params)], note)
}

/// Wraps an editable native replacement layer and its conversion note without inventing pattern resources.
pub(super) fn replace(layer: Layer, note: impl Into<String>) -> NativeMapping {
    NativeMapping { structure: Some(NativeStructure::Replace { layer: Box::new(layer), patterns: Vec::new() }), notes: vec![note.into()], ..Default::default() }
}

/// Builds an ordinary native branch composition with an explicit approximation note.
pub(super) fn composite(branches: Vec<NativeBranch>, note: impl Into<String>) -> NativeMapping {
    NativeMapping { structure: Some(NativeStructure::Composite { branches }), notes: vec![note.into()], ..Default::default() }
}

/// Requires a nonempty native evaluation frame before calculating geometry or normalized positions.
pub(super) fn bounds(c: &MappingContext) -> Result<Rect> {
    if c.native_filter_bounds.is_empty() { Err(invalid("native geometry needs nonempty evaluation bounds")) } else { Ok(c.native_filter_bounds) }
}

/// Authors a closed four-corner native polygon from already validated rectangle coordinates.
pub(super) fn rect(x: f64, y: f64, w: f64, h: f64) -> Subpath {
    Subpath::polygon(&[(x, y), (x + w, y), (x + w, y + h), (x, y + h)])
}

/// Creates an editable native filled path layer from supplied subpaths and CMS-converted paint.
pub(super) fn shape(name: &str, paths: Vec<Subpath>, paint: Color) -> Layer {
    Layer::new(name, LayerContent::Shape(ShapeLayer { path: Path::new(paths), fill: Some(Fill::Solid(paint)), ..Default::default() }))
}

/// Advances the deterministic local generator used for reproducible native maze geometry.
pub(super) fn random(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *state
}

/// Constructs bounded native maze walls with declared spacing, colors and seed; opposite-edge connectivity follows the selected tileability policy.
fn maze(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let b = bounds(c)?;
    let sx = integer(e, "x", 16, 1, i32::MAX)?;
    let sy = integer(e, "y", 16, 1, i32::MAX)?;
    let algorithm = choice(e, "algorithm-type", 0, &[0, 1])?;
    let tile = boolean(e, "tileable", false)?;
    let mut rng = seed(e)?;
    let fg = color(e, "fg-color", [0., 0., 0., 1.], c)?;
    let bg = color(e, "bg-color", [1., 1., 1., 1.], c)?;
    let nx = (i64::from(b.width()) + i64::from(sx) - 1) / i64::from(sx);
    let ny = (i64::from(b.height()) + i64::from(sy) - 1) / i64::from(sy);
    let count = nx.checked_mul(ny).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| invalid("maze cell count overflow"))?;
    if count.checked_mul(4).is_none_or(|n| n > MAX_GEOMETRY) {
        return Err("native maze exceeds65,536 bounded cell/wall work items".into());
    }
    geometry_bytes(
        c,
        count.checked_mul(4).ok_or_else(|| invalid("maze geometry overflow"))?,
        count.checked_mul(16).ok_or_else(|| invalid("maze knot count overflow"))?,
    )?;
    let mut walls = vec![15u8; count];
    let mut seen = vec![false; count];
    let mut frontier = vec![0usize];
    if let Some(v) = seen.first_mut() {
        *v = true;
    }
    while !frontier.is_empty() {
        generation_check(c)?;
        let index = if algorithm == 0 { frontier.len() - 1 } else { random(&mut rng) as usize % frontier.len() };
        let cell = *frontier.get(index).ok_or_else(|| invalid("maze frontier"))?;
        let cell_i = i64::try_from(cell).map_err(|_| invalid("maze cell index"))?;
        let x = cell_i % nx;
        let y = cell_i / nx;
        let mut next = Vec::new();
        for (dx, dy, bit, opposite) in [(0, -1, 1u8, 4u8), (1, 0, 2, 8), (0, 1, 4, 1), (-1, 0, 8, 2)] {
            let mut px = x + dx;
            let mut py = y + dy;
            if tile {
                px = px.rem_euclid(nx);
                py = py.rem_euclid(ny);
            } else if px < 0 || px >= nx || py < 0 || py >= ny {
                continue;
            }
            let dest = usize::try_from(py * nx + px).map_err(|_| invalid("maze neighbor"))?;
            if !seen.get(dest).copied().ok_or_else(|| invalid("maze visited index"))? {
                next.push((dest, bit, opposite));
            }
        }
        if next.is_empty() {
            frontier.swap_remove(index);
            continue;
        }
        let n = random(&mut rng) as usize % next.len();
        let (dest, bit, opposite) = next.get(n).copied().ok_or_else(|| invalid("maze random neighbor"))?;
        *walls.get_mut(cell).ok_or_else(|| invalid("maze source cell"))? &= !bit;
        *walls.get_mut(dest).ok_or_else(|| invalid("maze destination cell"))? &= !opposite;
        *seen.get_mut(dest).ok_or_else(|| invalid("maze mark"))? = true;
        frontier.push(dest);
    }
    let mut paths = Vec::new();
    for (cell, wall) in walls.iter().enumerate() {
        generation_check(c)?;
        let cell_i = cell as i64;
        let x = f64::from(b.x0) + (cell_i % nx) as f64 * f64::from(sx);
        let y = f64::from(b.y0) + (cell_i / nx) as f64 * f64::from(sy);
        let w = f64::from(sx).min(f64::from(b.x1) - x);
        let h = f64::from(sy).min(f64::from(b.y1) - y);
        if wall & 1 != 0 {
            paths.push(rect(x, y, w, 1.));
        }
        if wall & 8 != 0 {
            paths.push(rect(x, y, 1., h));
        }
        if cell_i / nx == ny - 1 && wall & 4 != 0 {
            paths.push(rect(x, y + h - 1., w, 1.));
        }
        if cell_i % nx == nx - 1 && wall & 2 != 0 {
            paths.push(rect(x + w - 1., y, 1., h));
        }
    }
    let mut background = Layer::new("Maze background", LayerContent::Fill(Fill::Solid(bg)));
    background.vector_mask = Some(VectorMask::new(Path::new(vec![rect(f64::from(b.x0), f64::from(b.y0), f64::from(b.width()), f64::from(b.height()))])));
    let layer = Layer::new(
        "Imported editable maze",
        LayerContent::Group(Group { children: vec![background, shape("Maze walls", paths, fg)], expanded: true, artboard: None, frame_stroke: None }),
    );
    Ok(replace(
        layer,
        format!(
            "Native editable maze walls use source cell spacing/seed/colors and {} spanning-tree construction; source tileability={tile} connects opposite-edge cells. Independently specified native random sequence, one-pixel wall width and antialiasing differ from source maze rasterization; all generated geometry remains editable",
            if algorithm == 0 { "depth-first" } else { "random frontier" }
        ),
    ))
}

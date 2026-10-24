//! Saved XCF paths translated to ordinary document-pixel cubic geometry.
//!
//! Wire layouts: public XCF PATHS/VECTORS records and version-18 standalone path
//! records. Pressure, item presentation flags and source parasites remain inert;
//! none of these records authorize paint or external resources.

use photocraft_color::Color;
use photocraft_doc::Fill;
use photocraft_doc::vector::{FillRule, Knot, LineCap, LineJoin, NamedPath, Path, PathOp, ShapeLayer, ShapeStroke, Subpath};
use photocraft_geom::Point;

use super::{Cursor, Result, invalid, props};

const MAX_PATHS: usize = 1024;
const MAX_POINTS: usize = 262_144;
const MAX_COORD: f64 = 10_000_000.0;

pub(super) struct DecodedPath {
    pub named: NamedPath,
    pub tattoo: Option<u32>,
    pub selected: bool,
    /// Source flags, pressure/tilt, parasites or unsupported stroke records.
    pub source_only: bool,
}

pub(super) struct VectorStyle {
    pub path_tattoo: u32,
    pub modified: bool,
    pub shape: ShapeLayer,
    pub source_only: Vec<String>,
}

fn boolean(c: &mut Cursor<'_>) -> Result<bool> {
    match c.u32()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid("invalid vector boolean")),
    }
}

fn paint(c: &mut Cursor<'_>, notes: &mut Vec<String>) -> Result<Fill> {
    let style = c.u32()?;
    let aa = boolean(c)?;
    if !aa {
        notes.push("Source paint antialiasing is disabled; native shape rasterization applies".into());
    }
    let encoding = string(c)?;
    let color = if encoding.is_empty() {
        notes.push("Source paint has no stored color; native black is an explicit editable approximation".into());
        Color::BLACK
    } else {
        let length = count(c, 256, "vector color byte")?;
        let raw = c.take(length)?;
        let profile_length = count(c, super::MAX_PAYLOAD, "vector color profile byte")?;
        let profile = c.take(profile_length)?;
        if encoding == "R'G'B'A u8" && length == 4 && profile_length == 0 {
            let mut values = [0.0; 4];
            for (out, &v) in values.iter_mut().zip(raw) {
                *out = f32::from(v) / 255.0;
            }
            let [r, g, b, a] = values;
            Color::rgba(r, g, b, a)
        } else {
            match super::effects::literal_rgb(&super::effects::ColorLiteral { encoding: &encoding, pixels: raw, profile }) {
                Ok([r, g, b, a]) => Color::rgba(r, g, b, a),
                Err(message) => {
                    notes.push(format!("Source color {encoding} retained; native black approximation used: {message}"));
                    Color::BLACK
                }
            }
        }
    };
    // A source resource identity never becomes permission to open a file.
    let resource = string(c)?;
    if !resource.is_empty() {
        string(c)?;
        boolean(c)?;
    }
    match style {
        0 => Ok(Fill::Solid(color)),
        1 => {
            notes.push("Source pattern resource is unavailable; native solid paint approximation used and resource identity retained".into());
            Ok(Fill::Solid(color))
        }
        _ => Err(invalid("unsupported vector paint style")),
    }
}

/// PROP_VECTOR_LAYER style, separate from its named saved-path geometry.
/// The caller resolves `path_tattoo` only against paths in the same source image.
/// Modified vector pixels must remain a raster fallback rather than using this
/// shape. Every successful shape starts without a source raster cache.
pub(super) fn vector_style(data: &[u8]) -> Result<VectorStyle> {
    let mut c = Cursor { b: data, pos: 0, wide: false };
    let modified = boolean(&mut c)?;
    let path_tattoo = c.u32()?;
    let fill_enabled = boolean(&mut c)?;
    let stroke_enabled = boolean(&mut c)?;
    let mut source_only = Vec::new();
    let fill = paint(&mut c, &mut source_only)?;
    let stroke_paint = paint(&mut c, &mut source_only)?;
    let width = coordinate(&mut c)? as f32;
    let cap = match c.u32()? {
        0 => LineCap::Butt,
        1 => LineCap::Round,
        2 => LineCap::Square,
        _ => return Err(invalid("invalid vector line cap")),
    };
    let join = match c.u32()? {
        0 => LineJoin::Miter,
        1 => LineJoin::Round,
        2 => LineJoin::Bevel,
        _ => return Err(invalid("invalid vector line join")),
    };
    let miter_limit = coordinate(&mut c)? as f32;
    let n = count(&mut c, 32, "stroke dash")?;
    let mut dashes = Vec::new();
    for _ in 0..n {
        dashes.push(coordinate(&mut c)? as f32);
    }
    let stroke = ShapeStroke { width, paint: stroke_paint, cap, join, miter_limit, dashes, ..Default::default() };
    stroke.validate().map_err(invalid)?;
    if c.pos != data.len() {
        return Err(invalid("trailing vector layer configuration bytes"));
    }
    Ok(VectorStyle {
        path_tattoo,
        modified,
        shape: ShapeLayer { fill: fill_enabled.then_some(fill), stroke: stroke_enabled.then_some(stroke), ..Default::default() },
        source_only,
    })
}

fn string(c: &mut Cursor<'_>) -> Result<String> {
    let length = count(c, super::MAX_STRING, "path string byte")?;
    if length == 0 {
        return Ok(String::new());
    }
    let raw = c.take(length)?;
    let raw = raw.strip_suffix(&[0]).ok_or_else(|| invalid("path string lacks its NUL terminator"))?;
    std::str::from_utf8(raw).map(str::to_owned).map_err(|_| invalid("invalid UTF-8 path name"))
}

fn count(c: &mut Cursor<'_>, limit: usize, what: &str) -> Result<usize> {
    let n = c.u32()? as usize;
    if n > limit {
        return Err(invalid(format!("{what} count exceeds {limit}")));
    }
    Ok(n)
}

fn coordinate(c: &mut Cursor<'_>) -> Result<f64> {
    let v = f64::from(f32::from_bits(c.u32()?));
    if !v.is_finite() || v.abs() > MAX_COORD {
        return Err(invalid("path coordinate is non-finite or exceeds the native geometry bound"));
    }
    Ok(v)
}

fn native_path(mut subpaths: Vec<Subpath>) -> Path {
    // GIMP's saved-path fill uses the even-odd rule across the whole path.
    // Native Join makes the subpaths one shape component (rather than unioning
    // independently filled contours, which would incorrectly fill holes).
    for subpath in subpaths.iter_mut().skip(1) {
        subpath.op = PathOp::Join;
    }
    Path { subpaths, fill_rule: FillRule::EvenOdd, inverted: false }
}

fn strokes(c: &mut Cursor<'_>, n: usize, budget: &mut usize) -> Result<(Path, bool)> {
    let mut out = Vec::new();
    let mut source_only = false;
    for _ in 0..n {
        let kind = c.u32()?;
        let closed = c.u32()?;
        let axes = count(c, 6, "path axis")?;
        let points = count(c, *budget, "path point")?;
        *budget = budget.saturating_sub(points);
        if axes < 2 || closed > 1 {
            return Err(invalid("invalid path axes or closed flag"));
        }
        // Validate the whole framing before allocating geometry.
        let bytes = points
            .checked_mul(axes.checked_add(1).and_then(|n| n.checked_mul(4)).ok_or_else(|| invalid("path stride overflow"))?)
            .ok_or_else(|| invalid("path point size overflow"))?;
        let raw = c.take(bytes)?;
        if kind != 1 {
            source_only = true;
            continue;
        }
        if !points.is_multiple_of(3) {
            return Err(invalid("cubic path control points must occur in handle/anchor/handle triples"));
        }
        let mut p = Cursor { b: raw, pos: 0, wide: false };
        let mut knots = Vec::new();
        for _ in 0..points / 3 {
            let mut triple = [Point::new(0.0, 0.0); 3];
            for (i, point) in triple.iter_mut().enumerate() {
                if p.u32()? != if i == 1 { 0 } else { 1 } {
                    return Err(invalid("invalid cubic path anchor ordering"));
                }
                *point = Point::new(coordinate(&mut p)?, coordinate(&mut p)?);
                for _ in 2..axes {
                    // Validate source-only pressure/tilt/wheel without using them
                    // as native curve or paint instructions.
                    let v = f32::from_bits(p.u32()?);
                    if !v.is_finite() {
                        return Err(invalid("non-finite path auxiliary coordinate"));
                    }
                    source_only = true;
                }
            }
            let [in_ctrl, anchor, out_ctrl] = triple;
            knots.push(Knot { anchor, in_ctrl, out_ctrl, smooth: false });
        }
        out.push(Subpath { closed: closed != 0, knots, ..Default::default() });
    }
    Ok((native_path(out), source_only))
}

/// The pre-version-18 image PROP_VECTORS payload.
pub(super) fn vectors(data: &[u8]) -> Result<Vec<DecodedPath>> {
    let mut c = Cursor { b: data, pos: 0, wide: false };
    if c.u32()? != 1 {
        return Err(invalid("unsupported saved-vector record version"));
    }
    let active = c.u32()? as usize;
    let n = count(&mut c, MAX_PATHS, "saved path")?;
    let mut budget = MAX_POINTS;
    let mut out = Vec::new();
    for i in 0..n {
        let name = string(&mut c)?;
        let tattoo = c.u32()?;
        let visible = c.u32()?;
        let linked = c.u32()?;
        let parasites = count(&mut c, 4096, "path parasite")?;
        let n = count(&mut c, MAX_POINTS, "path stroke")?;
        for _ in 0..parasites {
            string(&mut c)?;
            c.u32()?;
            let bytes = count(&mut c, super::MAX_PAYLOAD, "path parasite byte")?;
            c.take(bytes)?;
        }
        let (path, source_only) = strokes(&mut c, n, &mut budget)?;
        out.push(DecodedPath {
            named: NamedPath { name, path, psd_raw: None },
            tattoo: (tattoo != 0).then_some(tattoo),
            selected: i == active,
            source_only: source_only || visible != 0 || linked != 0 || parasites != 0,
        });
    }
    if c.pos != data.len() {
        return Err(invalid("trailing saved-vector payload bytes"));
    }
    Ok(out)
}

/// The oldest PROP_PATHS records. Coordinates are signed integers in record
/// version 1 and IEEE binary32 in versions 2/3 (the historical comment calling
/// them doubles does not match the on-disk float reader/writer).
pub(super) fn legacy(data: &[u8]) -> Result<Vec<DecodedPath>> {
    let mut c = Cursor { b: data, pos: 0, wide: false };
    let active = c.u32()? as usize;
    let n = count(&mut c, MAX_PATHS, "legacy path")?;
    let mut budget = MAX_POINTS;
    let mut out = Vec::new();
    for index in 0..n {
        let name = string(&mut c)?;
        let linked = c.u32()?;
        let state = c.take(1)?.first().copied().unwrap_or(0);
        let closed = c.u32()?;
        let points = count(&mut c, budget, "legacy path point")?;
        budget = budget.saturating_sub(points);
        let version = c.u32()?;
        let tattoo = match version {
            1 => None,
            2 => {
                c.u32()?;
                None
            }
            3 => {
                c.u32()?;
                Some(c.u32()?).filter(|v| *v != 0)
            }
            _ => return Err(invalid("unsupported legacy path record version")),
        };
        if closed > 1 {
            return Err(invalid("invalid legacy path closure"));
        }
        let mut records = Vec::new();
        // Check all bytes before allocating, with one aggregate point budget.
        let raw = c.take(points.checked_mul(12).ok_or_else(|| invalid("legacy path size overflow"))?)?;
        let mut p = Cursor { b: raw, pos: 0, wide: false };
        for _ in 0..points {
            let kind = p.u32()?;
            let point = if version == 1 {
                Point::new(f64::from(p.u32()? as i32), f64::from(p.u32()? as i32))
            } else {
                Point::new(coordinate(&mut p)?, coordinate(&mut p)?)
            };
            if !(1..=3).contains(&kind) || point.x.abs() > MAX_COORD || point.y.abs() > MAX_COORD {
                return Err(invalid("invalid legacy path point"));
            }
            records.push((kind, point));
        }
        let mut subpaths = Vec::new();
        let mut start = 0;
        for end in 1..=records.len() {
            let boundary = records.get(end).is_none_or(|(kind, _)| *kind == 3);
            if !boundary {
                continue;
            }
            let part = records.get(start..end).ok_or_else(|| invalid("legacy path range"))?;
            let is_closed = end < records.len() || closed != 0;
            let first = part.first().ok_or_else(|| invalid("empty legacy path stroke"))?;
            if !matches!(first.0, 1 | 3) {
                return Err(invalid("legacy stroke does not start with an anchor"));
            }
            // Legacy sequence is A,O,I,A,O,I,...,A,O for an open
            // curve; a closed curve also stores the first incoming handle last.
            let needed = if is_closed { 0 } else { 2 };
            if part.len() % 3 != needed {
                return Err(invalid("invalid legacy cubic point sequence"));
            }
            let mut knots = Vec::new();
            for (i, &(kind, anchor)) in part.iter().enumerate().step_by(3) {
                if !matches!(kind, 1 | 3) {
                    return Err(invalid("legacy cubic anchor expected"));
                }
                let outgoing = part.get(i + 1).filter(|(kind, _)| *kind == 2).map(|(_, p)| *p).ok_or_else(|| invalid("legacy outgoing handle expected"))?;
                let incoming = if i == 0 {
                    if is_closed { part.last().map(|(_, p)| *p).unwrap_or(anchor) } else { anchor }
                } else {
                    part.get(i - 1).filter(|(kind, _)| *kind == 2).map(|(_, p)| *p).ok_or_else(|| invalid("legacy incoming handle expected"))?
                };
                knots.push(Knot { anchor, in_ctrl: incoming, out_ctrl: outgoing, smooth: false });
            }
            subpaths.push(Subpath { closed: is_closed, knots, ..Default::default() });
            start = end;
        }
        out.push(DecodedPath {
            named: NamedPath { name, path: native_path(subpaths), psd_raw: None },
            tattoo,
            selected: index == active,
            source_only: linked != 0 || state != 0,
        });
    }
    if c.pos != data.len() {
        return Err(invalid("trailing legacy path bytes"));
    }
    Ok(out)
}

/// A standalone version-18+ path, bounded by its declared payload length.
pub(super) fn standalone(c: &Cursor<'_>, offset: usize) -> Result<DecodedPath> {
    let mut c = c.at(offset);
    let name = string(&mut c)?;
    let length = count(&mut c, super::MAX_PAYLOAD, "path payload byte")?;
    let raw = c.take(length)?;
    let mut p = Cursor { b: raw, pos: 0, wide: false };
    let properties = props(&mut p)?;
    let mut tattoo = None;
    let mut selected = false;
    let mut source_only = false;
    for property in properties {
        match property.kind {
            20 => tattoo = super::be_u32(property.data).filter(|n| *n != 0),
            43 => selected = true,
            _ => source_only = true,
        }
    }
    if p.u32()? != 1 {
        return Err(invalid("unsupported standalone path version"));
    }
    let n = count(&mut p, MAX_POINTS, "path stroke")?;
    let mut budget = MAX_POINTS;
    let (path, extra) = strokes(&mut p, n, &mut budget)?;
    if p.pos != raw.len() {
        return Err(invalid("standalone path payload length mismatch"));
    }
    Ok(DecodedPath { named: NamedPath { name, path, psd_raw: None }, tattoo, selected, source_only: source_only || extra })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn u(out: &mut Vec<u8>, v: u32) {
        out.extend(v.to_be_bytes());
    }
    fn string(out: &mut Vec<u8>, s: &str) {
        u(out, s.len() as u32 + 1);
        out.extend(s.as_bytes());
        out.push(0);
    }
    fn stroke(out: &mut Vec<u8>, closed: bool, axes: u32) {
        for v in [1, u32::from(closed), axes, 6] {
            u(out, v);
        }
        for (kind, x, y) in [(1, -4.5f32, 2.0f32), (0, 1.0, 3.0), (1, 6.0, 7.0), (1, 8.0, 9.0), (0, 20.0, -3.0), (1, 25.0, 6.0)] {
            u(out, kind);
            u(out, x.to_bits());
            u(out, y.to_bits());
            for _ in 2..axes {
                u(out, 0.5f32.to_bits());
            }
        }
    }
    #[test]
    fn old_vectors_preserve_independent_native_cubic_geometry() {
        let mut b = Vec::new();
        for v in [1, 0, 1] {
            u(&mut b, v);
        }
        string(&mut b, "曲線");
        for v in [123, 0, 0, 0, 2] {
            u(&mut b, v);
        }
        stroke(&mut b, false, 2);
        stroke(&mut b, true, 6);
        let p = vectors(&b).expect("vectors").remove(0);
        assert_eq!(p.named.name, "曲線");
        assert_eq!(p.tattoo, Some(123));
        assert!(p.selected && p.source_only);
        assert!(!p.named.path.subpaths[0].closed);
        assert!(p.named.path.subpaths[1].closed);
        let k = p.named.path.subpaths[0].knots[0];
        assert_eq!(k.anchor, Point::new(1.0, 3.0));
        assert_eq!(k.in_ctrl, Point::new(-4.5, 2.0));
        assert_eq!(k.out_ctrl, Point::new(6.0, 7.0));
        assert_eq!(p.named.path.control_bounds(), Some((-4.5, -3.0, 25.0, 9.0)));
    }
    #[test]
    fn standalone_is_framed_and_rejects_every_truncation() {
        let mut payload = Vec::new();
        for v in [20, 4, 72, 43, 0, 0, 0, 1, 1] {
            u(&mut payload, v);
        }
        stroke(&mut payload, true, 2);
        let mut b = Vec::new();
        string(&mut b, "path");
        u(&mut b, payload.len() as u32);
        b.extend(payload);
        let c = Cursor { b: &b, pos: 0, wide: false };
        let p = standalone(&c, 0).expect("path");
        assert_eq!(p.tattoo, Some(72));
        assert!(p.selected);
        for end in 0..b.len() {
            assert!(standalone(&Cursor { b: &b[..end], pos: 0, wide: false }, 0).is_err());
        }
        let offset = b.len() - 8;
        b[offset..offset + 4].copy_from_slice(&f32::NAN.to_bits().to_be_bytes());
        assert!(standalone(&Cursor { b: &b, pos: 0, wide: false }, 0).is_err());
    }
    #[test]
    fn legacy_versions_keep_native_geometry_and_native_save_reopen() {
        for version in [1, 2, 3] {
            let mut b = Vec::new();
            u(&mut b, 0);
            u(&mut b, 1);
            string(&mut b, "Legacy");
            u(&mut b, 0);
            b.push(0);
            for value in [1, 6, version] {
                u(&mut b, value);
            }
            if version >= 2 {
                u(&mut b, 0);
            }
            if version == 3 {
                u(&mut b, 99);
            }
            for (kind, x, y) in [(1, 0i32, 0i32), (2, 4, 0), (2, 6, 8), (1, 10, 10), (2, 4, 8), (2, -3, 0)] {
                u(&mut b, kind);
                for value in [x, y] {
                    u(&mut b, if version == 1 { value as u32 } else { (value as f32).to_bits() });
                }
            }
            let path = legacy(&b).expect("legacy").remove(0);
            assert_eq!(path.named.path.subpaths[0].knots.len(), 2);
            assert_eq!(path.named.path.subpaths[0].knots[0].in_ctrl, Point::new(-3.0, 0.0));
            assert_eq!(path.named.path.subpaths[0].knots[1].anchor, Point::new(10.0, 10.0));
            let mut doc =
                photocraft_doc::Document::new("Paths", photocraft_geom::Size::new(20, 20), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
            doc.paths.push(path.named.clone());
            let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).expect("save");
            let reopened = photocraft_format::load_from_bytes(&bytes).expect("reopen");
            assert_eq!(reopened.paths[0], path.named);
        }
    }
    #[test]
    fn vector_layer_import_resolves_local_path_without_using_source_raster_cache() {
        use super::super::tests::{TestImage, TestLayer};
        let mut path = Vec::new();
        for v in [1, 0, 1] {
            u(&mut path, v);
        }
        string(&mut path, "Outline");
        for v in [72, 0, 0, 0, 1] {
            u(&mut path, v);
        }
        stroke(&mut path, true, 2);
        let mut style = Vec::new();
        for v in [0, 72, 1, 1] {
            u(&mut style, v);
        }
        for _ in 0..2 {
            u(&mut style, 0);
            u(&mut style, 1);
            string(&mut style, "R'G'B'A u8");
            u(&mut style, 4);
            style.extend([0, 0, 0, 255]);
            u(&mut style, 0);
            u(&mut style, 0);
        }
        for v in [2.0f32.to_bits(), 0, 0, 4.0f32.to_bits(), 0] {
            u(&mut style, v);
        }
        for modified in [false, true] {
            let mut image = TestImage::rgb8(32, 24);
            image.image_props.push((25, path.clone()));
            let mut layer = TestLayer::rgba("Shape", 1, 1, vec![200, 20, 100, 255]);
            style[..4].copy_from_slice(&u32::from(modified).to_be_bytes());
            layer.props.push((47, style.clone()));
            image.layers.push(layer);
            let doc = crate::import("shape.xcf", &image.write()).expect("import").document;
            if modified {
                assert!(matches!(doc.layers[0].content, photocraft_doc::LayerContent::Raster(_)));
                continue;
            }
            let photocraft_doc::LayerContent::Shape(shape) = &doc.layers[0].content else { panic!("editable shape") };
            assert!(shape.cache.is_none());
            assert_eq!(shape.path, doc.paths[0].path);
            assert_eq!(shape.path.subpaths[0].knots[1].anchor, Point::new(20.0, -3.0));
            let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).expect("save");
            let reopened = photocraft_format::load_from_bytes(&saved).expect("reopen");
            let photocraft_doc::LayerContent::Shape(reopened_shape) = &reopened.layers[0].content else { panic!("reopenedshape") };
            assert_eq!(shape, reopened_shape);
        }
    }
    #[test]
    fn vector_style_maps_portable_paint_stroke_units_and_modified_flag() {
        let mut b = Vec::new();
        for v in [0, 72, 1, 1] {
            u(&mut b, v);
        }
        for rgba in [[32, 64, 128, 255], [255, 0, 0, 128]] {
            u(&mut b, 0);
            u(&mut b, 1);
            string(&mut b, "R'G'B'A u8");
            u(&mut b, 4);
            b.extend(rgba);
            u(&mut b, 0);
            u(&mut b, 0);
        }
        u(&mut b, 3.5f32.to_bits());
        u(&mut b, 1);
        u(&mut b, 2);
        u(&mut b, 4.0f32.to_bits());
        u(&mut b, 2);
        u(&mut b, 2.0f32.to_bits());
        u(&mut b, 1.0f32.to_bits());
        let style = vector_style(&b).expect("vector style");
        assert_eq!(style.path_tattoo, 72);
        assert!(!style.modified);
        assert!(style.shape.cache.is_none());
        assert!(style.source_only.is_empty());
        assert_eq!(style.shape.fill, Some(Fill::Solid(Color::rgba(32.0 / 255.0, 64.0 / 255.0, 128.0 / 255.0, 1.0))));
        let stroke = style.shape.stroke.expect("stroke");
        assert_eq!(stroke.width, 3.5);
        assert_eq!(stroke.cap, LineCap::Round);
        assert_eq!(stroke.join, LineJoin::Bevel);
        assert_eq!(stroke.dashes, [2.0, 1.0]);
        for end in 0..b.len() {
            assert!(vector_style(&b[..end]).is_err());
        }
        b[..4].copy_from_slice(&1u32.to_be_bytes());
        assert!(vector_style(&b).expect("modified").modified);
    }

    #[test]
    fn portable_profiled_and_gray_vector_paints_use_native_cms() {
        let profile = photocraft_cms::Builtin::DisplayP3.profile().to_bytes();
        let mut b = Vec::new();
        u(&mut b, 0);
        u(&mut b, 1);
        string(&mut b, "R'G'B'A u8");
        u(&mut b, 4);
        b.extend([255, 0, 0, 128]);
        u(&mut b, profile.len() as u32);
        b.extend(profile.iter());
        string(&mut b, "");
        let mut cursor = Cursor { b: &b, pos: 0, wide: false };
        let mut notes = Vec::new();
        let actual = paint(&mut cursor, &mut notes).unwrap();
        let transform = photocraft_cms::Transform::new(
            photocraft_cms::Builtin::DisplayP3.profile(),
            photocraft_cms::Builtin::Srgb.profile(),
            photocraft_cms::Intent::RelativeColorimetric,
            false,
        )
        .unwrap();
        let mut expected = [0.; 3];
        transform.convert_f32(&[1., 0., 0.], 3, &mut expected, 3, false);
        let Fill::Solid(color) = actual else { panic!("native solid paint") };
        for (actual, expected) in color.c.iter().zip(expected) {
            assert!((actual - expected).abs() < 0.00001);
        }
        assert_eq!(color.alpha, 128. / 255.);
        assert!(notes.is_empty());
        let mut gray = Vec::new();
        u(&mut gray, 0);
        u(&mut gray, 1);
        string(&mut gray, "Y'A u8");
        u(&mut gray, 2);
        gray.extend([128, 64]);
        u(&mut gray, 0);
        string(&mut gray, "");
        let Fill::Solid(gray) = paint(&mut Cursor { b: &gray, pos: 0, wide: false }, &mut Vec::new()).unwrap() else { panic!("native gray paint") };
        assert!((gray.c[0] - gray.c[1]).abs() < 0.00001);
        assert_eq!(gray.alpha, 64. / 255.);
    }
}

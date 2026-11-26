//! Map bounded shared source geometry to editable PhotoCraft paths.

use craft_xcf::records::{Budget, paths};
use paths::SourcePath;
use photocraft_doc::Document;
use photocraft_doc::vector::{FillRule, Knot, NamedPath, Path, PathOp, Subpath};
use photocraft_geom::Point;
use photocraft_raster::Interrupt;

use crate::ora::Warnings;

use super::{Result, invalid, map_error};

pub(super) fn import_saved_paths(
    xcf: &craft_xcf::Xcf<'_>,
    source_budget: &mut Budget,
    document: &mut Document,
    warnings: &mut Warnings,
    ctl: &Interrupt<'_>,
) -> Result<Option<usize>> {
    if xcf.image().path_count == 0 {
        return Ok(None);
    }
    let paths = xcf.paths(paths::Limits::default(), source_budget, &|| !ctl.cancelled()).map_err(map_error)?;
    if paths.stopped.is_some() {
        warnings.note("some saved XCF path records could not be read");
    }
    let mut native_budget = craft_xcf::MAX_METADATA_BYTES;
    let mut omitted = 0;
    let mut selected = None;
    for source in &paths.records {
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        match named_path(source, &mut native_budget) {
            Ok((path, partial)) => {
                if paths.stopped.is_none()
                    && source.selected
                    && !source.invalid_selection_marker
                    && !partial
                    && selected.replace(document.paths.len()).is_some()
                {
                    warnings.note("the last selected XCF path is used as the native path target");
                }
                document.paths.push(path);
                omitted += usize::from(partial);
            }
            Err(_) => omitted += 1,
        }
    }
    if omitted != 0 {
        warnings.note(&format!("{omitted} saved XCF path(s) contained strokes or coordinates that could not be mapped"));
    }
    Ok(selected)
}

/// Preserve supported strokes and report whether other geometry was omitted.
pub(super) fn named_path(source: &SourcePath<'_>, budget: &mut usize) -> Result<(NamedPath, bool)> {
    let mut remaining = *budget;
    charge::<Subpath>(&mut remaining, source.strokes.len())?;
    charge::<u8>(&mut remaining, source.name.len())?;
    let mut name = String::new();
    name.try_reserve_exact(source.name.len()).map_err(|_| invalid("not enough memory for an XCF path name"))?;
    name.push_str(source.name);

    let mut subpaths = Vec::new();
    subpaths.try_reserve_exact(source.strokes.len()).map_err(|_| invalid("not enough memory for XCF path strokes"))?;
    let mut omitted = false;
    for stroke in &source.strokes {
        if stroke.kind != 1 || !stroke.knots.iter().all(mapped_coordinates) {
            omitted = true;
            continue;
        }
        let split = stroke.closed && stroke.knots.len() == 1;
        let count = stroke.knots.len().saturating_add(usize::from(split));
        charge::<Knot>(&mut remaining, count)?;
        let mut knots = Vec::new();
        knots.try_reserve_exact(count).map_err(|_| invalid("not enough memory for XCF path knots"))?;
        let point = |value: [f64; 2]| Point::new(value[0], value[1]);
        knots.extend(stroke.knots.iter().map(|knot| Knot {
            anchor: point(knot.anchor.position),
            in_ctrl: point(knot.incoming.position),
            out_ctrl: point(knot.outgoing.position),
            smooth: false,
        }));
        if split {
            split_closed_knot(&mut knots);
        }
        let op = if subpaths.is_empty() { PathOp::Combine } else { PathOp::Join };
        subpaths.push(Subpath { closed: stroke.closed, knots, op });
    }
    let path = Path { subpaths, fill_rule: FillRule::EvenOdd, inverted: false };
    *budget = remaining;
    Ok((NamedPath { name, path, psd_raw: None }, omitted))
}

fn mapped_coordinates(knot: &craft_xcf::records::paths::Knot) -> bool {
    [knot.incoming, knot.anchor, knot.outgoing].iter().flat_map(|point| point.position).all(|coordinate| coordinate.abs() <= 10_000_000.0)
}

fn charge<T>(budget: &mut usize, count: usize) -> Result<()> {
    let bytes = count.checked_mul(std::mem::size_of::<T>()).ok_or_else(|| invalid("XCF native path size overflow"))?;
    *budget = budget.checked_sub(bytes).ok_or_else(|| invalid("the XCF paths exceed the native conversion budget"))?;
    Ok(())
}

/// Native subpaths need two knots to emit a cubic. Bisect a closed loop with a
/// single anchor at half its parameter range to keep its shape.
fn split_closed_knot(knots: &mut Vec<Knot>) {
    let Some(knot) = knots.first_mut() else { return };
    let midpoint = |a: Point, b: Point| Point::new(a.x.midpoint(b.x), a.y.midpoint(b.y));
    let out = midpoint(knot.anchor, knot.out_ctrl);
    let controls = midpoint(knot.out_ctrl, knot.in_ctrl);
    let incoming = midpoint(knot.in_ctrl, knot.anchor);
    let first = midpoint(out, controls);
    let second = midpoint(controls, incoming);
    knot.out_ctrl = out;
    knot.in_ctrl = incoming;
    knots.push(Knot { anchor: midpoint(first, second), in_ctrl: first, out_ctrl: second, smooth: false });
}

#[cfg(test)]
mod tests {
    use craft_xcf::records::paths::{Knot as SourceKnot, Point as SourcePoint, Stroke};

    use super::*;

    #[test]
    fn import_and_native_reopen_keep_a_saved_xcf_path() {
        let words = |out: &mut Vec<u8>, values: &[u32]| {
            for value in values {
                out.extend_from_slice(&value.to_be_bytes());
            }
        };
        let mut file = b"gimp xcf v024\0".to_vec();
        words(&mut file, &[32, 24, 0, 150, 0, 0]);
        file.extend_from_slice(&[0; 16]); // Empty layer and channel offset tables.
        let path_offset = file.len() as u64 + 16;
        file.extend_from_slice(&path_offset.to_be_bytes());
        file.extend_from_slice(&[0; 8]);

        let mut path = Vec::new();
        words(&mut path, &[0, 0, 1, 1, 1, 0, 2, 3]);
        for (kind, x, y) in [(1, -2.0f32, 3.5f32), (0, 4.25, 5.0), (1, 6.0, 7.0)] {
            words(&mut path, &[kind, x.to_bits(), y.to_bits()]);
        }
        words(&mut file, &[6]);
        file.extend_from_slice(b"Curve\0");
        words(&mut file, &[path.len() as u32]);
        file.extend_from_slice(&path);

        let opened = crate::import("source.xcf", &file).unwrap();
        let named = &opened.document.paths[0];
        assert_eq!(named.name, "Curve");
        let knot = &named.path.subpaths[0].knots[0];
        assert_eq!(knot.anchor, Point::new(4.25, 5.0));
        assert_eq!(knot.in_ctrl, Point::new(-2.0, 3.5));
        assert_eq!(knot.out_ctrl, Point::new(6.0, 7.0));
        let saved = photocraft_format::save_to_bytes(&opened.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened.paths[0].path, named.path);
    }

    #[test]
    fn saved_contours_join_into_one_even_odd_path_with_cubic_controls() {
        let knot = SourceKnot {
            incoming: SourcePoint { position: [0.0, 6.0], ..Default::default() },
            anchor: SourcePoint { position: [0.0, 0.0], ..Default::default() },
            outgoing: SourcePoint { position: [8.0, 0.0], ..Default::default() },
        };
        let source = SourcePath {
            source_index: 0,
            offset: 0,
            name: "Saved path",
            tattoo: None,
            selected: false,
            invalid_selection_marker: false,
            visible: 1,
            linked: 0,
            legacy_state: None,
            legacy_version: None,
            properties: Vec::new(),
            parasites: &[],
            strokes: vec![
                Stroke { kind: 1, closed: true, axes: 2, knots: vec![knot], raw: &[] },
                Stroke { kind: 1, closed: false, axes: 2, knots: vec![knot], raw: &[] },
                Stroke { kind: 99, closed: false, axes: 2, knots: Vec::new(), raw: &[] },
            ],
            raw: &[],
        };
        let mut budget = 4096;
        let (named, omitted) = named_path(&source, &mut budget).unwrap();
        assert!(omitted);
        assert_eq!(named.name, "Saved path");
        assert_eq!(named.path.fill_rule, FillRule::EvenOdd);
        let contours = &named.path.subpaths;
        assert_eq!(contours.len(), 2);
        assert_eq!(contours[1].op, PathOp::Join);
        assert_eq!(contours[0].segments().len(), 2);
        assert_eq!(contours[0].knots[0].out_ctrl, Point::new(4.0, 0.0));
        assert_eq!(contours[1].knots[0].in_ctrl, Point::new(0.0, 6.0));
        let mut budget = 1;
        assert!(named_path(&source, &mut budget).is_err());
        assert_eq!(budget, 1);
    }
}

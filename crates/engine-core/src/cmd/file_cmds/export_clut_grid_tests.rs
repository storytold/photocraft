//! #13749: the exported CUBE's grid defaults to 32 points and takes 7 to 256, decimals truncated.
//! Out of range the command refuses rather than writing, and the limit is the export's own: the
//! reader's `MAX_SIZE` still bounds what `image.adjustments.colorLookup` imports.

use super::*;
use serde_json::json;

/// An RGB/8 document with one Invert: the smallest stack the export bakes.
fn session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 8, "depth": 8})).unwrap();
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    s
}

/// The CUBE text an export returns for `params`.
fn cube(s: &mut Session, params: Value) -> String {
    s.execute("file.export.colorLookupTables", params).unwrap()["cube"].as_str().unwrap().to_owned()
}

/// The grid the `LUT_3D_SIZE` line names.
fn header_points(cube: &str) -> u64 {
    cube.lines().find_map(|l| l.trim_start_matches("LUT_3D_SIZE ").trim().parse().ok()).expect("the header names its grid")
}

/// The data rows: the lines that begin with a number.
fn rows(cube: &str) -> usize {
    cube.lines().filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_digit())).count()
}

#[test]
fn the_grid_defaults_to_32_points() {
    let mut s = session();
    let reply = s.execute("file.export.colorLookupTables", json!({})).unwrap();
    assert_eq!(reply["size"], 32, "the reply names the grid it wrote");
    let text = reply["cube"].as_str().unwrap();
    assert_eq!((header_points(text), rows(text)), (32, 32 * 32 * 32), "the default grid");
    // An unset row sends null: the same default as leaving the key out.
    assert_eq!(s.execute("file.export.colorLookupTables", json!({"size": null})).unwrap()["size"], 32);
}

#[test]
fn seven_to_256_take_the_range_and_both_ends_are_inside_it() {
    let mut s = session();
    let text = cube(&mut s, json!({"size": 7}));
    assert_eq!((header_points(&text), rows(&text)), (7, 7 * 7 * 7), "the low end of the range");
    // Past the reader's 129 the export still runs: the grid limit is the export's own constant, not
    // `craft_cms::lutfile::MAX_SIZE`. The 8 x 8 canvas is too small for lattices this large, so the
    // bake's work limit refuses them before any table of that size is built: that is the export
    // staying bounded, not the range refusing the number.
    for size in [130, 256] {
        let err = s.execute("file.export.colorLookupTables", json!({"size": size})).unwrap_err().to_string();
        assert!(err.contains(CANVAS_TOO_SMALL) && !err.contains("size must be"), "{size} points: {err}");
    }
}

/// One real write past the reader's 129: the adjustment-list path has no canvas work limit, so a
/// 130-point request through it produces the whole table — the header names 130 and the body holds
/// 130³ rows. The same grid still will not come back in as an adjustment (see the reader test below).
#[test]
fn a_grid_past_the_reader_limit_writes_through_the_layers_path() {
    let mut s = session();
    let a = s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap()["layer"].as_u64().unwrap();
    let reply = s.execute("file.export.colorLookupTables", json!({"size": 130, "layers": [a]})).unwrap();
    assert_eq!(reply["size"], 130, "the reply names the grid it wrote");
    let text = reply["cube"].as_str().unwrap();
    assert!(text.contains("LUT_3D_SIZE 130"), "the header names 130 points");
    assert_eq!(rows(text), 130 * 130 * 130, "130 cubed rows written");
}

/// The canvas-work limit that refuses a 256-point request on a small canvas is canvas-relative, not
/// a cap on the grid: the same 256 points is 256³ entries sharing 64 pixels there (refused before
/// anything is allocated) and 65536 renders on a 256 × 256 canvas (accepted, and then really 256³
/// rows long). No test runs the second one: its lattice is about 256³ × 16 B and its text about
/// 256³ × 24 characters.
#[test]
fn the_work_limit_that_refuses_256_points_is_canvas_relative() {
    assert!(stack_render_rows(256, 8, 8) > MAX_STACK_RENDERS, "256³ entries over 64 pixels: refused");
    assert!(stack_render_rows(256, 256, 256) <= MAX_STACK_RENDERS, "the same grid over a canvas that holds it: allowed");
}

#[test]
fn a_grid_of_n_points_holds_n_cubed_rows() {
    let mut s = session();
    for size in [7u64, 9, 17, 32] {
        let text = cube(&mut s, json!({"size": size}));
        assert_eq!(header_points(&text), size, "{size}: the header names the grid it wrote");
        assert_eq!(rows(&text), usize::try_from(size * size * size).unwrap(), "{size}: points cubed rows");
    }
}

#[test]
fn a_decimal_truncates_rather_than_rounds() {
    let mut s = session();
    for typed in [9.6, 9.99] {
        let reply = s.execute("file.export.colorLookupTables", json!({"size": typed})).unwrap();
        assert_eq!(reply["size"], 9, "{typed}: truncated, not rounded");
        let text = reply["cube"].as_str().unwrap();
        assert_eq!((header_points(text), rows(text)), (9, 9 * 9 * 9), "{typed}");
    }
    // 6.9 truncates to 6, which the range refuses.
    let err = s.execute("file.export.colorLookupTables", json!({"size": 6.9})).unwrap_err().to_string();
    assert!(err.contains("size must be 7..=256"), "6.9: {err}");
    // Text stays today's refusal.
    let err = s.execute("file.export.colorLookupTables", json!({"size": "abc"})).unwrap_err().to_string();
    assert!(err.contains("size must be an integer"), "\"abc\": {err}");
}

#[test]
fn outside_the_range_refuses_and_leaves_the_document_alone() {
    let mut s = session();
    let (rev, history) = (s.active().unwrap().revision, s.active().unwrap().history.past_len());
    for params in [json!({"size": 6}), json!({"size": 0}), json!({"size": -3}), json!({"size": 257}), json!({"size": 1000})] {
        let err = s.execute("file.export.colorLookupTables", params.clone()).unwrap_err().to_string();
        assert!(err.contains("size must be 7..=256"), "{params}: {err}");
        assert_eq!(s.active().unwrap().revision, rev, "{params}: export stays read-only");
        assert_eq!(s.active().unwrap().history.past_len(), history, "{params}");
    }
}

#[test]
fn the_written_file_holds_the_same_grid_as_the_returned_text() {
    let dir = std::env::temp_dir().join(format!("photocraft-clut-grid-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("grid.cube").to_string_lossy().into_owned();
    let mut s = session();
    let r = s.execute("file.export.colorLookupTables", json!({"size": 7, "path": &path})).unwrap();
    assert_eq!(r["size"], 7);
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!((header_points(&text), rows(&text)), (7, 7 * 7 * 7), "the file holds points cubed rows");
    let parsed = craft_cms::lutfile::parse_cube(&text).unwrap();
    assert_eq!((parsed.size, parsed.data.len()), (7, 7 * 7 * 7 * 3), "and the reader takes it back");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The reader's own limit is untouched: the export's range is not the import's (#13749 keeps the
/// two apart), so a 130-point file still refuses to load as a Color Lookup adjustment.
#[test]
fn a_grid_the_reader_refuses_still_refuses_to_import() {
    let dir = std::env::temp_dir().join(format!("photocraft-clut-grid-read-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("wide.cube").to_string_lossy().into_owned();
    std::fs::write(&path, "LUT_3D_SIZE 130\n0 0 0\n").unwrap();
    let mut s = session();
    s.execute("layer.new.layer", json!({"name": "Pixels"})).unwrap();
    let err = s.execute("image.adjustments.colorLookup", json!({"file": &path})).unwrap_err().to_string();
    assert!(err.contains("LUT_3D_SIZE 130 out of range"), "the reader still names its own limit: {err}");
    assert_eq!(s.active().unwrap().doc.layers.len(), 3, "nothing was added");
    let _ = std::fs::remove_dir_all(&dir);
}

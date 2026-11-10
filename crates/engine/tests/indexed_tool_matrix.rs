//! Adapted from @squintdev's Indexed palette coverage matrix (immutable source commit
//! [`59ca72b1b72dd930f7f43977de3c4817b2a301d0`](https://github.com/squintdev/photocraft/blob/59ca72b1b72dd930f7f43977de3c4817b2a301d0/crates/engine/tests/indexed_tool_matrix.rs).
//!
//! Each operation is exercised against the original two-palette matrix: a strict black/white
//! split and a 40-colour gradient. Indexed layers keep expanded RGB samples plus a fixed
//! [`ColorTable`], so these tests always compare samples with the table captured before the tool
//! runs. A successful operation may legitimately quantize back to its source entry; the RGB
//! control for each such operation must still make a real edit.

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{ColorTable, Document, Size};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::json;

const COLOR_REPLACEMENT_INDEXED: &str =
    "Could not use the Color Replacement tool because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";
const MIXER_INDEXED: &str = "Could not use the Mixer Brush tool because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";
const DODGE_INDEXED: &str = "Could not use the dodge tool because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";

type Paint = fn(i32, i32) -> [f32; 4];
type ToolRun = fn(&mut Session);

#[derive(Clone, Debug, PartialEq, Eq)]
struct RasterSnapshot {
    size: (u32, u32),
    layers: Vec<Vec<u8>>,
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn table(s: &Session) -> ColorTable {
    doc(s).color_table.clone().unwrap()
}

fn history_len(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn rgb8(p: &[f32]) -> [u8; 3] {
    [p[0], p[1], p[2]].map(|v| (v * 255.0).round() as u8)
}

/// Capture every raster layer, not just the active one, so refusal and undo/redo assertions cannot
/// be satisfied by changing a different layer or by dropping a pasted layer.
fn raster_snapshot(s: &Session) -> RasterSnapshot {
    let d = doc(s);
    RasterSnapshot {
        size: (d.size.width, d.size.height),
        layers: d.layers.iter().filter_map(|layer| layer.surface().map(|surface| surface.to_interleaved(d.bounds()))).collect(),
    }
}

/// Every visible raster sample must be one of the entries captured before the operation. The
/// caller deliberately supplies the original table; reading `doc(s).color_table` here would let a
/// broken operation grow a table and then pass by checking itself against the new entries.
fn assert_original_table_samples(s: &Session, original: &ColorTable, label: &str) {
    let d = doc(s);
    let canvas = d.bounds();
    for (layer_index, layer) in d.layers.iter().enumerate() {
        let Some(surface) = layer.surface() else { continue };
        let bounds = surface.content_bounds().intersect(&canvas);
        for y in bounds.y0..bounds.y1 {
            for x in bounds.x0..bounds.x1 {
                let pixel = surface.rgba(x, y);
                if pixel[3] <= 0.0 {
                    continue;
                }
                let colour = rgb8(&pixel);
                assert!(original.colors.contains(&colour), "{label}: layer {layer_index} sample ({x},{y}) {pixel:?} is outside original table {original:?}");
            }
        }
    }
}

fn assert_indexed_state(s: &Session, original_mode: ColorMode, original_depth: SampleType, original_table: &ColorTable, label: &str) {
    let d = doc(s);
    assert_eq!(d.mode, ColorMode::Indexed, "{label}: operation left Indexed Color");
    assert_eq!(d.mode, original_mode, "{label}: operation changed the document mode");
    assert_eq!(d.depth, original_depth, "{label}: operation changed the document depth");
    assert_eq!(d.color_table.as_ref(), Some(original_table), "{label}: operation changed the complete Color Table");
    assert_original_table_samples(s, original_table, label);
}

/// The two matrix inputs remain named in every assertion so a failure identifies its palette.
fn palette_cases() -> [(&'static str, Paint); 2] {
    [("strict black and white", halves), ("40-colour gradient", gradient)]
}

fn halves(x: i32, _: i32) -> [f32; 4] {
    if x < 20 { [0.0, 0.0, 0.0, 1.0] } else { [1.0, 1.0, 1.0, 1.0] }
}

fn gradient(x: i32, _: i32) -> [f32; 4] {
    [x as f32 / 40.0, 0.5, 0.5, 1.0]
}

/// A non-linear edge gives Sharpen/Blur a real RGB signal even when a fixed palette legitimately
/// maps the result back to the same entries.
fn soft_edge(x: i32, _: i32) -> [f32; 4] {
    let red = match x {
        ..=17 => 0.25,
        18 => 0.35,
        19 => 0.45,
        20 => 0.55,
        21 => 0.65,
        _ => 0.75,
    };
    [red, 0.5, 0.5, 1.0]
}

fn rgb_document(paint: Paint) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 20, "depth": 8, "mode": "rgb", "background": "transparent"})).unwrap();
    s.edit("indexed matrix setup", |doc, active| {
        let bounds = doc.bounds();
        let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let format = surface.format();
        let mut data = Vec::with_capacity((bounds.width() * bounds.height()) as usize * format.channels());
        for y in bounds.y0..bounds.y1 {
            for x in bounds.x0..bounds.x1 {
                data.extend(photocraft_raster::from_rgba(&format, paint(x, y)));
            }
        }
        surface.write_region(bounds, &data);
        Ok(())
    })
    .unwrap();
    s
}

fn indexed(paint: Paint) -> Session {
    let mut s = rgb_document(paint);
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none"})).unwrap();
    s.execute("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
    s
}

/// Check a successful operation without demanding that every quantized output differ. The RGB
/// control is intentionally required to change, while the Indexed result is checked against the
/// fixed table and round-tripped through one undo/redo pair.
fn assert_quantized_tool(palette_name: &str, paint: Paint, tool: &str, run: ToolRun, control_paint: Paint, guaranteed_probe: Option<(i32, i32)>) {
    let mut s = indexed(paint);
    let original_mode = doc(&s).mode;
    let original_depth = doc(&s).depth;
    let original_table = table(&s);
    let before = raster_snapshot(&s);
    let before_probe = guaranteed_probe.map(|(x, y)| ((x, y), doc(&s).layers[0].surface().unwrap().rgba(x, y)));
    let steps = history_len(&s);

    run(&mut s);

    let label = format!("{palette_name} / {tool}");
    if let Some(((x, y), before_pixel)) = before_probe {
        assert_ne!(doc(&s).layers[0].surface().unwrap().rgba(x, y), before_pixel, "{label}: guaranteed probe did not change");
    }
    assert_eq!(history_len(&s), steps + 1, "{label}: successful operation did not make one history step");
    assert_indexed_state(&s, original_mode, original_depth, &original_table, &label);
    let after = raster_snapshot(&s);

    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(raster_snapshot(&s), before, "{label}: undo did not restore all raster layers");
    assert_eq!(history_len(&s), steps, "{label}: undo did not restore history");
    assert_eq!(table(&s), original_table, "{label}: undo changed the original Color Table");

    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(raster_snapshot(&s), after, "{label}: redo did not restore all raster layers");
    assert_eq!(history_len(&s), steps + 1, "{label}: redo did not restore history");
    assert_indexed_state(&s, original_mode, original_depth, &original_table, &label);

    let mut control = rgb_document(control_paint);
    let control_before = raster_snapshot(&control);
    run(&mut control);
    assert_ne!(raster_snapshot(&control), control_before, "{label}: RGB control did not make a meaningful edit");
}

fn assert_indexed_refusal(palette_name: &str, paint: Paint, tool: &str, params: serde_json::Value, expected: &str) {
    let mut s = indexed(paint);
    let original_mode = doc(&s).mode;
    let original_depth = doc(&s).depth;
    let original_table = table(&s);
    let before = raster_snapshot(&s);
    let steps = history_len(&s);

    let error = s.execute(tool, params).unwrap_err().to_string();
    let label = format!("{palette_name} / {tool}");
    assert_eq!(error, expected, "{label}: unexpected Indexed refusal");
    assert_eq!(raster_snapshot(&s), before, "{label}: refusal changed samples or layers");
    assert_eq!(history_len(&s), steps, "{label}: refusal added history");
    assert_indexed_state(&s, original_mode, original_depth, &original_table, &label);
}

fn run_gradient(s: &mut Session) {
    s.execute(
        "paint.gradient",
        json!({
            "from": [0, 0],
            "to": [39, 0],
            "colors": ["#ff0000", "#00ff00"],
            "dither": false,
            "opacity": 100
        }),
    )
    .unwrap();
}

fn run_bucket(s: &mut Session) {
    s.execute("paint.bucket", json!({"x": 30, "y": 10, "tolerance": 0, "contiguous": true, "antiAlias": false, "color": "#ff0000"})).unwrap();
}

fn run_blur(s: &mut Session) {
    s.execute("paint.blur", json!({"points": [[20, 10], [30, 10]], "size": 20, "hardness": 0, "strength": 100, "opacity": 50})).unwrap();
}

fn run_sharpen(s: &mut Session) {
    s.execute("paint.sharpen", json!({"points": [[20, 10], [30, 10]], "size": 20, "hardness": 0, "strength": 100, "opacity": 50})).unwrap();
}

fn run_smudge(s: &mut Session) {
    s.execute("paint.smudge", json!({"points": [[20, 10], [30, 10]], "size": 20, "hardness": 0, "strength": 100, "opacity": 50})).unwrap();
}

fn run_burn(s: &mut Session) {
    s.execute(
        "paint.burn",
        json!({
            "points": [[24, 10], [36, 10]],
            "size": 20,
            "hardness": 0,
            "opacity": 50,
            "exposure": 100,
            "range": "highlights",
            "protectTones": false
        }),
    )
    .unwrap();
}

fn run_sponge(s: &mut Session) {
    s.execute("paint.sponge", json!({"points": [[20, 10], [30, 10]], "size": 20, "hardness": 0, "opacity": 50, "mode": "saturate"})).unwrap();
}

#[test]
fn paint_gradient_stays_in_the_original_table() {
    for (palette_name, paint) in palette_cases() {
        assert_quantized_tool(palette_name, paint, "paint.gradient", run_gradient, paint, Some((30, 10)));
    }
}

#[test]
fn paint_bucket_stays_in_the_original_table() {
    for (palette_name, paint) in palette_cases() {
        assert_quantized_tool(palette_name, paint, "paint.bucket", run_bucket, paint, Some((30, 10)));
    }
}

#[test]
fn paint_color_replacement_refuses_indexed_without_editing() {
    for (palette_name, paint) in palette_cases() {
        assert_indexed_refusal(
            palette_name,
            paint,
            "paint.colorReplacement",
            json!({"points": [[4, 10], [36, 10]], "size": 8, "mode": "color", "tolerance": 30}),
            COLOR_REPLACEMENT_INDEXED,
        );
    }
}

#[test]
fn paint_mixer_brush_refuses_indexed_without_editing() {
    for (palette_name, paint) in palette_cases() {
        assert_indexed_refusal(
            palette_name,
            paint,
            "paint.mixerBrush",
            json!({"points": [[4, 10], [36, 10]], "size": 8, "wet": 0, "load": 100, "mix": 0}),
            MIXER_INDEXED,
        );
    }
}

#[test]
fn paint_blur_stays_in_the_original_table() {
    for (palette_name, paint) in palette_cases() {
        assert_quantized_tool(palette_name, paint, "paint.blur", run_blur, soft_edge, None);
    }
}

#[test]
fn paint_sharpen_stays_in_the_original_table() {
    for (palette_name, paint) in palette_cases() {
        // A sharp black/white edge may legitimately snap back to the same two entries. A non-linear
        // RGB edge still proves that this test exercises a real sharpen edit.
        assert_quantized_tool(palette_name, paint, "paint.sharpen", run_sharpen, soft_edge, None);
    }
}

#[test]
fn paint_smudge_stays_in_the_original_table() {
    for (palette_name, paint) in palette_cases() {
        assert_quantized_tool(palette_name, paint, "paint.smudge", run_smudge, paint, None);
    }
}

#[test]
fn paint_dodge_refuses_indexed_without_editing() {
    for (palette_name, paint) in palette_cases() {
        assert_indexed_refusal(
            palette_name,
            paint,
            "paint.dodge",
            json!({"points": [[4, 10], [36, 10]], "size": 8, "hardness": 100, "opacity": 100, "range": "midtones", "exposure": 100}),
            DODGE_INDEXED,
        );
    }
}

#[test]
fn paint_burn_stays_in_the_original_table() {
    for (palette_name, paint) in palette_cases() {
        assert_quantized_tool(palette_name, paint, "paint.burn", run_burn, paint, None);
    }
}

#[test]
fn paint_sponge_stays_in_the_original_table() {
    for (palette_name, paint) in palette_cases() {
        // Saturating black/white is a legitimate quantized no-op; use the coloured RGB control to
        // ensure the matrix entry is not merely accepting an inert command.
        assert_quantized_tool(palette_name, paint, "paint.sponge", run_sponge, gradient, None);
    }
}

fn paste_red_clipboard(s: &mut Session) {
    let mut source = Session::new();
    source.execute("file.new", json!({"width": 10, "height": 10, "background": "#ff0000"})).unwrap();
    source.execute("select.all", json!({})).unwrap();
    source.execute("edit.copy", json!({})).unwrap();
    s.clipboard = source.clipboard.take();
    s.execute("edit.paste", json!({})).unwrap();
}

#[test]
fn edit_paste_stays_in_the_original_table_and_adds_a_layer() {
    for (palette_name, paint) in palette_cases() {
        let mut s = indexed(paint);
        let original_mode = doc(&s).mode;
        let original_depth = doc(&s).depth;
        let original_table = table(&s);
        let before = raster_snapshot(&s);
        let before_layers = doc(&s).layers.len();
        let steps = history_len(&s);

        paste_red_clipboard(&mut s);

        let label = format!("{palette_name} / edit.paste");
        assert_eq!(doc(&s).layers.len(), before_layers + 1, "{label}: paste did not add a layer");
        let active = s.active().unwrap().active_layer.unwrap();
        let pasted = doc(&s).layer(active).unwrap();
        let pasted_surface = pasted.surface().unwrap();
        assert_eq!(pasted_surface.content_bounds(), Rect::new(15, 5, 25, 15), "{label}: unexpected pasted geometry");
        let pasted_pixel = pasted_surface.rgba(20, 10);
        assert!(pasted_pixel[3] > 0.0, "{label}: pasted layer has no visible pixel");
        assert!(original_table.colors.contains(&rgb8(&pasted_pixel)), "{label}: pasted sample escaped original table: {pasted_pixel:?}");
        assert_eq!(history_len(&s), steps + 1, "{label}: paste did not make one history step");
        assert_indexed_state(&s, original_mode, original_depth, &original_table, &label);
        let after = raster_snapshot(&s);

        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(raster_snapshot(&s), before, "{label}: undo did not remove pasted layer");
        assert_eq!(history_len(&s), steps, "{label}: undo did not restore history");
        assert_eq!(table(&s), original_table, "{label}: undo changed the original Color Table");

        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(raster_snapshot(&s), after, "{label}: redo did not restore pasted layer");
        assert_eq!(history_len(&s), steps + 1, "{label}: redo did not restore history");
        assert_indexed_state(&s, original_mode, original_depth, &original_table, &label);
    }
}

#[test]
fn image_rotation_arbitrary_bilinear_stays_in_the_original_table_and_changes_geometry() {
    for (palette_name, paint) in palette_cases() {
        let mut s = indexed(paint);
        let original_mode = doc(&s).mode;
        let original_depth = doc(&s).depth;
        let original_table = table(&s);
        let before = raster_snapshot(&s);
        let steps = history_len(&s);

        s.execute("image.rotation.arbitrary", json!({"angle": 30, "interpolation": "bilinear"})).unwrap();

        let label = format!("{palette_name} / image.rotation.arbitrary bilinear");
        assert_eq!(doc(&s).size, Size::new(45, 38), "{label}: rotation did not change the canvas geometry as expected");
        assert!(
            doc(&s).layers.iter().filter_map(|layer| layer.surface()).any(|surface| !surface.content_bounds().is_empty()),
            "{label}: rotation removed all visible layer content"
        );
        assert_eq!(history_len(&s), steps + 1, "{label}: rotation did not make one history step");
        assert_indexed_state(&s, original_mode, original_depth, &original_table, &label);
        let after = raster_snapshot(&s);

        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(raster_snapshot(&s), before, "{label}: undo did not restore pre-rotation geometry and pixels");
        assert_eq!(history_len(&s), steps, "{label}: undo did not restore history");
        assert_eq!(table(&s), original_table, "{label}: undo changed the original Color Table");

        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(raster_snapshot(&s), after, "{label}: redo did not restore rotated geometry and pixels");
        assert_eq!(history_len(&s), steps + 1, "{label}: redo did not restore history");
        assert_indexed_state(&s, original_mode, original_depth, &original_table, &label);
    }
}

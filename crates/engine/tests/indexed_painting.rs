//! Consumer-visible Indexed Color painting regressions.
//!
//! Indexed layers retain expanded RGB samples plus a fixed [`ColorTable`].  These tests use the
//! command surface rather than private implementation helpers: every paint is a real Session
//! transaction, and assertions inspect the document's native samples and palette sidecar.

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{ColorTable, Document};
use photocraft_engine::Session;
use serde_json::json;

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn surface(s: &Session) -> &photocraft_raster::Surface {
    let state = s.active().unwrap();
    state.doc.layer(state.active_layer.unwrap()).unwrap().surface().unwrap()
}

fn native(s: &Session, x: i32, y: i32) -> Vec<f32> {
    surface(s).pixel(x, y)
}

fn rgba(s: &Session, x: i32, y: i32) -> [f32; 4] {
    surface(s).rgba(x, y)
}

fn table(s: &Session) -> ColorTable {
    doc(s).color_table.clone().unwrap()
}

fn pixels(s: &Session) -> Vec<u8> {
    let state = s.active().unwrap();
    state.doc.layer(state.active_layer.unwrap()).unwrap().surface().unwrap().to_interleaved(state.doc.bounds())
}

/// An opaque black/white document converted through Image › Mode › Indexed Color.
fn indexed_black_white(width: u32, height: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": width, "height": height, "background": "transparent"})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#000000"})).unwrap();
    s.execute("select.rect", json!({"x": (width / 2) as i32, "y": 0, "width": width / 2, "height": height})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ffffff"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "colors": 2, "forced": "none", "dither": "none"})).unwrap();

    assert_eq!(doc(&s).mode, ColorMode::Indexed);
    assert_eq!(doc(&s).depth, SampleType::U8);
    let t = table(&s);
    assert_eq!(t.colors, vec![[0, 0, 0], [255, 255, 255]]);
    assert_eq!(t.transparent, None);
    s
}

/// An Indexed document with a deliberately misleading transparent RGB entry.  The generated
/// black/white palette has two opaque entries and a reserved duplicate slot; Color Table then
/// changes only that reserved slot to red through the public command.
fn indexed_red_transparent_black(width: u32, height: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": width, "height": height, "background": "transparent"})).unwrap();
    s.execute("select.rect", json!({"x": width / 3, "y": 4, "width": width / 3, "height": height - 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#000000"})).unwrap();
    s.execute("select.rect", json!({"x": 2 * width / 3, "y": 4, "width": width / 3, "height": height - 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ffffff"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "colors": 2, "forced": "none", "dither": "none"})).unwrap();
    let generated = table(&s);
    assert_eq!(generated.colors.len(), 3);
    assert_eq!(&generated.colors[..2], &[[0, 0, 0], [255, 255, 255]]);
    assert_eq!(generated.transparent, Some(2));

    s.execute("image.mode.colorTable", json!({"entries": {"2": "#ff0000"}})).unwrap();
    assert_eq!(doc(&s).mode, ColorMode::Indexed);
    assert_eq!(table(&s), ColorTable { colors: vec![[0, 0, 0], [255, 255, 255], [255, 0, 0]], transparent: Some(2) });
    s
}

/// A Background layer with both entries present, used to exercise the transparency lock while
/// the document is Indexed and has no transparent palette entry.
fn indexed_background_black_white(width: u32, height: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": width, "height": height, "background": "#ffffff"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": width / 2, "height": height})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#000000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "colors": 2, "forced": "none", "dither": "none"})).unwrap();
    assert_eq!(table(&s).transparent, None);
    s
}

fn rgb8(p: &[f32]) -> [u8; 3] {
    [(p[0] * 255.0).round() as u8, (p[1] * 255.0).round() as u8, (p[2] * 255.0).round() as u8]
}

fn assert_indexed_palette_samples(s: &Session, t: &ColorTable) {
    let state = s.active().unwrap();
    let b = state.doc.bounds();
    let surf = state.doc.layer(state.active_layer.unwrap()).unwrap().surface().unwrap();
    for y in b.y0..b.y1 {
        for x in b.x0..b.x1 {
            let p = surf.pixel(x, y);
            assert_eq!(p[3], 1.0, "painted Indexed samples must remain opaque at ({x},{y}): {p:?}");
            assert!(t.colors.contains(&rgb8(&p)), "pixel outside the table at ({x},{y}): {p:?}, table {t:?}");
        }
    }
}

#[test]
fn indexed_brush_and_pencil_snap_unavailable_red_to_black_without_growing_the_table() {
    for command in ["paint.stroke", "paint.pencil"] {
        let mut s = indexed_black_white(24, 12);
        let before_table = table(&s);
        let before_white = native(&s, 18, 6);
        s.execute("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
        let params = if command == "paint.stroke" {
            json!({
                "points": [[4, 6]],
                "size": 5,
                "hardness": 1.0,
                "opacity": 1.0,
                "flow": 1.0
            })
        } else {
            json!({"points": [[4, 6]], "size": 3})
        };
        s.execute(command, params).unwrap();

        assert_eq!(doc(&s).mode, ColorMode::Indexed, "{command} changed the document mode");
        assert_eq!(doc(&s).depth, SampleType::U8, "{command} changed the document depth");
        assert_eq!(table(&s), before_table, "{command} changed the Color Table");
        assert_eq!(native(&s, 4, 6), vec![0.0, 0.0, 0.0, 1.0], "{command} left red outside the palette");
        assert_eq!(native(&s, 18, 6), before_white, "{command} changed an untouched white pixel");
    }
}

#[test]
fn indexed_opacity_soft_edges_and_feathered_selection_quantize_after_blending() {
    // Red is nearest black as an opaque foreground, but 50 % red over white is nearer white.
    // Snapping the foreground before compositing would therefore produce the wrong entry.
    let mut opacity = indexed_black_white(32, 16);
    let fixed = table(&opacity);
    opacity
        .execute(
            "paint.stroke",
            json!({
                "points": [[24, 8]],
                "size": 5,
                "hardness": 1.0,
                "opacity": 0.5,
                "flow": 1.0,
                "color": "#ff0000"
            }),
        )
        .unwrap();
    assert_eq!(native(&opacity, 24, 8), vec![1.0, 1.0, 1.0, 1.0], "opacity must blend before palette lookup");
    assert_eq!(table(&opacity), fixed);

    // A soft brush must leave only representable entries, including its partially covered edge.
    let mut soft = indexed_black_white(32, 16);
    let soft_table = table(&soft);
    let untouched = native(&soft, 31, 0);
    assert_eq!(untouched, vec![1.0, 1.0, 1.0, 1.0]);
    soft.execute(
        "paint.stroke",
        json!({
            "points": [[24, 8]],
            "size": 10,
            "hardness": 0.5,
            "opacity": 1.0,
            "flow": 1.0,
            "color": "#ff0000"
        }),
    )
    .unwrap();
    assert_eq!(native(&soft, 24, 8), vec![0.0, 0.0, 0.0, 1.0], "the fully covered soft-brush centre");
    assert_eq!(native(&soft, 31, 0), untouched, "the untouched white side of the soft brush");
    assert_indexed_palette_samples(&soft, &soft_table);
    assert_eq!(table(&soft), soft_table);

    // Feathered selection coverage is also blended first.  The interior must become black, while
    // the feather leaves at least some of the white source visible; every result remains in-table.
    let mut feathered = indexed_black_white(32, 16);
    let feather_table = table(&feathered);
    feathered.execute("select.rect", json!({"x": 20, "y": 3, "width": 8, "height": 10, "feather": 2.0})).unwrap();
    feathered.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    let mut black = 0;
    let mut white = 0;
    for y in 1..15 {
        for x in 18..30 {
            let p = native(&feathered, x, y);
            let colour = rgb8(&p);
            assert!(feather_table.colors.contains(&colour), "feathered result outside table at ({x},{y}): {p:?}");
            assert!(colour == [0, 0, 0] || colour == [255, 255, 255], "unexpected indexed colour {colour:?}");
            if colour == [0, 0, 0] {
                black += 1;
            } else {
                white += 1;
            }
            assert_eq!(p[3], 1.0, "opaque Indexed source became transparent at ({x},{y})");
        }
    }
    assert!(black > 0, "feathered interior was not painted");
    assert!(white > 0, "feathered edge did not preserve any white source");
    assert_eq!(table(&feathered), feather_table);
}

#[test]
fn indexed_undo_and_redo_restore_pixels_and_the_fixed_palette() {
    let mut s = indexed_black_white(24, 12);
    let before_pixels = pixels(&s);
    let before_table = table(&s);
    s.execute("paint.stroke", json!({"points": [[18, 6]], "size": 5, "hardness": 1.0, "color": "#ff0000"})).unwrap();
    let painted_pixels = pixels(&s);
    assert_ne!(painted_pixels, before_pixels);
    assert_eq!(table(&s), before_table);

    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(pixels(&s), before_pixels, "undo did not restore the native Indexed samples");
    assert_eq!(table(&s), before_table, "undo changed the Color Table");

    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(pixels(&s), painted_pixels, "redo did not restore the quantized samples");
    assert_eq!(table(&s), before_table, "redo changed the Color Table");
}

#[test]
fn transparent_entry_is_excluded_from_opaque_matching_and_erase_is_binary() {
    let mut s = indexed_red_transparent_black(24, 16);
    let fixed = table(&s);
    let untouched_clear = rgba(&s, 0, 0);
    assert_eq!(native(&s, 12, 8), vec![0.0, 0.0, 0.0, 1.0]);

    // The transparent red entry is the closest RGB colour to this red foreground, but an opaque
    // edit must choose the only opaque entry instead.
    s.execute("paint.pencil", json!({"points": [[12, 8]], "size": 3, "color": "#ff0000"})).unwrap();
    assert_eq!(native(&s, 12, 8), vec![0.0, 0.0, 0.0, 1.0]);
    assert_eq!(table(&s), fixed);

    // Erasing the same pixel is representable by the reserved entry, with binary alpha and its
    // table RGB (rather than a newly invented transparent colour).
    s.execute("paint.stroke", json!({"points": [[12, 8]], "size": 4, "hardness": 1.0, "erase": true})).unwrap();
    assert_eq!(native(&s, 12, 8), vec![1.0, 0.0, 0.0, 0.0]);
    assert_eq!(rgba(&s, 0, 0), untouched_clear, "painting must not rewrite untouched transparent pixels");
    assert_eq!(table(&s), fixed);
}

#[test]
fn transparency_locked_background_erasure_stays_opaque_and_uses_the_existing_table() {
    let mut s = indexed_background_black_white(24, 12);
    let fixed = table(&s);
    s.execute("tools.setColors", json!({"background": "#ff0000"})).unwrap();
    s.execute("paint.stroke", json!({"points": [[20, 6]], "size": 5, "hardness": 1.0, "erase": true})).unwrap();

    let p = native(&s, 20, 6);
    assert_eq!(p[3], 1.0, "transparency-locked Background erasure must remain opaque");
    assert_eq!(rgb8(&p), [0, 0, 0], "background red must be constrained to the existing palette");
    assert_eq!(table(&s), fixed);
}

#[test]
fn indexed_bucket_edits_are_constrained_by_the_same_transaction_policy() {
    let mut s = indexed_black_white(32, 16);
    let fixed = table(&s);
    s.execute("paint.bucket", json!({"x": 24, "y": 8, "tolerance": 0, "antiAlias": false, "color": "#ff0000"})).unwrap();

    assert_eq!(native(&s, 24, 8), vec![0.0, 0.0, 0.0, 1.0]);
    assert_eq!(native(&s, 4, 8), vec![0.0, 0.0, 0.0, 1.0]);
    assert_eq!(table(&s), fixed);
}

#[test]
fn indexed_new_layer_keeps_sparse_transparency_and_snaps_its_paint() {
    let mut s = indexed_red_transparent_black(24, 16);
    let fixed = table(&s);
    s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
    let sparse_before = native(&s, 1, 1);
    assert_eq!(sparse_before[3], 0.0);
    s.execute("paint.stroke", json!({"points": [[12, 8]], "size": 3, "hardness": 1.0, "color": "#ff0000"})).unwrap();
    assert_eq!(native(&s, 12, 8), vec![0.0, 0.0, 0.0, 1.0]);
    assert_eq!(native(&s, 1, 1), sparse_before, "the untouched sparse default changed");
    assert_eq!(table(&s), fixed);
}

#[test]
fn masks_alpha_channels_and_quick_mask_keep_grayscale_coverage_outside_the_image_palette() {
    let mut s = indexed_black_white(24, 16);
    let fixed = table(&s);
    let image_before = pixels(&s);

    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "black", "opacity": 50, "target": "mask"})).unwrap();
    let state = s.active().unwrap();
    let mask_value = state.doc.layer(state.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.sample_channel(4, 4, 0);
    assert!((mask_value - 0.5).abs() < 0.01, "layer mask coverage was quantized: {mask_value}");
    assert_eq!(pixels(&s), image_before, "editing a layer mask changed Indexed image pixels");
    assert_eq!(table(&s), fixed);

    s.execute("channel.new", json!({"fill": "white", "name": "Alpha"})).unwrap();
    s.execute("edit.fill", json!({"contents": "black", "opacity": 50})).unwrap();
    let alpha_value = doc(&s).channels[0].surface.sample_channel(4, 4, 0);
    assert!((alpha_value - 0.5).abs() < 0.01, "alpha-channel coverage was quantized: {alpha_value}");
    assert_eq!(pixels(&s), image_before, "editing an alpha channel changed Indexed image pixels");
    assert_eq!(table(&s), fixed);

    s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
    s.execute("edit.fill", json!({"contents": "black", "opacity": 50})).unwrap();
    let quick_value = doc(&s).quick_mask.as_ref().unwrap().surface.sample_channel(4, 4, 0);
    assert!((quick_value - 0.5).abs() < 0.01, "Quick Mask coverage was quantized: {quick_value}");
    assert_eq!(pixels(&s), image_before, "editing Quick Mask changed Indexed image pixels");
    assert_eq!(table(&s), fixed);
}

#[test]
fn rgb_mode_paint_remains_rgb_and_does_not_create_a_color_table() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 12, "mode": "rgb", "background": "transparent"})).unwrap();
    s.execute("paint.stroke", json!({"points": [[8, 6]], "size": 4, "hardness": 1.0, "color": "#ff0000"})).unwrap();

    assert_eq!(doc(&s).mode, ColorMode::Rgb);
    assert_eq!(doc(&s).color_table, None);
    assert_eq!(rgba(&s, 8, 6), [1.0, 0.0, 0.0, 1.0]);
}

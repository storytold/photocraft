//! #15291: which area File > Export As exports for a document with artboards, and which area
//! Layer > Export As on an artboard exports. The report: three artboards, two of them deleted,
//! then File > Export As (JPG) gives a picture with a white block where the deleted boards were;
//! the artboard layer's own Export As is right.
//!
//! These tests pin the two paths' bounds as upstream/main computes them:
//! - the canvas grows to fit every board (`artboard_cmds::fit_canvas`, seeded with the canvas's
//!   own extent, so it only ever grows) and `layer.delete` never touches it;
//! - File > Export As takes the whole document canvas (`export_dialog::open`, `export_document`);
//! - Layer > Export As on an artboard takes the layer's own drawn bounds (`layer_menu_cmds::layer_document`);
//! - for a format without transparency the empty canvas is flattened over white
//!   (`export_document`'s `layer.flattenImage` with `flatten_to_surface(.., Some([1.0, 1.0, 1.0]))`).

use super::*;

fn export(doc: &Document, f: &Map<String, Value>) -> Document {
    super::export_document(doc, f, None, &photocraft_engine::color_cmds::document_profile(doc)).unwrap()
}

fn fields(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect()
}

/// The document's canvas composited, to read one pixel of what an export would write.
fn at(doc: &Document, x: i32, y: i32) -> [f32; 4] {
    photocraft_compose::flatten(doc).get(x, y)
}

/// A transparent 300 x 200 document and three white boards: the canvas ends up 350 x 200.
fn three_boards(app: &mut PhotocraftApp) -> Vec<photocraft_doc::LayerId> {
    app.run("file.new", json!({"width": 300, "height": 200, "background": "transparent"})).unwrap();
    let mut ids = Vec::new();
    for rect in [[0, 0, 100, 100], [150, 0, 100, 100], [250, 50, 100, 100]] {
        let id = app.run("layer.new.artboard", json!({"rect": rect, "background": "white"})).unwrap()["layer"].as_u64().unwrap();
        ids.push(photocraft_doc::LayerId(id));
    }
    ids
}

/// The three boards grew the canvas to 350 x 200; deleting two of them leaves the canvas where
/// it grew, with one board left on it.
#[test]
fn deleting_two_of_three_boards_keeps_the_canvas_the_three_grew() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ids = three_boards(&mut app);
    assert_eq!((app.session.active().unwrap().doc.size.width, app.session.active().unwrap().doc.size.height), (350, 200), "grown for the boards");
    for id in [&ids[1], &ids[2]] {
        app.run("layer.delete", json!({"layer": id.0})).unwrap();
    }
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.artboards().len(), 1, "the other two boards are gone");
    assert_eq!((st.doc.size.width, st.doc.size.height), (350, 200), "the canvas keeps the grown size");
    assert_eq!(st.doc.artboards()[0].2.rect, photocraft_geom::Rect::new(0, 0, 100, 100), "the board left on it");
}

/// File > Export As sizes its dialog fields and its export on that canvas: after the deletion it
/// offers 350 x 200, the grown canvas, and not the remaining board's 100 x 100.
#[test]
fn file_export_as_after_deleting_boards_exports_the_canvas() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ids = three_boards(&mut app);
    for id in [&ids[1], &ids[2]] {
        app.run("layer.delete", json!({"layer": id.0})).unwrap();
    }
    let id = open(&mut app).unwrap();
    let f = app.ui.dialog_mut(id).unwrap().fields.clone();
    assert_eq!((f["__w"].as_f64(), f["__h"].as_f64()), (Some(350.0), Some(200.0)), "the dialog's source size");
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(350.0), Some(200.0)), "the size it exports at");
    let png = export(&app.session.active().unwrap().doc, &fields(&[("format", json!("png")), ("transparency", json!(true)), ("scale", json!(100))]));
    assert_eq!((png.size.width, png.size.height), (350, 200), "the exported area is the canvas");
}

/// The residue itself: where the deleted boards were the canvas is empty, so PNG leaves it
/// transparent while JPG, which has no transparency, flattens it over white.
#[test]
fn the_empty_canvas_where_a_board_was_is_transparent_in_png_and_white_in_jpg() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ids = three_boards(&mut app);
    for id in [&ids[1], &ids[2]] {
        app.run("layer.delete", json!({"layer": id.0})).unwrap();
    }
    let doc = app.session.active().unwrap().doc.clone();
    let png = export(&doc, &fields(&[("format", json!("png")), ("transparency", json!(true)), ("scale", json!(100))]));
    let jpg = export(&doc, &fields(&[("format", json!("jpg")), ("scale", json!(100))]));
    // (200, 50) is inside the deleted second board's area; (50, 50) is inside the board left.
    assert_eq!(at(&png, 200, 50), [0.0, 0.0, 0.0, 0.0], "PNG: the empty canvas stays empty");
    assert_eq!(at(&png, 50, 50), [1.0, 1.0, 1.0, 1.0], "PNG: the remaining board paints its white background");
    assert_eq!(at(&jpg, 200, 50), [1.0, 1.0, 1.0, 1.0], "JPG: the empty canvas is flattened over white");
    assert_eq!(at(&jpg, 50, 50), [1.0, 1.0, 1.0, 1.0], "JPG: the board, the same white");
}

/// Neighbour, no artboards at all: the document canvas is the export area, as it has always been.
#[test]
fn a_document_without_artboards_still_exports_its_canvas() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 200, "background": "transparent"})).unwrap();
    let id = open(&mut app).unwrap();
    let f = app.ui.dialog_mut(id).unwrap().fields.clone();
    assert_eq!((f["__w"].as_f64(), f["__h"].as_f64()), (Some(300.0), Some(200.0)));
    let png = export(&app.session.active().unwrap().doc, &fields(&[("format", json!("png")), ("transparency", json!(true)), ("scale", json!(100))]));
    assert_eq!((png.size.width, png.size.height), (300, 200));
}

/// Neighbour, every board still there: the export covers the whole canvas, including the gap
/// between the boards - not their union and not one file per board.
#[test]
fn export_as_with_all_boards_covers_the_canvas_between_them() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    three_boards(&mut app);
    let png = export(&app.session.active().unwrap().doc, &fields(&[("format", json!("png")), ("transparency", json!(true)), ("scale", json!(100))]));
    assert_eq!((png.size.width, png.size.height), (350, 200), "the canvas, not the boards' 300 x 150 union");
    assert_eq!(at(&png, 125, 125), [0.0, 0.0, 0.0, 0.0], "the gap between the boards travels with it");
}

/// Neighbour, the path the report says works: Layer > Export As on the artboard takes the board's
/// own bounds, not the canvas.
#[test]
fn artboard_layer_export_as_opens_with_the_board_size() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ids = three_boards(&mut app);
    for id in [&ids[1], &ids[2]] {
        app.run("layer.delete", json!({"layer": id.0})).unwrap();
    }
    let id = open_layer(&mut app, ids[0]).unwrap();
    let f = app.ui.dialog_mut(id).unwrap().fields.clone();
    assert_eq!((f["__w"].as_f64(), f["__h"].as_f64()), (Some(100.0), Some(100.0)), "the board, not the 350 x 200 canvas");
}

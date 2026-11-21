//! #15291: which area File > Export As exports for a document with artboards. The report: three
//! artboards, two of them deleted, then File > Export As (JPG) gives a picture with a white block
//! where the deleted boards were; the artboard layer's own Export As is right.
//!
//! The cause is two rules meeting: the canvas grows to fit every board (`artboard_cmds::fit_canvas`
//! is seeded with the canvas's own extent, so it only ever grows) and `layer.delete` never shrinks
//! it again; File > Export As then used that canvas (`export_dialog::open` seeded the dialog from
//! `doc.size`), so the empty ground where a deleted board was travels into the file - flattened
//! over white when the format has no alpha. These tests pin the rule measured by the report:
//! - on a document with artboards, File > Export As exports the boards' union, clipped to the
//!   canvas: the empty ground outside it never enters the file, and the gap between the boards
//!   stays inside it (a rect union, not a trim to painted pixels);
//! - a document without artboards, and one whose boards already cover the canvas, keep the canvas
//!   exactly as today (guard tests: they pass before and after);
//! - the boards define the exported area, so real content that lies outside every board is not in
//!   the file (same rule as exporting artboards only), while content that only crosses into the
//!   area still travels; a hidden board counts toward the area as much as a visible one;
//! - Scale All exports every row from that area, not from the grown canvas;
//! - the engine's canvas rule (#14565 owns any change there) and the Layer > Export As path
//!   (#11479 and #13736 own picking a board or one file per board) are untouched.

use super::*;

use craft::{
    host::{Picked, ScriptedFiles},
    ui::patterns::form_dialogs::ScaleSize,
};
use egui_kittest::Harness;

type AppHarness = Harness<'static, Box<PhotocraftApp>>;

/// The documents the mocked exporter was asked to encode, in order.
type Sent = std::rc::Rc<std::cell::RefCell<Vec<Document>>>;

/// An app whose exporter and writer accept everything and record what the exporter is handed, with
/// the file dialogs held open so a test can answer them itself (Scale All asks for a folder).
fn artboard_app_with_picks(width: u32, height: u32) -> (Box<PhotocraftApp>, Sent, Arc<ScriptedFiles>) {
    let sent: Sent = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = sent.clone();
    let files = Arc::new(ScriptedFiles::held());
    let services = crate::Services {
        files: Some(files.clone()),
        export: Some(Box::new(move |doc, _, _| {
            log.borrow_mut().push(doc.clone());
            Ok((Vec::new(), Vec::new()))
        })),
        write: Some(Box::new(|_, _| Ok(()))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": width, "height": height, "background": "transparent"})).unwrap();
    (app, sent, files)
}

/// As [`artboard_app_with_picks`], without the handle on its file dialogs.
fn artboard_app(width: u32, height: u32) -> (Box<PhotocraftApp>, Sent) {
    let (app, sent, _files) = artboard_app_with_picks(width, height);
    (app, sent)
}

/// The dialog host and its answers, as a frame runs them, on the app's own context.
fn harness(app: Box<PhotocraftApp>) -> AppHarness {
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .with_step_dt(1.0 / 60.0)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    h.run_steps(2);
    h
}

/// Opens File > Export As and runs the dialog's first frames.
fn open_and_settle(h: &mut AppHarness) -> DialogId {
    let id = open(h.state_mut()).unwrap();
    h.run_steps(3);
    id
}

/// The settings the open Export As dialog holds.
fn dialog_options(h: &AppHarness) -> ExportOptions {
    h.state().framework.dialogs.get::<HostedExportAs>().expect("Export As is open").options().clone()
}

/// The output size the dialog's settings give for the source it was opened for.
fn output_of(h: &AppHarness, options: &ExportOptions) -> (Option<f64>, Option<f64>) {
    let f = settings_from(options, h.state().export_open.as_ref().expect("Export As is open").source);
    (f["width"].as_f64(), f["height"].as_f64())
}

/// Sets the dialog's fields to `options` and a path, presses OK through the host, and runs frames.
fn export_with(h: &mut AppHarness, id: DialogId, options: &ExportOptions) {
    let ctx = h.ctx.clone();
    let Value::Object(fields) = serde_json::to_value(options).unwrap() else { panic!("options are an object") };
    for (field, value) in fields {
        h.state_mut().framework.dialogs.set_json_field(&ctx, id, &field, value).unwrap();
    }
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, "path", json!("/out/x")).unwrap();
    crate::dialogs::confirm(h.state_mut(), &ctx, id).expect("OK is enabled");
    h.run_steps(3);
}

/// The last document the exporter was asked to encode; the preview/estimate encodes come earlier.
fn exported(sent: &Sent) -> Document {
    sent.borrow().last().cloned().expect("the exporter was asked")
}

/// One pixel of a composited export, to read what the file writes there.
fn at(doc: &Document, x: i32, y: i32) -> [f32; 4] {
    photocraft_compose::flatten(doc).get(x, y)
}

/// Three white boards on a transparent 300 x 200 document: the canvas grows to 350 x 200, the
/// boards' own union is 350 x 150, and the gap around (125, 125) belongs to no board.
fn three_boards(app: &mut PhotocraftApp) -> Vec<photocraft_doc::LayerId> {
    let mut ids = Vec::new();
    for rect in [[0, 0, 100, 100], [150, 0, 100, 100], [250, 50, 100, 100]] {
        let id = app.run("layer.new.artboard", json!({"rect": rect, "background": "white"})).unwrap()["layer"].as_u64().unwrap();
        ids.push(photocraft_doc::LayerId(id));
    }
    ids
}

fn delete_two(app: &mut PhotocraftApp, ids: &[photocraft_doc::LayerId]) {
    for id in [&ids[1], &ids[2]] {
        app.run("layer.delete", json!({"layer": id.0})).unwrap();
    }
}

fn png_options() -> ExportOptions {
    let mut o = ExportOptions::default();
    o.format = "png".into();
    o.transparency = true;
    o
}

fn jpg_options() -> ExportOptions {
    let mut o = ExportOptions::default();
    o.format = "jpg".into();
    o.transparency = false;
    o
}

/// The engine rule this fix does NOT touch: the canvas keeps the size the three boards grew it
/// to when two of them are deleted (#14565 owns any change to that). Guard test: passes before
/// and after this fix.
#[test]
fn deleting_two_of_three_boards_keeps_the_canvas_the_three_grew() {
    let (mut app, _sent) = artboard_app(300, 200);
    let ids = three_boards(app.as_mut());
    assert_eq!((app.session.active().unwrap().doc.size.width, app.session.active().unwrap().doc.size.height), (350, 200), "grown for the boards");
    delete_two(app.as_mut(), &ids);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.artboards().len(), 1, "the other two boards are gone");
    assert_eq!((st.doc.size.width, st.doc.size.height), (350, 200), "the canvas keeps the grown size");
    assert_eq!(st.doc.artboards()[0].2.rect, photocraft_geom::Rect::new(0, 0, 100, 100), "the board left on it");
}

/// Red before the fix: after the deletion the dialog opens on the remaining board's 100 x 100,
/// not the 350 x 200 the three boards grew the canvas to.
#[test]
fn file_export_as_after_deleting_boards_offers_the_remaining_board() {
    let (mut app, _sent) = artboard_app(300, 200);
    let ids = three_boards(app.as_mut());
    delete_two(app.as_mut(), &ids);
    let mut h = harness(app);
    open_and_settle(&mut h);
    let o = dialog_options(&h);
    assert_eq!(h.state().export_open.as_ref().unwrap().source, PixelSize::new(100, 100), "the board's area, not the grown canvas");
    assert_eq!(output_of(&h, &o), (Some(100.0), Some(100.0)), "the size it exports at");
}

/// Red before the fix: the residue itself. The exported PNG and JPG are exactly the remaining
/// board, so the empty ground where a deleted board was never enters the file - nothing to leave
/// transparent (PNG) or to flatten over white (JPG).
#[test]
fn the_exported_file_holds_no_empty_ground_where_a_board_was() {
    let (mut app, sent) = artboard_app(300, 200);
    let ids = three_boards(app.as_mut());
    delete_two(app.as_mut(), &ids);
    let mut h = harness(app);
    let id = open_and_settle(&mut h);
    sent.borrow_mut().clear();
    export_with(&mut h, id, &png_options());
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
    let png = exported(&sent);
    assert_eq!((png.size.width, png.size.height), (100, 100), "the board, not the 350 x 200 canvas: no ground to stay empty");
    assert_eq!(at(&png, 50, 50), [1.0, 1.0, 1.0, 1.0], "the board paints its white background");

    let id = open_and_settle(&mut h);
    sent.borrow_mut().clear();
    export_with(&mut h, id, &jpg_options());
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
    let jpg = exported(&sent);
    assert_eq!((jpg.size.width, jpg.size.height), (100, 100), "JPG: the same board area, no white block");
    assert_eq!(at(&jpg, 99, 99), [1.0, 1.0, 1.0, 1.0], "the board reaches the file's corner");
}

/// Red before the fix, and not the deletion case alone: the rule is the boards' union for any
/// artboard document. With all three boards the export is their 350 x 150 union, not the
/// 350 x 200 canvas, and the gap between the boards (which belongs to no board) still travels
/// inside it: a rect union, not a trim to painted pixels.
#[test]
fn file_export_as_with_all_boards_uses_the_boards_union_not_the_canvas() {
    let (mut app, sent) = artboard_app(300, 200);
    three_boards(app.as_mut());
    let mut h = harness(app);
    let id = open_and_settle(&mut h);
    assert_eq!(h.state().export_open.as_ref().unwrap().source, PixelSize::new(350, 150), "the boards' union, not the 350 x 200 canvas");
    sent.borrow_mut().clear();
    export_with(&mut h, id, &png_options());
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
    let png = exported(&sent);
    assert_eq!((png.size.width, png.size.height), (350, 150));
    assert_eq!(at(&png, 125, 125), [0.0, 0.0, 0.0, 0.0], "the gap between the boards travels with the union");
    assert_eq!(at(&png, 300, 100), [1.0, 1.0, 1.0, 1.0], "the far board paints inside it");
}

/// Neighbour, no artboards at all: the document canvas stays the export area.
/// Guard test: passes before and after the fix.
#[test]
fn a_document_without_artboards_still_exports_its_canvas() {
    let (app, sent) = artboard_app(300, 200);
    let mut h = harness(app);
    let id = open_and_settle(&mut h);
    assert_eq!(h.state().export_open.as_ref().unwrap().source, PixelSize::new(300, 200));
    sent.borrow_mut().clear();
    export_with(&mut h, id, &png_options());
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
    let png = exported(&sent);
    assert_eq!((png.size.width, png.size.height), (300, 200), "guard: the whole canvas, as it has always been");
}

/// Neighbour, one board already covering the canvas: the union IS the canvas, so that export is
/// unchanged. Guard test: passes before and after the fix.
#[test]
fn one_board_covering_the_canvas_still_exports_the_canvas() {
    let (mut app, sent) = artboard_app(200, 150);
    app.run("layer.new.artboard", json!({"rect": [0, 0, 200, 150], "background": "white"})).unwrap();
    let mut h = harness(app);
    let id = open_and_settle(&mut h);
    assert_eq!(h.state().export_open.as_ref().unwrap().source, PixelSize::new(200, 150));
    sent.borrow_mut().clear();
    export_with(&mut h, id, &jpg_options());
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
    let jpg = exported(&sent);
    assert_eq!((jpg.size.width, jpg.size.height), (200, 150), "guard: board-sized canvas, unchanged export");
    assert_eq!(at(&jpg, 100, 75), [1.0, 1.0, 1.0, 1.0]);
}

/// Neighbour, the path the report says works: Layer > Export As on the artboard keeps taking the
/// board's own bounds through the layer path. Guard test: passes before and after the fix.
#[test]
fn artboard_layer_export_as_still_opens_with_the_board_size() {
    let (mut app, _sent) = artboard_app(300, 200);
    let ids = three_boards(app.as_mut());
    delete_two(app.as_mut(), &ids);
    let mut h = harness(app);
    open_layer(h.state_mut(), ids[0]).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().export_open.as_ref().unwrap().source, PixelSize::new(100, 100), "guard: the board, not the 350 x 200 canvas");
    assert_eq!(output_of(&h, &dialog_options(&h)), (Some(100.0), Some(100.0)));
}

/// Red before the fix: a recorded Export As step (#7207) replays the same area the dialog
/// exported - the boards' union - instead of the whole grown canvas.
#[test]
fn a_replayed_export_as_step_crops_the_same_boards_area() {
    let (mut app, sent) = artboard_app(300, 200);
    let ids = three_boards(app.as_mut());
    delete_two(app.as_mut(), &ids);
    replay(app.as_mut(), &json!({"path": "/out/x.png", "format": "png", "scale": 100})).unwrap();
    let png = exported(&sent);
    assert_eq!((png.size.width, png.size.height), (100, 100), "the replay matches what the dialog records (#7207)");
}

/// The designed rule for real content outside every board: the boards define the exported area, so
/// what lies on the canvas ground outside them is not in the file - the same rule as exporting
/// artboards only. A layer that crosses the area's edge keeps the part inside it; the part outside
/// every board is dropped, and the document itself is untouched by that.
#[test]
fn content_outside_every_board_is_dropped_from_the_export() {
    let (mut app, sent) = artboard_app(300, 200);
    app.run("layer.new.artboard", json!({"rect": [0, 0, 100, 100], "background": "white"})).unwrap();
    // A red band that crosses the board's bottom edge and goes on over the ground beside it.
    app.run("shape.create", json!({"kind": "rect", "rect": [0, 80, 300, 40], "fill": "#ff0000"})).unwrap();
    let on_canvas = app.session.active().unwrap().doc.clone();
    assert_eq!(at(&on_canvas, 150, 90), [1.0, 0.0, 0.0, 1.0], "the document paints outside every board");

    let mut h = harness(app);
    let id = open_and_settle(&mut h);
    assert_eq!(h.state().export_open.as_ref().unwrap().source, PixelSize::new(100, 100), "the board defines the area, not the canvas the band paints on");
    sent.borrow_mut().clear();
    export_with(&mut h, id, &png_options());
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
    let png = exported(&sent);
    assert_eq!((png.size.width, png.size.height), (100, 100), "the band's ground outside the board is not in the file");
    assert_eq!(at(&png, 50, 90), [1.0, 0.0, 0.0, 1.0], "the part that crosses into the area travels with it");
    assert_eq!(at(&png, 50, 50), [1.0, 1.0, 1.0, 1.0], "the board's own white under the band");
}

/// Not measured by the issue; hidden boards currently count. `artboard_area` walks
/// `doc.artboards()`, which lists every top-level board whatever its eye says, so hiding one board
/// leaves the exported area as wide as if it were shown - while it paints nothing into it. Pinned
/// as today's behaviour, not a claim that it is the rule to keep.
#[test]
fn a_hidden_board_still_widens_the_exported_area() {
    let (mut app, sent) = artboard_app(300, 200);
    let ids = three_boards(app.as_mut());
    app.run("layer.setProps", json!({"layer": ids[2].0, "visible": false})).unwrap();
    let mut h = harness(app);
    let id = open_and_settle(&mut h);
    assert_eq!(
        h.state().export_open.as_ref().unwrap().source,
        PixelSize::new(350, 150),
        "the hidden board's rect still counts: the two shown ones alone would give 250 x 100"
    );
    sent.borrow_mut().clear();
    export_with(&mut h, id, &png_options());
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
    let png = exported(&sent);
    assert_eq!((png.size.width, png.size.height), (350, 150), "pin: hiding a board does not narrow the area");
    assert_eq!(at(&png, 300, 100), [0.0, 0.0, 0.0, 0.0], "the hidden board widens the area but paints nothing in it");
    assert_eq!(at(&png, 50, 50), [1.0, 1.0, 1.0, 1.0], "the boards that are shown still paint");
}

/// Scale All on an artboard document: each row scales the boards' area, not the canvas, because the
/// batch gets the same `__area` field the single file does (#15028 and #15291 meeting).
#[test]
fn scale_all_exports_the_boards_area_at_every_row_size() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, sent, files) = artboard_app_with_picks(300, 200);
    let ids = three_boards(app.as_mut());
    delete_two(app.as_mut(), &ids);
    let id = open(app.as_mut()).unwrap();
    let ctx = app.ctx().clone();
    let rows = [ScaleRow::new(ScaleSize::X1, ""), ScaleRow::new(ScaleSize::X2, "@2x")];
    app.framework.dialogs.set_json_field(&ctx, id, "scale_all", serde_json::to_value(&rows).unwrap()).unwrap();
    crate::dialogs::confirm(app.as_mut(), &ctx, id).expect("OK is enabled");
    files.answer(Picked::Paths(vec![dir.path().to_path_buf()])).expect("a folder choice is open");
    app.poll_file_dialog();
    sent.borrow_mut().clear();
    crate::dialogs::confirm(app.as_mut(), &ctx, id).expect("OK is enabled");
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let sizes: Vec<(u32, u32)> = sent.borrow().iter().map(|d| (d.size.width, d.size.height)).collect();
    assert_eq!(sizes, vec![(100, 100), (200, 200)], "one file per row, each at the board area scaled by that row");
}

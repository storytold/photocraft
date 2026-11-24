//! #14480: Image › Apply Image's Source document dropdown, the Layer and Channel rows that follow it, the Target
//! line, and the refusal of a source of another bit depth.
//!
//! The five documents of the issue, opened in its order: "other" (RGB 8 bit, 300 x 200), "gray" (Grayscale 8 bit,
//! 400 x 300, left half black, right half white), "rgb16" (RGB 16 bit, 400 x 300), "source" (RGB 8 bit, 400 x 300,
//! Background 128 plus a layer "S1" white on the right half) and "target" (RGB 8 bit, 400 x 300; Background left half
//! 200/100/50, right half 50/150/250; layers "Top" and "Masked"; alpha channel "Alpha 1" white over (0,0)-(200,150)).
//! "target" is active, its Background targeted.

use super::params_for;

/// The dialog's row keys (shell keys: `__` keys never reach the command).
const SOURCE: &str = "__sourceDocument";

/// The Layer row of a source document that is not the target.
fn layer_key(index: usize) -> String {
    format!("__sourceLayer/{index}")
}

/// The Channel row of a source document that is not the target.
fn channel_key(index: usize) -> String {
    format!("__sourceChannel/{index}")
}
use crate::PhotocraftApp;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::ParamKind;
use serde_json::{Value, json};

const APPLY: &str = "image.applyImage";

fn run(app: &mut PhotocraftApp, id: &str, params: Value) {
    app.run(id, params).unwrap_or_else(|e| panic!("{id}: {e}"));
}

/// Fills the rectangle `x, y, w, h` of the active layer with `color`.
fn fill(app: &mut PhotocraftApp, rect: [u32; 4], color: &str) {
    run(app, "select.rect", json!({"x": rect[0], "y": rect[1], "width": rect[2], "height": rect[3]}));
    run(app, "edit.fill", json!({"color": color}));
    run(app, "select.deselect", json!({}));
}

fn five_documents() -> (Box<PhotocraftApp>, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    run(&mut app, "file.new", json!({"width": 300, "height": 200, "name": "other"}));
    run(&mut app, "file.new", json!({"width": 400, "height": 300, "mode": "gray", "background": "#000000", "name": "gray"}));
    fill(&mut app, [200, 0, 200, 300], "#ffffff");
    run(&mut app, "file.new", json!({"width": 400, "height": 300, "depth": 16, "background": "#404040", "name": "rgb16"}));
    run(&mut app, "file.new", json!({"width": 400, "height": 300, "background": "#808080", "name": "source"}));
    run(&mut app, "layer.new.layer", json!({"name": "S1"}));
    fill(&mut app, [200, 0, 200, 300], "#ffffff");
    run(&mut app, "file.new", json!({"width": 400, "height": 300, "background": "#c86432", "name": "target"}));
    fill(&mut app, [200, 0, 200, 300], "#3296fa");
    run(&mut app, "select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 150}));
    run(&mut app, "select.saveSelection", json!({"name": "Alpha 1"}));
    run(&mut app, "select.deselect", json!({}));
    run(&mut app, "layer.new.layer", json!({"name": "Top"}));
    run(&mut app, "layer.new.layer", json!({"name": "Masked"}));
    run(&mut app, "layer.select", json!({"name": "Background"}));
    app.sync_views();
    let ctx = app.ctx().clone();
    (app, ctx)
}

fn frame(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let mut output = ctx.run_ui(Default::default(), |ui| {
        let mut frame = eframe::Frame::_new_kittest();
        app.logic_frame(ui.ctx(), &mut frame);
        app.ui_frame(ui, &mut frame);
    });
    output.textures_delta.clear();
}

fn open(app: &mut PhotocraftApp, ctx: &egui::Context) -> DialogId {
    let reply = crate::menus::invoke(app, ctx, APPLY, json!({})).unwrap();
    let want = reply["dialog"].as_str().expect("the reply names a dialog").to_owned();
    let id = app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|id| id.value().to_string() == want).expect("the dialog is open");
    frame(app, ctx);
    id
}

fn fields(app: &PhotocraftApp, id: DialogId) -> Value {
    app.framework.dialogs.open_dialogs().into_iter().find(|open| open.id == id).and_then(|open| open.fields).expect("an open dialog")
}

fn set(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId, key: &str, value: Value) {
    app.framework.dialogs.set_json_field(ctx, id, key, value).unwrap_or_else(|e| panic!("{key}: {e}"));
    // The form brings its derived keys up to date at its next pass.
    frame(app, ctx);
}

/// The `(id, label)` options of choice row `key`.
fn options(app: &mut PhotocraftApp, key: &str) -> Vec<(String, String)> {
    let (params, _) = params_for(app, APPLY).expect("Apply Image has a form");
    match &params.iter().find(|p| p.key == key).unwrap_or_else(|| panic!("no row {key}")).kind {
        ParamKind::Choice { options, .. } => options.iter().map(|(id, label)| (id.clone(), label.as_str().to_owned())).collect(),
        other => panic!("{key} is not a choice: {other:?}"),
    }
}

fn ids(app: &mut PhotocraftApp, key: &str) -> Vec<String> {
    options(app, key).into_iter().map(|(id, _)| id).collect()
}

fn index_of(app: &PhotocraftApp, name: &str) -> usize {
    app.session.documents().iter().position(|st| st.doc.name == name).unwrap_or_else(|| panic!("no document {name}"))
}

/// The active layer's 8-bit RGB at `(x, y)`.
fn rgb(app: &PhotocraftApp, x: i32, y: i32) -> [u8; 3] {
    let st = app.session.active().unwrap();
    let p = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y);
    [0, 1, 2].map(|c| (p[c] * 255.0).round() as u8)
}

#[test]
fn source_lists_same_size_and_depth_documents_and_defaults_to_the_target() {
    let (mut app, ctx) = five_documents();
    let names: Vec<String> = options(&mut app, SOURCE).into_iter().map(|(_, name)| name).collect();
    assert_eq!(names, ["gray", "source", "target"], "not other (size), not rgb16 (bit depth)");
    let d = open(&mut app, &ctx);
    assert_eq!(fields(&app, d)[SOURCE], index_of(&app, "target").to_string());
}

#[test]
fn another_target_lists_its_own_size_and_depth() {
    let (mut app, _) = five_documents();
    // rgb16 as the target: only itself.
    let rgb16 = index_of(&app, "rgb16");
    run(&mut app, "document.activate", json!({"document": rgb16}));
    let names: Vec<String> = options(&mut app, SOURCE).into_iter().map(|(_, name)| name).collect();
    assert_eq!(names, ["rgb16"]);
    // gray as the target: the RGB documents of its size and depth too.
    let gray = index_of(&app, "gray");
    run(&mut app, "document.activate", json!({"document": gray}));
    let names: Vec<String> = options(&mut app, SOURCE).into_iter().map(|(_, name)| name).collect();
    assert_eq!(names, ["gray", "source", "target"]);
}

#[test]
fn layer_and_channel_follow_the_chosen_document() {
    let (mut app, ctx) = five_documents();
    let (gray, source) = (index_of(&app, "gray"), index_of(&app, "source"));
    // The target's own rows: Merged, then its layers top to bottom.
    assert_eq!(ids(&mut app, "sourceLayer"), ["merged", "Masked", "Top", "Background"]);
    assert_eq!(ids(&mut app, &layer_key(source)), ["merged", "S1", "Background"]);
    assert_eq!(ids(&mut app, &channel_key(source)), ["RGB", "Red", "Green", "Blue"]);
    // A single-layer document lists only Background, with no Merged entry.
    assert_eq!(ids(&mut app, &layer_key(gray)), ["Background"]);
    assert_eq!(ids(&mut app, &channel_key(gray)), ["Gray"]);
    // Choosing a document shows its rows and hides the target's.
    let d = open(&mut app, &ctx);
    set(&mut app, &ctx, d, SOURCE, json!(source.to_string()));
    let shown = fields(&app, d);
    assert_eq!(shown["__sourceIsTarget"], false);
    assert_eq!(shown[format!("__sourceShown/{source}")], true);
    assert_eq!(shown[format!("__sourceShown/{gray}")], false);
    assert_eq!(shown["source"], json!({"document": source, "layer": "merged", "channel": "RGB", "invert": false}));
    let target = index_of(&app, "target");
    set(&mut app, &ctx, d, SOURCE, json!(target.to_string()));
    let shown = fields(&app, d);
    assert_eq!(shown["__sourceIsTarget"], true);
    assert!(shown.get("source").is_none(), "the target sends the flat keys: {shown}");
}

#[test]
fn the_source_rows_read_layer_and_channel_for_every_document() {
    let (mut app, _) = five_documents();
    let source = index_of(&app, "source");
    let (params, _) = params_for(&mut app, APPLY).unwrap();
    let label = |key: &str| params.iter().find(|p| p.key == key).map(|p| p.label.as_str().to_owned());
    assert_eq!(label("sourceChannel").as_deref(), Some("Channel"));
    assert_eq!(label(&channel_key(source)).as_deref(), Some("Channel"));
    assert_eq!(label("sourceLayer").as_deref(), Some("Layer"));
}

#[test]
fn the_target_line_names_the_target() {
    let (mut app, _) = five_documents();
    let (params, _) = params_for(&mut app, APPLY).unwrap();
    let row = params.iter().find(|p| p.key == "__target").expect("a Target row");
    assert_eq!(row.label.as_str(), "Target");
    match &row.kind {
        ParamKind::ReadOnlyText { text, .. } => assert_eq!(text.as_str(), "target"),
        other => panic!("the Target row is static text: {other:?}"),
    }
}

#[test]
fn ok_applies_the_chosen_source_document() {
    let (mut app, ctx) = five_documents();
    let source = index_of(&app, "source");
    let d = open(&mut app, &ctx);
    set(&mut app, &ctx, d, "blending", json!("normal"));
    set(&mut app, &ctx, d, SOURCE, json!(source.to_string()));
    crate::dialogs::confirm(&mut app, &ctx, d).unwrap();
    // "source" Merged: 128 on the left, white on the right.
    assert_eq!(rgb(&app, 50, 50), [128, 128, 128]);
    assert_eq!(rgb(&app, 300, 50), [255, 255, 255]);
    run(&mut app, "edit.undo", json!({}));
    // Its Background alone (128 everywhere), inverted.
    let d = open(&mut app, &ctx);
    set(&mut app, &ctx, d, &layer_key(source), json!("Background"));
    set(&mut app, &ctx, d, "sourceInvert", json!(true));
    crate::dialogs::confirm(&mut app, &ctx, d).unwrap();
    assert_eq!(rgb(&app, 50, 50), [127, 127, 127]);
    assert_eq!(rgb(&app, 300, 50), [127, 127, 127]);
}

#[test]
fn ok_with_the_mask_of_the_issue() {
    let (mut app, ctx) = five_documents();
    let (source, target) = (index_of(&app, "source"), index_of(&app, "target"));
    for (invert, at_50, at_300) in [(false, [128, 128, 128], [50, 150, 250]), (true, [200, 100, 50], [255, 255, 255])] {
        let d = open(&mut app, &ctx);
        set(&mut app, &ctx, d, "blending", json!("normal"));
        set(&mut app, &ctx, d, SOURCE, json!(source.to_string()));
        set(&mut app, &ctx, d, "__mask", json!(true));
        set(&mut app, &ctx, d, "__maskSource", json!(target.to_string()));
        set(&mut app, &ctx, d, &format!("__maskChannel/{target}"), json!("Alpha 1"));
        set(&mut app, &ctx, d, "__maskInvert", json!(invert));
        crate::dialogs::confirm(&mut app, &ctx, d).unwrap();
        assert_eq!(rgb(&app, 50, 50), at_50, "invert {invert}");
        assert_eq!(rgb(&app, 300, 50), at_300, "invert {invert}");
        run(&mut app, "edit.undo", json!({}));
    }
}

#[test]
fn a_source_of_another_bit_depth_is_refused_and_changes_nothing() {
    let (mut app, _) = five_documents();
    let rgb16 = index_of(&app, "rgb16");
    let revision = app.session.active().unwrap().revision;
    for params in [
        json!({"source": {"document": rgb16}, "blending": "normal"}),
        json!({"sourceDocument": rgb16, "blending": "normal"}),
        json!({"mask": {"document": rgb16, "channel": "RGB"}, "blending": "normal"}),
    ] {
        let error = app.session.execute(APPLY, params.clone()).expect_err("another bit depth is refused").to_string();
        assert!(error.contains("bit depth"), "{params}: {error}");
        assert_eq!(app.session.active().unwrap().revision, revision, "{params} changed the target");
    }
    assert_eq!(rgb(&app, 50, 50), [200, 100, 50]);
    // The same bit depth in another mode still applies.
    let gray = index_of(&app, "gray");
    app.session.execute(APPLY, json!({"source": {"document": gray}, "blending": "normal"})).unwrap();
    assert_eq!(rgb(&app, 300, 50), [255, 255, 255]);
}

//! #14489: Image › Apply Image's Mask... checkbox and its group (Source, Layer, Channel, Invert).
//!
//! The five documents of the issue, opened in its order: "other" (RGB 8 bit, 300 x 200), "gray" (Grayscale 8 bit,
//! 400 x 300, left half black, right half white), "rgb16" (RGB 16 bit, 400 x 300), "source" (RGB 8 bit, 400 x 300,
//! Background 128 plus a layer "S1" white on the right half) and "target" (RGB 8 bit, 400 x 300; Background left half
//! 200/100/50, right half 50/150/250; layers "Top" and "Masked"; alpha channel "Alpha 1" white over (0,0)-(200,150)).
//! "target" is active, its Background targeted.

use super::params_for;

/// The dialog's row keys (shell keys: `__` keys never reach the command).
const MASK_ON: &str = "__mask";
const MASK_SOURCE: &str = "__maskSource";
const MASK_INVERT: &str = "__maskInvert";

fn mask_layer_key(index: usize) -> String {
    format!("__maskLayer/{index}")
}

fn mask_channel_key(index: usize) -> String {
    format!("__maskChannel/{index}")
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
fn mask_is_off_by_default_and_sends_no_mask() {
    let (mut app, ctx) = five_documents();
    let d = open(&mut app, &ctx);
    let shown = fields(&app, d);
    assert_eq!(shown[MASK_ON], false);
    for key in ["maskDocument", "maskLayer", "maskChannel", "maskInvert"] {
        assert!(shown.get(key).is_none(), "{key} is sent with Mask off: {shown}");
    }
    // Ticked, then unticked again: no mask either.
    set(&mut app, &ctx, d, MASK_ON, json!(true));
    assert!(fields(&app, d).get("maskChannel").is_some());
    set(&mut app, &ctx, d, MASK_ON, json!(false));
    assert!(fields(&app, d).get("maskChannel").is_none());
    set(&mut app, &ctx, d, "blending", json!("normal"));
    set(&mut app, &ctx, d, "sourceInvert", json!(true));
    crate::dialogs::confirm(&mut app, &ctx, d).unwrap();
    // The whole layer is the inverted target.
    assert_eq!(rgb(&app, 50, 50), [55, 155, 205]);
    assert_eq!(rgb(&app, 300, 50), [205, 105, 5]);
}

#[test]
fn mask_source_lists_same_size_and_depth_documents_in_open_order() {
    let (mut app, ctx) = five_documents();
    let names: Vec<String> = options(&mut app, MASK_SOURCE).into_iter().map(|(_, name)| name).collect();
    assert_eq!(names, ["gray", "source", "target"]);
    let d = open(&mut app, &ctx);
    assert_eq!(fields(&app, d)[MASK_SOURCE], index_of(&app, "gray").to_string(), "the first document of the list");
}

#[test]
fn mask_layer_follows_the_mask_source() {
    let (mut app, _) = five_documents();
    let (gray, source, target) = (index_of(&app, "gray"), index_of(&app, "source"), index_of(&app, "target"));
    assert_eq!(ids(&mut app, &mask_layer_key(gray)), ["Background"]);
    assert_eq!(ids(&mut app, &mask_layer_key(source)), ["merged", "S1", "Background"]);
    assert_eq!(ids(&mut app, &mask_layer_key(target)), ["merged", "Masked", "Top", "Background"]);
    let (params, _) = params_for(&mut app, APPLY).unwrap();
    let default = |key: &str| match &params.iter().find(|p| p.key == key).unwrap().kind {
        ParamKind::Choice { default, .. } => default.clone(),
        _ => None,
    };
    assert_eq!(default(&mask_layer_key(gray)).as_deref(), Some("Background"));
    assert_eq!(default(&mask_layer_key(target)).as_deref(), Some("merged"));
    // Documents of another size or bit depth have no rows.
    let rgb16 = index_of(&app, "rgb16");
    assert!(params.iter().all(|p| p.key != mask_layer_key(rgb16)));
}

#[test]
fn mask_channel_lists_gray_the_colours_and_the_alpha_channels() {
    let (mut app, _) = five_documents();
    let (gray, target) = (index_of(&app, "gray"), index_of(&app, "target"));
    let labels = |list: Vec<(String, String)>| list.into_iter().map(|(_, label)| label).collect::<Vec<_>>();
    assert_eq!(labels(options(&mut app, &mask_channel_key(gray))), ["Gray"]);
    assert_eq!(labels(options(&mut app, &mask_channel_key(target))), ["Gray", "Red", "Green", "Blue", "Alpha 1"]);
    // The composite keeps the engine's id; the engine reads a mask composite as luminosity.
    assert_eq!(ids(&mut app, &mask_channel_key(target))[0], "RGB");
}

#[test]
fn ok_applies_through_the_chosen_mask_and_its_invert() {
    let (mut app, ctx) = five_documents();
    let target = index_of(&app, "target");
    for (invert, at_50, at_300) in [(false, [55, 155, 205], [50, 150, 250]), (true, [200, 100, 50], [205, 105, 5])] {
        let d = open(&mut app, &ctx);
        set(&mut app, &ctx, d, "blending", json!("normal"));
        set(&mut app, &ctx, d, "sourceInvert", json!(true));
        set(&mut app, &ctx, d, MASK_ON, json!(true));
        set(&mut app, &ctx, d, MASK_SOURCE, json!(target.to_string()));
        set(&mut app, &ctx, d, &mask_channel_key(target), json!("Alpha 1"));
        set(&mut app, &ctx, d, MASK_INVERT, json!(invert));
        let sent = fields(&app, d);
        assert_eq!(sent["maskDocument"], target);
        assert_eq!(sent["maskLayer"], "merged");
        assert_eq!(sent["maskChannel"], "Alpha 1");
        // Invert is recorded only when it is on.
        assert_eq!(sent.get("maskInvert").cloned(), invert.then_some(json!(true)));
        crate::dialogs::confirm(&mut app, &ctx, d).unwrap();
        assert_eq!(rgb(&app, 50, 50), at_50, "invert {invert}");
        assert_eq!(rgb(&app, 300, 50), at_300, "invert {invert}");
        run(&mut app, "edit.undo", json!({}));
    }
}

#[test]
fn a_mask_from_another_document() {
    let (mut app, ctx) = five_documents();
    let gray = index_of(&app, "gray");
    let d = open(&mut app, &ctx);
    set(&mut app, &ctx, d, "blending", json!("normal"));
    set(&mut app, &ctx, d, "sourceInvert", json!(true));
    set(&mut app, &ctx, d, MASK_ON, json!(true));
    assert_eq!(fields(&app, d)["maskDocument"], gray);
    crate::dialogs::confirm(&mut app, &ctx, d).unwrap();
    // "gray" is black on the left, white on the right.
    assert_eq!(rgb(&app, 50, 50), [200, 100, 50]);
    assert_eq!(rgb(&app, 300, 50), [205, 105, 5]);
    run(&mut app, "edit.undo", json!({}));
    // OK right after an edit, with no pass in between, sends the edited mask.
    let d = open(&mut app, &ctx);
    app.framework.dialogs.set_json_field(&ctx, d, MASK_INVERT, json!(true)).unwrap();
    crate::dialogs::confirm(&mut app, &ctx, d).unwrap();
    assert_eq!(rgb(&app, 50, 50), [55, 155, 205]);
    assert_eq!(rgb(&app, 300, 50), [50, 150, 250]);
}

/// "clash" (RGB 8 bit, 400 x 300, Background 200/100/50): an alpha channel named "Red" white on the left half, and a
/// pixel layer named "merged" white on the right half above the Background, which is targeted.
fn names_like_the_built_in_choices() -> (Box<PhotocraftApp>, egui::Context, usize) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    run(&mut app, "file.new", json!({"width": 400, "height": 300, "background": "#c86432", "name": "clash"}));
    run(&mut app, "select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 300}));
    run(&mut app, "select.saveSelection", json!({"name": "Red"}));
    run(&mut app, "select.deselect", json!({}));
    run(&mut app, "layer.new.layer", json!({"name": "merged"}));
    fill(&mut app, [200, 0, 200, 300], "#ffffff");
    run(&mut app, "layer.select", json!({"name": "Background"}));
    app.sync_views();
    let ctx = app.ctx().clone();
    let clash = index_of(&app, "clash");
    (app, ctx, clash)
}

/// The option id of row `key` at `position` of its list, after checking the list's labels and that no two options
/// share an id.
fn option_at(app: &mut PhotocraftApp, key: &str, labels: &[&str], position: usize) -> String {
    let list = options(app, key);
    assert_eq!(list.iter().map(|(_, label)| label.as_str()).collect::<Vec<_>>(), labels, "{key}");
    let mut unique: Vec<&String> = list.iter().map(|(id, _)| id).collect();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), list.len(), "{key} options share an id: {list:?}");
    list[position].0.clone()
}

#[test]
fn a_mask_alpha_named_like_a_colour_channel_reads_the_alpha() {
    let (mut app, ctx, clash) = names_like_the_built_in_choices();
    let alpha = option_at(&mut app, &mask_channel_key(clash), &["Gray", "Red", "Green", "Blue", "Red"], 4);
    let d = open(&mut app, &ctx);
    set(&mut app, &ctx, d, "blending", json!("normal"));
    set(&mut app, &ctx, d, "sourceInvert", json!(true));
    set(&mut app, &ctx, d, MASK_ON, json!(true));
    set(&mut app, &ctx, d, MASK_SOURCE, json!(clash.to_string()));
    set(&mut app, &ctx, d, &mask_channel_key(clash), json!(alpha));
    assert_eq!(fields(&app, d)["maskChannel"], json!({"alpha": 0}), "an explicit alpha selector");
    crate::dialogs::confirm(&mut app, &ctx, d).unwrap();
    // The alpha masks the left half in and the right half out; the red plane (200 everywhere) would mix both.
    assert_eq!(rgb(&app, 50, 50), [55, 155, 205]);
    assert_eq!(rgb(&app, 300, 50), [200, 100, 50]);
}

#[test]
fn a_mask_layer_named_merged_is_that_layer_not_the_composite() {
    let (mut app, ctx, clash) = names_like_the_built_in_choices();
    let layer = option_at(&mut app, &mask_layer_key(clash), &["Merged", "merged", "Background"], 1);
    let d = open(&mut app, &ctx);
    set(&mut app, &ctx, d, "blending", json!("normal"));
    set(&mut app, &ctx, d, "sourceInvert", json!(true));
    set(&mut app, &ctx, d, MASK_ON, json!(true));
    set(&mut app, &ctx, d, MASK_SOURCE, json!(clash.to_string()));
    set(&mut app, &ctx, d, &mask_layer_key(clash), json!(layer));
    assert_eq!(fields(&app, d)["maskLayer"], json!({"name": "merged"}), "an explicit layer name selector");
    crate::dialogs::confirm(&mut app, &ctx, d).unwrap();
    let through_dialog = [rgb(&app, 50, 50), rgb(&app, 300, 50)];
    run(&mut app, "edit.undo", json!({}));
    // The same as the command with the layer named explicitly, and not the result of the composite as mask.
    let mask = |layer: Value| json!({"sourceInvert": true, "blending": "normal", "mask": {"document": clash, "layer": layer, "channel": "RGB"}});
    run(&mut app, APPLY, mask(json!({"name": "merged"})));
    assert_eq!([rgb(&app, 50, 50), rgb(&app, 300, 50)], through_dialog);
    run(&mut app, "edit.undo", json!({}));
    run(&mut app, APPLY, mask(json!("merged")));
    assert_ne!([rgb(&app, 50, 50), rgb(&app, 300, 50)], through_dialog, "the composite masks differently");
}

/// "sentinels" (RGB 8 bit, 400 x 300, Background 200/100/50, targeted): an alpha channel named "none" white on the left
/// half, and an alpha channel with no name white on the right half. The engine reads a mask channel "none" as no mask
/// and drops an empty one.
fn alphas_named_like_no_mask() -> (Box<PhotocraftApp>, egui::Context, usize) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    run(&mut app, "file.new", json!({"width": 400, "height": 300, "background": "#c86432", "name": "sentinels"}));
    run(&mut app, "select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 300}));
    run(&mut app, "select.saveSelection", json!({"name": "none"}));
    run(&mut app, "select.rect", json!({"x": 200, "y": 0, "width": 200, "height": 300}));
    run(&mut app, "select.saveSelection", json!({"name": "unnamed"}));
    run(&mut app, "select.deselect", json!({}));
    // The commands refuse an empty name; a file can still bring one.
    std::sync::Arc::make_mut(app.session.active_mut().unwrap().doc_mut()).channels[1].name = String::new();
    app.sync_views();
    let ctx = app.ctx().clone();
    let doc = index_of(&app, "sentinels");
    (app, ctx, doc)
}

/// Applies the inverted target with Normal through the mask alpha at `position` of the Channel list of "sentinels",
/// checking the selector the dialog sends.
fn apply_through_mask_alpha(position: usize, selector: Value) -> PhotocraftApp {
    let (mut app, ctx, doc) = alphas_named_like_no_mask();
    let alpha = option_at(&mut app, &mask_channel_key(doc), &["Gray", "Red", "Green", "Blue", "none", ""], position);
    let d = open(&mut app, &ctx);
    set(&mut app, &ctx, d, "blending", json!("normal"));
    set(&mut app, &ctx, d, "sourceInvert", json!(true));
    set(&mut app, &ctx, d, MASK_ON, json!(true));
    set(&mut app, &ctx, d, MASK_SOURCE, json!(doc.to_string()));
    set(&mut app, &ctx, d, &mask_channel_key(doc), json!(alpha));
    assert_eq!(fields(&app, d)["maskChannel"], selector, "an explicit alpha selector");
    crate::dialogs::confirm(&mut app, &ctx, d).unwrap();
    *app
}

#[test]
fn a_mask_alpha_named_none_masks() {
    let app = apply_through_mask_alpha(4, json!({"alpha": 0}));
    // Only the left half, where the alpha is white, takes the inverted target; without a mask both halves would.
    assert_eq!(rgb(&app, 50, 50), [55, 155, 205]);
    assert_eq!(rgb(&app, 300, 50), [200, 100, 50]);
}

#[test]
fn a_mask_alpha_with_no_name_masks() {
    let app = apply_through_mask_alpha(5, json!({"alpha": 1}));
    assert_eq!(rgb(&app, 50, 50), [200, 100, 50]);
    assert_eq!(rgb(&app, 300, 50), [55, 155, 205]);
}

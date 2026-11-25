//! #12121: dragging the Patch selection on a Type layer asks the rasterize question, as the
//! painting tools do, instead of refusing in the status bar. OK rasterizes the layer and applies
//! the patch the drag worked out; Cancel changes nothing. A lasso drag with the Patch tool still
//! only makes the selection. Shape layers, Content-Aware mode and a targeted mask are *not
//! measured* in the issue and keep today's behaviour, pinned here.

use craft::ui::dialog::DialogId;
use egui::Modifiers;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::dialogs::alerts;
use crate::rasterize_prompt::Rasterize;
use crate::state::Tool;

/// A 200 x 120 document whose active layer holds `kind` content (a Type, Shape, Smart Object or
/// fill layer, or plain pixels), the Patch tool active, and a hard 40 x 30 rectangle selected at
/// 40,40.
fn app(kind: &str) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    // The rasterize step and the patch settle in the pass that asks for them.
    app.background_jobs = false;
    app.run("file.new", json!({"width": 200, "height": 120, "background": "white"})).unwrap();
    match kind {
        "Type" => app.run("type.create", json!({"x": 20, "y": 60, "text": "Hello world"})).map(|_| ()),
        "Shape" => app.run("shape.create", json!({"kind": "rect", "rect": [20, 40, 90, 70], "fill": "#ff0000"})).map(|_| ()),
        "Smart Object" => {
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
            app.run("layer.smartObjects.convertToSmartObject", json!({})).map(|_| ())
        }
        "Fill" => app.run("layer.newFillLayer.solidColor", json!({"color": "#00ff00"})).map(|_| ()),
        _ => Ok(()),
    }
    .unwrap();
    app.run("select.rect", json!({"x": 40, "y": 40, "width": 40, "height": 30, "mode": "replace"})).unwrap();
    // The layer's own pixels, not its mask.
    app.ui.mask_target = false;
    app.sync_mask_targets();
    app.ui.tool = Tool::Patch;
    app.ui.tool_options.patch_content_aware = false;
    app
}

/// Drags the patch from inside the selection at 55,50 to 115,50: the offset 60,0.
fn drag_patch(app: &mut PhotocraftApp) {
    let m = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: 55.0, y: 50.0, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: 80.0, y: 50.0, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: 115.0, y: 50.0, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: 115.0, y: 50.0 }, m);
}

/// The open rasterize prompt, whichever family opened it.
fn prompt(app: &PhotocraftApp) -> Option<DialogId> {
    alerts::prompt_tagged::<Rasterize>(app)
}

/// OK on the open prompt, as `ui.dialog.confirm` does: the answer acts inside the call.
fn answer(app: &mut PhotocraftApp, id: DialogId) {
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(app, &ctx, id).unwrap();
}

/// The params of every `paint.patch` in the History.
fn patch_params(app: &PhotocraftApp) -> Vec<Value> {
    app.session.journal.iter().filter(|(id, _)| id == "paint.patch").map(|(_, p)| p.clone()).collect()
}

fn past(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// The active layer's content, as "Type", "Shape", "Pixel" ...
fn active_content(app: &PhotocraftApp) -> &'static str {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().content.kind_name()
}

/// The selected area as [x, y, width, height], for the Destination mode's moved selection.
fn selection_bounds(app: &PhotocraftApp) -> [i32; 4] {
    let sel = app.session.active().unwrap().doc.selection.clone().unwrap();
    let b = sel.content_bounds();
    [b.x0, b.y0, b.x1 - b.x0, b.y1 - b.y0]
}

/// Done when 1: the drag on a Type layer asks instead of refusing; nothing happens until the answer.
#[test]
fn a_patch_drag_on_a_type_layer_asks_to_rasterize_first() {
    let mut app = app("Type");
    let before = (past(&app), selection_bounds(&app));
    drag_patch(&mut app);
    prompt(&app).unwrap_or_else(|| panic!("the patch drag opened no prompt; status: {}", app.ui.status));
    assert!(!app.ui.status_error, "the question stands in place of the status-bar refusal: {}", app.ui.status);
    assert_eq!(active_content(&app), "Type", "the layer waits for the answer");
    assert_eq!(past(&app), before.0, "nothing is recorded before the answer");
    assert_eq!(selection_bounds(&app), before.1, "the selection is where the drag left it");
    assert!(patch_params(&app).is_empty(), "no patch ran");
    assert!(!crate::patch_preview::showing(&app), "the healed preview is off the canvas once the question is up");
    // A second drag while the question is up does not stack another prompt.
    drag_patch(&mut app);
    assert_eq!(app.framework.dialogs.open_dialogs().len(), 1, "{:?}", app.framework.dialogs.open_dialogs());
}

/// Done when 1: OK rasterizes the layer and applies the patch the drag worked out. Destination
/// mode also moves the selection, which proves the patch ran without depending on the glyphs.
#[test]
fn ok_rasterizes_the_layer_and_applies_the_patch() {
    let mut app = app("Type");
    app.ui.tool_options.patch_mode = "destination".into();
    let before = (past(&app), selection_bounds(&app));
    drag_patch(&mut app);
    let id = prompt(&app).expect("the prompt");
    answer(&mut app, id);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(active_content(&app), "Pixel", "OK rasterizes the type layer");
    assert_eq!(past(&app), before.0 + 2, "rasterize, then the patch");
    assert_eq!(patch_params(&app), vec![json!({"offset": [60, 0], "mode": "destination", "target": "pixels"})]);
    let [x, y, w, h] = before.1;
    assert_eq!(selection_bounds(&app), [x + 60, y, w, h], "the patch ran on the rasterized layer");
    assert!(app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().is_some());
    // One undo takes the patch back and keeps the rasterized layer.
    app.session.undo();
    assert_eq!(active_content(&app), "Pixel");
    assert_eq!(selection_bounds(&app), [x, y, w, h]);
    app.session.undo();
    assert_eq!(active_content(&app), "Type");
}

/// Done when 1: Cancel leaves the Type layer, the selection and the History exactly as they were.
#[test]
fn cancel_changes_nothing() {
    let mut app = app("Type");
    let before = (past(&app), app.session.active().unwrap().doc.clone());
    drag_patch(&mut app);
    let id = prompt(&app).expect("the prompt");
    let ctx = app.ctx().clone();
    crate::dialogs::cancel(&mut app, &ctx, id).unwrap();
    assert!(prompt(&app).is_none());
    assert!(app.framework.dialogs.is_empty());
    assert_eq!(past(&app), before.0);
    assert_eq!(*app.session.active().unwrap().doc, *before.1, "the document is untouched");
    assert_eq!(active_content(&app), "Type");
    assert!(patch_params(&app).is_empty());
}

/// Done when 2: a lasso drag with the Patch tool on a Type layer only makes the selection.
#[test]
fn a_lasso_drag_makes_the_selection_without_asking() {
    let mut app = app("Type");
    let m = Modifiers::NONE;
    // Outside the selection, so this is a lasso and not a patch drag.
    tool_event(&mut app, ToolEvent::Down { x: 120.0, y: 80.0, pressure: 1.0 }, m);
    for [x, y] in [[180.0, 80.0], [180.0, 110.0], [150.0, 110.0]] {
        tool_event(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
    }
    tool_event(&mut app, ToolEvent::Up { x: 150.0, y: 110.0 }, m);
    assert!(prompt(&app).is_none(), "the outline makes a selection, it does not paint");
    assert!(patch_params(&app).is_empty());
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let sel = app.session.active().unwrap().doc.selection.clone().unwrap();
    assert!(sel.sample_channel(160, 95, 0) > 0.0, "the lasso replaced the selection");
    assert_eq!(sel.sample_channel(55, 50, 0), 0.0, "the rectangle is gone");
    assert_eq!(active_content(&app), "Type");
}

/// Not measured: a patch of the type layer's own mask paints no pixels, so it still runs unasked.
#[test]
fn a_type_layers_mask_still_takes_the_patch() {
    let mut app = app("Type");
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.ui.mask_target = true;
    app.sync_mask_targets();
    let before = past(&app);
    drag_patch(&mut app);
    assert!(prompt(&app).is_none(), "the mask needs no rasterizing");
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(patch_params(&app), vec![json!({"offset": [60, 0], "mode": "source", "target": "mask"})]);
    assert_eq!(past(&app), before + 1, "the mask patch is one step");
    assert_eq!(active_content(&app), "Type", "the layer stays editable");
}

/// Not measured: Content-Aware mode on a Type layer keeps today's status-bar refusal.
#[test]
fn content_aware_mode_on_a_type_layer_still_refuses() {
    let mut app = app("Type");
    app.ui.tool_options.patch_content_aware = true;
    let before = past(&app);
    drag_patch(&mut app);
    assert!(prompt(&app).is_none(), "Content-Aware mode on a type layer was not measured");
    assert!(app.ui.status_error, "the refusal is still reported: {}", app.ui.status);
    assert!(app.ui.status.contains("Type layer"), "unexpected refusal: {}", app.ui.status);
    assert!(patch_params(&app).is_empty());
    assert_eq!(past(&app), before);
    assert_eq!(active_content(&app), "Type");
}

/// Not measured: a Shape, Smart Object or fill layer keeps today's status-bar refusal, no prompt. A
/// fill layer carries a mask, so the engine's own check lets it through and its surface lookup is
/// what says "not a pixel layer"; the wording differs from the other two and is pinned as it is.
#[test]
fn the_other_layer_kinds_still_refuse() {
    for (kind, words) in
        [("Shape", "a Shape layer, not a pixel layer"), ("Smart Object", "a Smart Object layer, not a pixel layer"), ("Fill", "not a pixel layer")]
    {
        let mut app = app(kind);
        let before = past(&app);
        drag_patch(&mut app);
        assert!(prompt(&app).is_none(), "{kind}: no prompt was measured for it");
        assert!(app.ui.status_error, "{kind}: the refusal is still reported: {}", app.ui.status);
        assert!(app.ui.status.contains(words), "{kind}: unexpected refusal: {}", app.ui.status);
        assert!(patch_params(&app).is_empty(), "{kind}");
        assert_eq!(past(&app), before, "{kind}");
        assert_eq!(active_content(&app), kind, "{kind} stays editable");
    }
}

/// Review round: a Type layer that is hidden, or sits in a hidden group, keeps today's refusal. The
/// engine never writes there, so a prompt would only let OK destroy the text for no patch.
#[test]
fn a_hidden_type_layer_is_refused_without_a_prompt() {
    for group in [false, true] {
        let mut app = app("Type");
        let text = app.session.active().unwrap().active_layer.unwrap();
        if group {
            let wrapped = app.run("layer.new.group", json!({"name": "Wrapped"})).unwrap()["layer"].as_u64().unwrap();
            app.run("layer.moveTo", json!({"layer": text.0, "target": wrapped, "position": "into"})).unwrap();
            app.run("layer.hideLayers", json!({"layer": wrapped})).unwrap();
        } else {
            app.run("layer.hideLayers", json!({"layer": text.0})).unwrap();
        }
        let what = if group { "type layer in a hidden group" } else { "hidden type layer" };
        assert_eq!(app.session.active().unwrap().active_layer, Some(text), "{what}: the type layer is the active one");
        let before = (past(&app), app.session.active().unwrap().doc.clone());
        drag_patch(&mut app);
        assert!(prompt(&app).is_none(), "{what}: no prompt, the patch could never land there");
        assert!(app.ui.status_error, "{what}: the refusal stands: {}", app.ui.status);
        assert!(patch_params(&app).is_empty(), "{what}");
        assert_eq!(past(&app), before.0, "{what}");
        assert_eq!(*app.session.active().unwrap().doc, *before.1, "{what}: the document is untouched");
        assert_eq!(active_content(&app), "Type", "{what} stays editable");
    }
}

/// Review round: OK applies the patch to the layer the question was about. The params name no layer,
/// so a different layer that is active when OK comes must not take the pixels.
#[test]
fn ok_after_the_active_layer_changed_refuses_the_patch() {
    let mut app = app("Type");
    let text = app.session.active().unwrap().active_layer.unwrap();
    drag_patch(&mut app);
    let id = prompt(&app).expect("the prompt");
    // Another layer becomes the active one while the question is up.
    let elsewhere = app.run("layer.new.layer", json!({"name": "Elsewhere"})).unwrap()["layer"].as_u64().unwrap();
    let before = past(&app);
    answer(&mut app, id);
    assert_eq!(app.session.active().unwrap().active_layer, Some(photocraft_doc::LayerId(elsewhere)), "the new layer is the active one");
    assert!(app.ui.status_error, "OK says why the patch did not run: {}", app.ui.status);
    assert!(patch_params(&app).is_empty(), "the patch did not run on the layer that is active now");
    assert_eq!(past(&app), before + 1, "the layer the question was about is still rasterized");
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(text).unwrap().content.kind_name(), "Pixel", "the stored layer took the rasterize");
}

/// Review round: a document read from a file keeps the layer ids stored in it, so two open
/// documents can hold the same number. OK must not rasterize or patch in another document than the
/// one the question was opened for.
#[test]
fn ok_after_another_document_became_active_refuses_the_patch() {
    let mut app = app("Type");
    let text = app.session.active().unwrap().active_layer.unwrap();
    let mine = app.session.active_index().unwrap();
    drag_patch(&mut app);
    let id = prompt(&app).expect("the prompt");
    // A second document made from this one's bytes: its layers keep the ids stored in the file, so
    // the same number names a Type layer there too, and that layer is the active one there.
    let bytes = photocraft_format::save_to_bytes(&app.session.active().unwrap().doc, &Default::default()).unwrap();
    let twin = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(twin.layer(text).map(|l| l.content.kind_name()), Some("Type"), "both documents hold a Type layer {}", text.0);
    app.session.add_document(twin, None);
    let other = app.session.active_index().unwrap();
    assert_ne!(other, mine, "the second document is the active one now");
    assert_eq!(app.session.documents()[other].active_layer, Some(text), "the other document's active layer carries the same id");
    let (before_other, before_mine) = (app.session.documents()[other].revision, app.session.documents()[mine].revision);
    answer(&mut app, id);
    assert!(app.ui.status_error, "OK says why nothing ran: {}", app.ui.status);
    assert!(patch_params(&app).is_empty(), "no patch ran in the other document");
    // Read both documents live, after the answer: a copy taken before it would show the old document
    // however much the answer changed it.
    assert_eq!(app.session.documents()[other].revision, before_other, "the other document is untouched");
    assert_eq!(app.session.documents()[other].doc.layer(text).unwrap().content.kind_name(), "Type", "the other document keeps its text");
    assert_eq!(app.session.documents()[mine].revision, before_mine, "the document the question was about is untouched too");
    assert_eq!(app.session.documents()[mine].doc.layer(text).unwrap().content.kind_name(), "Type", "and keeps its text");
}

/// Pin, passes before and after the change: a locked Type layer cannot be rasterized either, so the
/// drag keeps today's refusal and no prompt opens.
#[test]
fn a_locked_type_layer_still_refuses() {
    let mut app = app("Type");
    app.run("layer.lockLayers", json!({"pixels": true})).unwrap();
    let before = (past(&app), app.session.active().unwrap().doc.clone());
    drag_patch(&mut app);
    assert!(prompt(&app).is_none(), "a locked layer gets no prompt");
    assert!(app.ui.status_error, "the refusal stands: {}", app.ui.status);
    assert!(patch_params(&app).is_empty());
    assert_eq!(past(&app), before.0);
    assert_eq!(*app.session.active().unwrap().doc, *before.1, "the document is untouched");
    assert_eq!(active_content(&app), "Type");
}

/// A pixel layer is unaffected: the patch applies with no question asked.
#[test]
fn a_pixel_layer_patch_still_applies_without_asking() {
    let mut app = app("Pixel");
    let before = past(&app);
    drag_patch(&mut app);
    assert!(prompt(&app).is_none());
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(patch_params(&app), vec![json!({"offset": [60, 0], "mode": "source", "target": "pixels"})]);
    assert_eq!(past(&app), before + 1);
}

use photocraft_doc::{Fill, LayerContent, LayerId};
use serde_json::json;

use super::*;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "depth": 16})).unwrap();
    app
}

fn pattern_id(app: &PhotocraftApp, name: &str) -> String {
    app.session.patterns.items.iter().find(|p| p.name == name).unwrap().id.clone()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn set(app: &mut PhotocraftApp, id: DialogId, field: &str, value: Value) {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, field, value).unwrap();
}

fn current(app: &PhotocraftApp) -> PatternFillOptions {
    app.framework.dialogs.get::<PatternFillDialog>().unwrap().current().clone()
}

fn confirm(app: &mut PhotocraftApp, id: DialogId) {
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(app, &ctx, id).unwrap();
}

fn cancel(app: &mut PhotocraftApp, id: DialogId) {
    let ctx = app.ctx().clone();
    crate::dialogs::cancel(app, &ctx, id).unwrap();
}

/// A refused OK reports in the status bar and changes nothing.
fn refused(app: &mut PhotocraftApp, id: DialogId) -> bool {
    app.ui.status_error = false;
    confirm(app, id);
    app.ui.status_error
}

#[test]
fn new_pattern_fill_opens_the_dialog_previews_and_ok_is_one_step() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let before = app.session.active().unwrap().doc.clone();
    let count = steps(&app);
    crate::menus::invoke(&mut app, &ctx, cmds::CREATE, json!({})).unwrap();
    let open = app.framework.dialogs.open_dialogs();
    assert_eq!(open[0].key.as_deref(), Some("patternFill"));
    let dialog = open[0].id;
    let o = current(&app);
    assert_eq!((o.scale, o.angle, o.link), (100.0, 0.0, true));
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "no layer until OK");
    let bricks = pattern_id(&app, "Bricks");
    for (k, v) in [("pattern", json!(bricks)), ("scale", json!(200.0)), ("angle", json!(45.0)), ("link", json!(false))] {
        set(&mut app, dialog, k, v);
    }
    let (shown, key) = display_doc(&mut app, 0).unwrap();
    let (again, again_key) = display_doc(&mut app, 0).unwrap();
    assert!(Arc::ptr_eq(&shown, &again) && key == again_key, "the preview is cached");
    assert_eq!(shown.layers.len(), before.layers.len() + 1, "the preview shows the new layer");
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "previewing does not edit");
    assert_eq!(steps(&app), count);
    confirm(&mut app, dialog);
    assert_eq!(steps(&app), count + 1);
    let st = app.session.active().unwrap();
    let LayerContent::Fill(Fill::Pattern { id, scale, angle, link, .. }) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else {
        panic!("no pattern fill")
    };
    assert_eq!((id.as_str(), *scale, *angle, *link), (bricks.as_str(), 2.0, 45.0, false));
    assert!(display_doc(&mut app, 0).is_none() && app.pattern_fill.session.is_none());
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(*app.session.active().unwrap().doc, *before);
}

#[test]
fn cancelling_a_new_pattern_fill_adds_nothing() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let before = app.session.active().unwrap().doc.clone();
    let count = steps(&app);
    let dialog = open_new(&mut app).unwrap();
    set(&mut app, dialog, "scale", json!(300.0));
    assert!(display_doc(&mut app, 0).is_some());
    cancel(&mut app, dialog);
    assert!(display_doc(&mut app, 0).is_none() && app.pattern_fill.session.is_none());
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(steps(&app), count);
    // Explicit params (Actions, CLI, MCP) still create the layer directly.
    let r = crate::menus::invoke(&mut app, &ctx, cmds::CREATE, json!({"pattern": "Grid"})).unwrap();
    assert!(r["layer"].as_u64().is_some());
    // No document: an error, not a dialog or a panic.
    let mut empty = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = empty.ctx().clone();
    assert!(crate::menus::invoke(&mut empty, &ctx, cmds::CREATE, json!({})).is_err());
    assert!(!empty.framework.dialogs.is_open::<PatternFillDialog>());
}

#[test]
fn content_options_edit_the_layer_and_cancel_restores() {
    let mut app = app();
    let id = app.run(cmds::CREATE, json!({"pattern": "Checkerboard", "scale": 150, "angle": 10})).unwrap()["layer"].as_u64().unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let count = steps(&app);
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, "layer.layerContentOptions", Value::Null).unwrap();
    let dialog = app.framework.dialogs.open_dialogs()[0].id;
    let o = current(&app);
    assert_eq!((o.pattern, o.scale, o.angle), (pattern_id(&app, "Checkerboard"), 150.0, 10.0));
    assert_eq!(app.pattern_fill.session.as_ref().and_then(|s| s.layer()), Some(LayerId(id)));
    // Unchanged: no preview and OK is no History step.
    assert!(display_doc(&mut app, 0).is_none());
    confirm(&mut app, dialog);
    assert_eq!(steps(&app), count);
    // Changed then cancelled: previewed, but the layer keeps its fill.
    let dialog = open(&mut app).unwrap();
    let dots = pattern_id(&app, "Dots");
    set(&mut app, dialog, "pattern", json!(dots));
    set(&mut app, dialog, "phase", json!([5.0, 5.0]));
    let (shown, _) = display_doc(&mut app, 0).unwrap();
    assert!(matches!(&shown.layer(LayerId(id)).unwrap().content, LayerContent::Fill(Fill::Pattern { phase, .. }) if *phase == (5.0, 5.0)));
    cancel(&mut app, dialog);
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert!(display_doc(&mut app, 0).is_none());
    // A stale dialog never edits another document.
    let dialog = open(&mut app).unwrap();
    set(&mut app, dialog, "scale", json!(50.0));
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    assert!(display_doc(&mut app, 1).is_none());
    assert!(refused(&mut app, dialog), "the status bar reports the stale document");
    assert!(Arc::ptr_eq(&before, &app.session.documents().first().unwrap().doc), "the stale document is unchanged");
    // Gradient and Solid Color fills keep their own editors.
    app.run("layer.newFillLayer.gradient", json!({})).unwrap();
    assert!(open(&mut app).is_none());
}

#[test]
fn an_imported_out_of_range_fill_keeps_its_fields_when_the_phase_changes() {
    let mut app = app();
    let id = LayerId(app.run(cmds::CREATE, json!({"pattern": "Checkerboard"})).unwrap()["layer"].as_u64().unwrap());
    // A PSD may carry values outside the dialog's ranges; opening never rewrites them.
    {
        let st = app.session.active_mut().unwrap();
        let doc = Arc::make_mut(st.doc_mut());
        let LayerContent::Fill(Fill::Pattern { name, id: pattern, .. }) = &doc.layer(id).unwrap().content else { panic!() };
        let (name, pattern) = (name.clone(), pattern.clone());
        doc.layer_mut(id).unwrap().content =
            LayerContent::Fill(Fill::Pattern { name, id: pattern, scale: 20.0, angle: 300.0, link: true, phase: (2.5, -3.25) });
    }
    let before = app.session.active().unwrap().doc.clone();
    let count = steps(&app);
    let dialog = open(&mut app).unwrap();
    assert_eq!((current(&app).scale, current(&app).angle), (2000.0, 300.0));
    // Unchanged OK is no History step and rewrites nothing.
    confirm(&mut app, dialog);
    assert_eq!(steps(&app), count);
    assert_eq!(*app.session.active().unwrap().doc, *before);
    // Editing the phase sends only the phase: the imported values survive, and undo restores them.
    let dialog = open(&mut app).unwrap();
    set(&mut app, dialog, "phase", json!([5.0, 5.0]));
    confirm(&mut app, dialog);
    let st = app.session.active().unwrap();
    let LayerContent::Fill(Fill::Pattern { scale, angle, phase, .. }) = &st.doc.layer(id).unwrap().content else { panic!() };
    assert_eq!((*scale, *angle, *phase), (20.0, 300.0, (5.0, 5.0)));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(*app.session.active().unwrap().doc, *before);
}

#[test]
fn a_canvas_drag_needs_a_linked_editable_pattern_target() {
    let mut app = app();
    let id = app.run(cmds::CREATE, json!({"pattern": "Checkerboard"})).unwrap()["layer"].as_u64().unwrap();
    let dialog = open(&mut app).unwrap();
    assert!(drag_target(&app).is_some());
    // Unlinked: the pattern does not follow the pointer.
    set(&mut app, dialog, "link", json!(false));
    assert!(drag_target(&app).is_none());
    set(&mut app, dialog, "link", json!(true));
    assert!(drag_target(&app).is_some());
    // A wrong kind (the Background) is not a Pattern Fill.
    let background = app.session.active().unwrap().doc.layers[0].id;
    let session = app.pattern_fill.session.as_mut().unwrap();
    let edited = session.target;
    session.target = Target::Edit(background);
    assert!(drag_target(&app).is_none());
    app.pattern_fill.session.as_mut().unwrap().target = edited;
    // A pixel-locked fill cannot move.
    app.run("layer.lockLayers", json!({"pixels": true})).unwrap();
    assert!(drag_target(&app).is_none());
    app.run("layer.lockLayers", json!({"pixels": false})).unwrap();
    // A deleted layer...
    app.run("layer.delete", json!({"layer": id})).unwrap();
    assert!(drag_target(&app).is_none());
    // ...and a document switched away from are stale.
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    assert!(drag_target(&app).is_none());
    // A new fill may be inserted above a locked base: creation is not the edit's lock check.
    open_new(&mut app).unwrap();
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    assert!(drag_target(&app).is_some());
}

#[test]
fn a_pattern_drag_uses_the_total_travel_and_keeps_external_edits() {
    let mut app = app();
    app.run(cmds::CREATE, json!({"pattern": "Checkerboard"})).unwrap();
    let dialog = open(&mut app).unwrap();
    let phase = |app: &PhotocraftApp| current(app).phase;
    set(&mut app, dialog, "phase", json!([2.5, -3.25]));
    let mut g = press(&app, [0.0, 0.0]).unwrap();
    assert_eq!(phase(&app), [2.5, -3.25], "a press leaves the phase");
    assert!(drag(&mut app, &mut g, [3.0, 4.0]));
    assert!(drag(&mut app, &mut g, [7.0, 4.0]));
    assert_eq!(phase(&app), [9.5, 0.75], "the total since the press, not each move");
    assert!(drag(&mut app, &mut g, [0.0, 0.0]));
    assert_eq!(phase(&app), [2.5, -3.25], "a drag back restores the phase exactly");
    // An external phase set under the press is kept, and the drag continues from it.
    assert!(drag(&mut app, &mut g, [10.0, 0.0]));
    set(&mut app, dialog, "phase", json!([0.0, 0.0]));
    assert!(drag(&mut app, &mut g, [10.0, 0.0]));
    assert_eq!(phase(&app), [0.0, 0.0]);
    assert!(drag(&mut app, &mut g, [12.0, 0.0]));
    assert_eq!(phase(&app), [2.0, 0.0]);
    // The same at the bound: the rebase continues instead of stalling.
    set(&mut app, dialog, "phase", json!([-MAX_PHASE, 0.0]));
    let mut g = press(&app, [0.0, 0.0]).unwrap();
    assert!(drag(&mut app, &mut g, [100.0, 0.0]));
    assert_eq!(phase(&app), [-MAX_PHASE + 100.0, 0.0]);
    set(&mut app, dialog, "phase", json!([-MAX_PHASE, 0.0]));
    assert!(drag(&mut app, &mut g, [100.0, 0.0]));
    assert_eq!(phase(&app), [-MAX_PHASE, 0.0], "the external edit is kept");
    assert!(drag(&mut app, &mut g, [150.0, 0.0]));
    assert_eq!(phase(&app), [-MAX_PHASE + 50.0, 0.0], "and the drag continues from it");
    // A huge travel clamps to what the engine accepts; a release past the bound stays on it.
    set(&mut app, dialog, "phase", json!([0.0, 0.0]));
    let mut g = press(&app, [0.0, 0.0]).unwrap();
    assert!(drag(&mut app, &mut g, [1.0e12, -1.0e12]));
    assert_eq!(phase(&app), [MAX_PHASE, -MAX_PHASE]);
    set(&mut app, dialog, "phase", json!([0.0, 0.0]));
    let mut g = press(&app, [0.0, 0.0]).unwrap();
    assert!(drag(&mut app, &mut g, [MAX_PHASE + 1000.0, 0.0]));
    release(&mut app, g, [MAX_PHASE + 500.0, 0.0]);
    assert_eq!(phase(&app), [MAX_PHASE, 0.0]);
    // A click with no drag changes nothing.
    set(&mut app, dialog, "phase", json!([1.0, 2.0]));
    let g = press(&app, [5.0, 5.0]).unwrap();
    release(&mut app, g, [5.0, 5.0]);
    assert_eq!(phase(&app), [1.0, 2.0]);
    // A changed target ends the gesture without a write.
    let mut g = press(&app, [0.0, 0.0]).unwrap();
    set(&mut app, dialog, "link", json!(false));
    assert!(!drag(&mut app, &mut g, [5.0, 5.0]));
    assert_eq!(phase(&app), [1.0, 2.0]);
}

#[test]
fn a_new_fill_dialog_rejects_a_changed_insertion_layer() {
    let mut app = app();
    // A selected empty pixel layer is what the new fill would go above.
    app.run("layer.new.layer", json!({})).unwrap();
    let insert = app.session.active().unwrap().active_layer.unwrap();
    let background = app.session.active().unwrap().doc.layers[0].id;
    let dialog = open_new(&mut app).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let count = steps(&app);
    set(&mut app, dialog, "phase", json!([2.0, -1.0]));
    assert!(display_doc(&mut app, 0).is_some(), "the preview is live");
    // Selecting another layer under the dialog changes what the command would insert above.
    app.run("layer.select", json!({"layer": background.0})).unwrap();
    assert!(drag_target(&app).is_none(), "a changed insertion layer is stale");
    assert!(display_doc(&mut app, 0).is_none(), "no preview on the wrong insertion layer");
    assert!(refused(&mut app, dialog));
    let st = app.session.active().unwrap();
    assert_eq!(*st.doc, *before, "a stale OK creates nothing");
    assert_eq!(st.history.entries().len(), count, "and records no History step");
    assert_eq!(st.active_layer, Some(background), "the selected layer stays selected");
    assert!(matches!(&st.doc.layer(insert).unwrap().content, LayerContent::Raster(_)));
    // Reopening at the new active layer is a valid insertion context.
    let dialog = open_new(&mut app).unwrap();
    assert!(drag_target(&app).is_some());
    assert!(display_doc(&mut app, 0).is_some());
    confirm(&mut app, dialog);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.entries().len(), count + 1);
    assert_eq!(st.doc.layers.len(), before.layers.len() + 1);
    // No active layer is a valid insertion context too.
    let layers = st.doc.layers.len();
    app.session.active_mut().unwrap().set_active_layer(None);
    let dialog = open_new(&mut app).unwrap();
    assert!(drag_target(&app).is_some());
    assert!(display_doc(&mut app, 0).is_some());
    confirm(&mut app, dialog);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1);
}

#[test]
fn a_replaced_or_deleted_edit_target_is_not_previewed_or_committed() {
    let mut app = app();
    let id = LayerId(app.run(cmds::CREATE, json!({"pattern": "Checkerboard"})).unwrap()["layer"].as_u64().unwrap());
    let dialog = open(&mut app).unwrap();
    set(&mut app, dialog, "phase", json!([3.0, 4.0]));
    assert!(display_doc(&mut app, 0).is_some());
    let count = steps(&app);
    // Rasterizing replaces the Pattern Fill with pixels: the dialog's target no longer exists.
    app.run("layer.rasterize.layer", json!({"layer": id})).unwrap();
    let replaced = app.session.active().unwrap().doc.clone();
    assert!(drag_target(&app).is_none());
    assert!(display_doc(&mut app, 0).is_none());
    refused(&mut app, dialog);
    assert_eq!(*app.session.active().unwrap().doc, *replaced);
    assert_eq!(steps(&app), count + 1, "a stale OK adds no History step");
    // Deleting the fill is stale too.
    let id = LayerId(app.run(cmds::CREATE, json!({"pattern": "Checkerboard"})).unwrap()["layer"].as_u64().unwrap());
    let dialog = open(&mut app).unwrap();
    set(&mut app, dialog, "phase", json!([5.0, 6.0]));
    app.run("layer.delete", json!({"layer": id})).unwrap();
    let deleted = app.session.active().unwrap().doc.clone();
    let count = steps(&app);
    assert!(drag_target(&app).is_none());
    assert!(display_doc(&mut app, 0).is_none());
    refused(&mut app, dialog);
    assert_eq!(*app.session.active().unwrap().doc, *deleted);
    assert_eq!(steps(&app), count, "a stale OK adds no History step");
}

/// The picker paints its swatches by texture id and doesn't ask the texture cache again, so the dialog owns them:
/// emptying the cache (as it frees textures once it holds more than its capacity) leaves every swatch, and closing
/// the dialog frees them (#16252).
#[test]
fn the_open_dialog_keeps_its_swatch_textures_when_the_texture_cache_lets_go() {
    use craft::ui::primitives::textures::TextureCache;
    let mut app = app();
    let ctx = app.ctx().clone();
    let allocated = |ctx: &egui::Context| ctx.tex_manager().read().num_allocated();
    TextureCache::clear(&ctx);
    let before = allocated(&ctx);
    let patterns = app.session.patterns.items.len();
    assert!(patterns > 0, "the library has patterns");
    crate::menus::invoke(&mut app, &ctx, cmds::CREATE, json!({})).unwrap();
    let dialog = app.framework.dialogs.open_dialogs()[0].id;
    TextureCache::clear(&ctx);
    assert!(allocated(&ctx) >= before + patterns, "the {patterns} swatches outlive the cache");
    cancel(&mut app, dialog);
    assert_eq!(allocated(&ctx), before, "closing the dialog frees them");
}

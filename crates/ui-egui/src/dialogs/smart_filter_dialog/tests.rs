use std::sync::Arc;

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog};
use photocraft_doc::{SmartFilter, SmartObject};

use super::*;

/// The newest open dialog.
pub(crate) fn newest(app: &PhotocraftApp) -> DialogId {
    app.framework.dialogs.open_dialogs().pop().expect("an open dialog").id
}

/// How many dialogs are open.
pub(crate) fn open_count(app: &PhotocraftApp) -> usize {
    app.framework.dialogs.open_dialogs().len()
}

/// Sets `field` of dialog `id` as `ui.dialog.set` does.
pub(crate) fn set(app: &mut PhotocraftApp, id: DialogId, field: &str, value: Value) {
    let ctx = app.ctx().clone();
    for (field, value) in crate::control::dialogs::map_request(app, id, field, value) {
        app.framework.dialogs.set_json_field(&ctx, id, &field, value).unwrap();
    }
}

/// OK on dialog `id` through the dispatcher; a refused OK leaves the dialog open.
pub(crate) fn confirm(app: &mut PhotocraftApp, id: DialogId) {
    let _ = crate::dialogs::testing::confirm(app, id);
}

/// Cancel on dialog `id` through the dispatcher.
pub(crate) fn cancel(app: &mut PhotocraftApp, id: DialogId) {
    let _ = crate::dialogs::testing::cancel(app, id);
}

/// One frame of the app's dialogs.
pub(crate) fn pass(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
    crate::testing::run_ui(&ctx, input, |ui| crate::dialogs::run(app, ui));
}

/// The values of the hosted dialog `id`.
fn fields_of(app: &PhotocraftApp, id: DialogId) -> DialogFields {
    app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("a hosted dialog").fields().clone()
}

/// A document with one Smart Object holding Box Blur and Smart Sharpen, wrapped in an app that renders on the calling thread.
pub(crate) fn app(depth: u32, mode: &str) -> (Box<PhotocraftApp>, LayerId) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth, "mode": mode})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Smart Object"})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#305080"})).unwrap();
    s.execute("select.rect", json!({"x":0,"y":0,"width":32,"height":48})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#e09040"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let layer = s.active().unwrap().active_layer.unwrap();
    s.execute("filter.blur.boxBlur", json!({"radius": 4})).unwrap();
    s.execute("filter.sharpen.smartSharpen", json!({"amount": 125, "radius": 2})).unwrap();
    let mut app = PhotocraftApp::new(s, Default::default());
    app.background_jobs = false;
    (app, layer)
}

/// The Smart Object content of `layer` in the active document.
pub(crate) fn smart(app: &PhotocraftApp, layer: LayerId) -> &SmartObject {
    let LayerContent::Smart(sm) = &app.session.active().unwrap().doc.layer(layer).unwrap().content else { panic!("smart") };
    sm
}

/// A copy of the Smart Object's filters, for comparing before and after an edit.
fn filters(app: &PhotocraftApp, layer: LayerId) -> Vec<SmartFilter> {
    smart(app, layer).smart_filters.clone()
}

/// Closing a document cancels its open Smart Filter dialog, and a document reopened from the same data never answers it.
#[test]
fn closing_the_document_cancels_its_open_smart_filter_dialog() {
    let (mut app, layer) = app(8, "rgb");
    let saved = app.session.active().unwrap().doc.as_ref().clone();
    // The open document is one the app's views already know, as after any frame.
    app.sync_views();
    open(&mut app, layer, 0).unwrap();
    assert_eq!(open_count(&app), 1);
    app.run("file.close", json!({"save": false})).unwrap();
    assert_eq!(open_count(&app), 0);
    app.session.add_document(saved, None);
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    pass(&mut app);
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}

/// Blending edits one filter in a single history step, opacity is entered as a percentage, and undo/redo restore it.
#[test]
fn blending_edits_one_filter_with_percent_opacity_and_undo_redo() {
    let (mut app, layer) = app(16, "rgb");
    let original = filters(&app, layer);
    let source = photocraft_engine::smart_cmds::contents_of(&app.session.active().unwrap().doc, layer).unwrap();
    for blend in ["multiply", "luminosity"] {
        let steps = app.session.active().unwrap().history.past_len();
        let id = open(&mut app, layer, 0).unwrap();
        set(&mut app, id, "opacity", json!(1.0));
        set(&mut app, id, "blend", json!(blend));
        let target = target(&app, layer, 0).unwrap();
        let (cmd, params) = blend_command(&app, &target, &fields_of(&app, id)).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let expected = crate::filter_dialog::preview_document(&before, Some(layer), &cmd, &params, 1).unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
        confirm(&mut app, id);
        let changed = filters(&app, layer);
        assert!((changed[0].opacity - 0.01).abs() < 1e-6);
        assert_eq!((changed[0].blend).key().to_string(), blend);
        assert_eq!(serde_json::to_value(&changed[1]).unwrap(), serde_json::to_value(&original[1]).unwrap());
        assert_eq!(photocraft_engine::smart_cmds::contents_of(&app.session.active().unwrap().doc, layer).unwrap(), source);
        assert_eq!(app.session.active().unwrap().doc.layer(layer).unwrap().blend, BlendMode::Normal);
        assert_eq!(
            smart(&app, layer).cache.as_ref().unwrap().read_region(before.bounds()),
            expected.layer(layer).unwrap().surface().unwrap().read_region(expected.bounds())
        );
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(serde_json::to_value(filters(&app, layer)).unwrap(), serde_json::to_value(&original).unwrap());
        app.run("edit.redo", json!({})).unwrap();
        let id = open(&mut app, layer, 0).unwrap();
        assert!((fields_of(&app, id).number("opacity").unwrap() - 1.0).abs() < 1e-3, "the dialog shows the percent");
        cancel(&mut app, id);
        app.run("edit.undo", json!({})).unwrap();
    }
}

/// The live preview follows the open dialog's values, and Cancel drops it and leaves document and history alone.
#[test]
fn preview_toggle_and_cancel_leave_document_and_history_untouched() {
    let (mut app, layer) = app(8, "rgb");
    let st = app.session.active().unwrap();
    let before = st.doc.clone();
    let steps = st.history.past_len();
    let id = open(&mut app, layer, 0).unwrap();
    set(&mut app, id, "opacity", json!(1));
    pass(&mut app);
    let ctx = app.ctx().clone();
    let idx = app.session.active_index().unwrap();
    assert!(crate::canvas::ensure_filter_preview(&mut app, idx, &ctx).is_some());
    assert_eq!(app.filter_preview.as_ref().unwrap().key.params["opacity"], json!(0.01));
    cancel(&mut app, id);
    pass(&mut app);
    assert!(app.filter_preview.is_none());
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}

/// A held OK refuses when the document or filter has since changed, or been switched away from.
#[test]
fn stale_filter_dialogs_cannot_edit_a_switched_or_changed_target() {
    for command in ["file.new", "edit.undo", "layer.smartFilter.delete", "layer.smartFilter.move"] {
        let (mut app, layer) = app(8, "rgb");
        let id = open(&mut app, layer, 0).unwrap();
        let params = match command {
            "file.new" => json!({"width": 16,"height":16}),
            "edit.undo" => json!({}),
            "layer.smartFilter.move" => json!({"layer":layer.0,"index":0,"to":1}),
            _ => json!({"layer":layer.0,"index":0}),
        };
        app.run(command, params).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        confirm(&mut app, id);
        assert_eq!(open_count(&app), 1, "{command}: the refused dialog stays open");
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "{command}");
    }
}

/// A reduced-size preview scales the filters' pixel parameters and the shared mask, and still contains the same number of filters.
#[test]
fn reduced_preview_scales_placement_stack_and_mask_without_adding_filters() {
    let (mut app, layer) = app(16, "rgb");
    // A half-visible shared mask, different from the layer mask.
    let st = app.session.active_mut().unwrap();
    let doc = Arc::make_mut(st.doc_mut());
    let LayerContent::Smart(sm) = &mut doc.layer_mut(layer).unwrap().content else { panic!() };
    let mut mask = photocraft_doc::LayerMask::reveal_all();
    mask.surface.fill_rect(photocraft_geom::Rect::new(32, 0, 64, 48), &[0.0]);
    sm.filter_mask = Some(mask);
    let id = open(&mut app, layer, 0).unwrap();
    set(&mut app, id, "opacity", json!(50));
    let target = target(&app, layer, 0).unwrap();
    let (command, params) = blend_command(&app, &target, &fields_of(&app, id)).unwrap();
    let original = app.session.active().unwrap().doc.clone();
    let preview = crate::filter_dialog::preview_document(&original, Some(layer), &command, &params, 2).unwrap();
    assert_eq!(preview.size.width, 32);
    let LayerContent::Smart(sm) = &preview.layer(layer).unwrap().content else { panic!() };
    assert_eq!(sm.smart_filters.len(), 2);
    assert_eq!(sm.smart_filters[0].params["radius"], json!(2.0));
    assert_eq!(sm.smart_filters[1].params["radius"], json!(1.0));
    assert_eq!(sm.filter_mask.as_ref().unwrap().surface.pixel(17, 0)[0], 0.0);
    assert!(sm.cache.as_ref().unwrap().content_bounds().x1 <= 32);
    assert!(Arc::ptr_eq(&original, &app.session.active().unwrap().doc));
}

/// An edited blend and an Invert recorded earlier survive a save and reload of the native format.
#[test]
fn native_save_preserves_edited_blending_and_existing_invert() {
    let (mut app, layer) = app(8, "rgb");
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, "image.adjustments.invert", json!({})).unwrap();
    assert_eq!(smart(&app, layer).smart_filters.last().unwrap().command, "image.adjustments.invert");
    let id = open(&mut app, layer, 2).unwrap();
    set(&mut app, id, "opacity", json!(1));
    confirm(&mut app, id);
    let doc = &app.session.active().unwrap().doc;
    let loaded = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(doc, &Default::default()).unwrap()).unwrap();
    let LayerContent::Smart(sm) = &loaded.layers.last().unwrap().content else { panic!() };
    assert_eq!(serde_json::to_value(&sm.smart_filters).unwrap(), serde_json::to_value(filters(&app, layer)).unwrap());
}

/// Blending edits the filter the dialog opened on, whatever layer is selected and whether or not a layer mask is targeted.
#[test]
fn blending_does_not_follow_layer_selection_or_the_layer_mask_target() {
    let (mut app, layer) = app(8, "rgb");
    app.run("layer.new.layer", json!({"name":"Selected pixel layer"})).unwrap();
    let selected = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.layerMask.revealAll", json!({"layer":selected.0})).unwrap();
    app.ui.mask_target = true;
    let doc = app.session.active().unwrap().doc.clone();
    let mask = doc.layer(selected).unwrap().mask.as_ref().unwrap().surface.clone();
    let id = open(&mut app, layer, 0).unwrap();
    set(&mut app, id, "opacity", json!(25));
    pass(&mut app);
    let ctx = app.ctx().clone();
    let idx = app.session.active_index().unwrap();
    assert!(crate::canvas::ensure_filter_preview(&mut app, idx, &ctx).is_some());
    confirm(&mut app, id);
    assert_eq!(smart(&app, layer).smart_filters[0].opacity, 0.25);
    assert_eq!(app.session.active().unwrap().active_layer, Some(selected));
    assert_eq!(app.session.active().unwrap().doc.layer(selected).unwrap().mask.as_ref().unwrap().surface.generation(), mask.generation());
}

/// Blending an Invert smart filter keeps the translucent pixels' alpha at 8, 16 and 32 bits.
#[test]
fn invert_blending_keeps_transparency_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width":8,"height":8,"depth":depth})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("synthetic translucent pixels", |doc, active| {
            let bounds = doc.bounds();
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(bounds, &[0.2, 0.4, 0.6, 0.5]);
            Ok(())
        })
        .unwrap();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let layer = s.active().unwrap().active_layer.unwrap();
        let before = s.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().pixel(3, 3);
        s.execute("image.adjustments.invert", json!({})).unwrap();
        let mut app = PhotocraftApp::new(s, Default::default());
        app.background_jobs = false;
        let id = open(&mut app, layer, 0).unwrap();
        set(&mut app, id, "opacity", json!(1));
        confirm(&mut app, id);
        assert_eq!(smart(&app, layer).cache.as_ref().unwrap().pixel(3, 3)[3], before[3]);
    }
}

//! #12042: a right-click with the Horizontal Type tool away from text opens the generic layer
//! menu at the pointer, with the reference application's entries and greyed rules (the same rows
//! the Layers panel menu enables through the main menus). Over committed type the #7324 type
//! layer menu stays; while editing, the type edit menu stays. The Vertical Type and Type Mask
//! tools, and the menu with an active selection, are not measured: they keep today's no menu.

use serde_json::json;

use photocraft_doc::LayerContent;

use super::Tool;
use super::count_tests::{call, listed, right_click};
use super::tests::{labels, open_here, screen_of};
use super::{COUNT_MENU, MenuKind, available, entry_enabled, rows};
use crate::PhotocraftApp;

/// A 64 × 48 document: committed "Hello" at the left (no edit running) and a pixel layer "Ink"
/// active on top. Away from the glyphs: (30, 2); over them: (16, 16).
fn type_app() -> (Box<PhotocraftApp>, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.ui.tool = Tool::Type;
    app.run("type.create", json!({"text": "Hello", "size": 14, "x": 1, "y": 22})).unwrap();
    assert!(app.ui.text_edit.is_none(), "committed: no edit running");
    app.run("layer.new.layer", json!({"name": "Ink"})).unwrap();
    app.sync_views();
    let ctx = app.ctx().clone();
    (app, ctx)
}

/// The open menu, or panic with the app state.
fn open_menu(app: &PhotocraftApp) -> super::CanvasToolMenu {
    app.ui.canvas_tool_menu.clone().expect("the right-click opened a menu")
}

#[test]
fn off_text_right_click_opens_the_layer_menu_at_the_pointer() {
    let (mut app, ctx) = type_app();
    right_click(&mut app, 30.0, 2.0);
    let menu = open_menu(&app);
    assert_eq!(menu.tool, Tool::Type, "the menu is the Type tool's");
    assert!(matches!(menu.kind, MenuKind::Tool), "the generic layer menu, not a type menu: {:?}", menu.kind);
    assert_eq!(labels(rows(&menu)), labels(COUNT_MENU), "the reference application's rows in order");
    // Reachable over the control channel: `ui.inspect` lists the entries in the issue's order.
    assert_eq!(
        listed(&mut app, &ctx),
        [
            "layer.layerStyle.blendingOptions",
            "―",
            "layer.duplicate",
            "layer.delete",
            "layer.new.groupFromLayers",
            "―",
            "layer.smartObjects.convertToSmartObject",
            "layer.smartObjects.convertToLayers",
            "―",
            "layer.rasterize.layer",
            "layer.rasterize.layerStyle",
            "―",
            "edit.freeTransform",
            "―",
            "select.colorRange",
            "select.loadSelection",
            "select.reselect",
        ]
    );
    // The menu sits at the pointer: the screen point of document (30, 2).
    let at = screen_of(&app, [30.0, 2.0]);
    let (dx, dy) = (menu.pos[0] - at.x, menu.pos[1] - at.y);
    assert!(dx.abs() <= 0.01 && dy.abs() <= 0.01, "menu at {:?}, pointer at {at:?}", menu.pos);
}

#[test]
fn rows_are_greyed_as_measured_for_a_pixel_layer() {
    let (mut app, _ctx) = type_app();
    right_click(&mut app, 30.0, 2.0);
    let menu = open_menu(&app);
    assert_eq!(app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().name, "Ink");
    for id in rows(&menu).iter().flatten().map(|(_, id)| *id) {
        assert_eq!(entry_enabled(&app, &menu, id), crate::menus::is_enabled(&app, id), "{id}: as the Layers panel menu enables it");
    }
    // Measured with a pixel layer active and no selection: these four grey out...
    for id in ["layer.smartObjects.convertToLayers", "layer.rasterize.layer", "layer.rasterize.layerStyle", "select.reselect"] {
        assert!(!available(&app, &menu, id), "{id} is greyed as measured");
    }
    // ...and the rest of the list is live.
    for id in [
        "layer.layerStyle.blendingOptions",
        "layer.duplicate",
        "layer.delete",
        "layer.new.groupFromLayers",
        "layer.smartObjects.convertToSmartObject",
        "edit.freeTransform",
        "select.colorRange",
        "select.loadSelection",
    ] {
        assert!(available(&app, &menu, id), "{id} is enabled");
    }
}

#[test]
fn rasterize_layer_is_enabled_for_an_active_text_layer() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.ui.tool = Tool::Type;
    app.run("type.create", json!({"text": "Hello", "size": 14, "x": 1, "y": 22})).unwrap();
    app.sync_views();
    // The text layer is active; the right-click is away from its text.
    right_click(&mut app, 50.0, 40.0);
    let menu = open_menu(&app);
    assert!(entry_enabled(&app, &menu, "layer.rasterize.layer"), "Rasterize Layer is enabled for a text layer");
    assert!(!entry_enabled(&app, &menu, "layer.rasterize.layerStyle"), "no layer style to rasterize");
    assert!(!entry_enabled(&app, &menu, "layer.smartObjects.convertToLayers"), "not a smart object");
    assert!(!entry_enabled(&app, &menu, "select.reselect"), "no earlier selection");
}

#[test]
fn picking_a_row_runs_its_command_on_the_active_layer() {
    // Delete Layer removes the active pixel layer, not the type under the canvas.
    let (mut app, ctx) = type_app();
    let layers_before = app.session.active().unwrap().doc.layers.len();
    right_click(&mut app, 30.0, 2.0);
    let reply = call(&mut app, &ctx, "ui.context.choose", json!({"id": "layer.delete"}));
    assert_eq!(reply["ok"], true, "{reply}");
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers.len(), layers_before - 1, "the active layer is deleted");
    assert!(st.doc.layers.iter().all(|l| l.name != "Ink"), "the pixel layer is gone");
    assert!(st.doc.layers.iter().any(|l| matches!(l.content, LayerContent::Text(_))), "the type layer stays");

    // Reselect, after a selection was made and cleared, brings it back.
    let (mut app, ctx) = type_app();
    app.run("select.rect", json!({"x": 40, "y": 30, "width": 10, "height": 10})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    right_click(&mut app, 30.0, 2.0);
    let reply = call(&mut app, &ctx, "ui.context.choose", json!({"id": "select.reselect"}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(app.session.active().unwrap().doc.selection.is_some(), "the selection returns");
    assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("select.reselect"));

    // Color Range… runs its menu item: the dialog opens instead of editing anything.
    let (mut app, ctx) = type_app();
    let (revision, journal) = (app.session.active().unwrap().revision, app.session.journal.len());
    right_click(&mut app, 30.0, 2.0);
    let reply = call(&mut app, &ctx, "ui.context.choose", json!({"id": "select.colorRange"}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(!app.framework.dialogs.is_empty(), "the Color Range dialog opens");
    assert_eq!(app.session.active().unwrap().revision, revision, "opening the dialog edits nothing");
    assert_eq!(app.session.journal.len(), journal, "and runs no journaled command beyond the dialog");
}

#[test]
fn the_right_click_starts_no_text_edit_and_changes_nothing() {
    let (mut app, _ctx) = type_app();
    let st = app.session.active().unwrap();
    let (doc, revision, steps, active, selected) = (st.doc.clone(), st.revision, st.history.past_len(), st.active_layer, st.selected_layers().to_vec());
    right_click(&mut app, 30.0, 2.0);
    let st = app.session.active().unwrap();
    assert!(app.ui.text_edit.is_none(), "no text edit starts");
    assert_eq!(st.doc, doc, "the layers stay as they are");
    assert_eq!(st.revision, revision, "no document revision is bumped");
    assert_eq!(st.history.past_len(), steps, "no history state is added");
    assert_eq!(st.active_layer, active, "the active layer does not change");
    assert_eq!(st.selected_layers().to_vec(), selected, "the layer selection does not change");
    assert!(app.ui.canvas_tool_menu.is_some(), "only the menu opens");
}

#[test]
fn over_committed_type_the_type_layer_menu_stays() {
    let (mut app, ctx) = type_app();
    right_click(&mut app, 16.0, 16.0);
    let menu = open_menu(&app);
    assert!(matches!(menu.kind, MenuKind::TypeLayer { .. }), "the #7324 type layer menu, got {:?}", menu.kind);
    let ids = listed(&mut app, &ctx);
    assert!(ids.iter().any(|id| id == "type.editText"), "its rows, not the layer menu's: {ids:?}");
    assert!(!ids.iter().any(|id| id == "layer.duplicate"), "no generic layer rows over the glyph");
}

#[test]
fn while_editing_the_type_edit_menu_still_opens() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.ui.tool = Tool::Type;
    app.run("type.create", json!({"text": "Hello", "size": 14, "x": 1, "y": 22})).unwrap();
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, "type.editText", json!({})).unwrap();
    assert!(app.ui.text_edit.is_some());
    // Away from every glyph, the edit menu is what opens - not the layer menu.
    assert!(open_here(&mut app, Tool::Type, [50.0, 40.0]));
    let menu = open_menu(&app);
    assert!(matches!(menu.kind, MenuKind::TypeEdit { .. }), "the type edit menu, got {:?}", menu.kind);
}

#[test]
fn unmeasured_tools_and_a_selection_keep_opening_nothing() {
    // Vertical Type and both Type Mask tools: not measured, they open nothing today and still do.
    for tool in [Tool::VerticalType, Tool::HorizontalTypeMask, Tool::VerticalTypeMask] {
        let (mut app, _ctx) = type_app();
        app.ui.tool = tool;
        right_click(&mut app, 30.0, 2.0);
        assert!(app.ui.canvas_tool_menu.is_none(), "{tool:?} keeps opening nothing");
    }
    // The menu with an active selection is not measured: the right-click keeps opening nothing.
    let (mut app, _ctx) = type_app();
    app.run("select.rect", json!({"x": 40, "y": 30, "width": 10, "height": 10})).unwrap();
    right_click(&mut app, 30.0, 2.0);
    assert!(app.ui.canvas_tool_menu.is_none(), "with a selection, nothing opens (not measured)");
}

#[test]
fn pasteboard_and_other_tools_keep_their_routes() {
    // Outside the image, the Type tool's right-click stays the pasteboard menu.
    let (mut app, _ctx) = type_app();
    right_click(&mut app, -4.0, -4.0);
    let menu = open_menu(&app);
    assert!(matches!(menu.kind, MenuKind::Pasteboard), "pasteboard, got {:?}", menu.kind);
    // Other tools are untouched: a Count right-click is the same layer menu, a Brush right-click
    // stays the Brush Preset picker.
    let (mut app, ctx) = type_app();
    app.ui.tool = Tool::Count;
    right_click(&mut app, 30.0, 2.0);
    let menu = open_menu(&app);
    assert_eq!(labels(rows(&menu)), labels(COUNT_MENU), "Count keeps its menu");
    assert_eq!(listed(&mut app, &ctx).len(), 17);
    let (mut app, _ctx) = type_app();
    app.ui.tool = Tool::Brush;
    right_click(&mut app, 30.0, 2.0);
    assert!(app.ui.canvas_tool_menu.is_none(), "the Brush still takes its own right-click");
    assert!(app.ui.brush_picker.is_some(), "the picker, not a menu");
}

#[test]
fn shared_rows_are_the_layers_panel_menus_rows() {
    // Every canvas row that the Layers panel menu also carries has the same label and is enabled
    // the same way for the active layer (the panel greys through the main menus).
    let (mut app, _ctx) = type_app();
    right_click(&mut app, 30.0, 2.0);
    let menu = open_menu(&app);
    let st = app.session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let panel = crate::layer_menu_ui::entries(layer, false, false);
    let mut shared = 0;
    for (label, id) in rows(&menu).iter().flatten() {
        if let Some((plabel, _)) = panel.iter().flatten().find(|(_, pid)| pid == id) {
            assert_eq!(*label, *plabel, "the panel row for {id} reads {plabel}");
            assert_eq!(entry_enabled(&app, &menu, id), crate::menus::is_enabled(&app, id), "{id} greys as the panel greys it");
            shared += 1;
        }
    }
    assert_eq!(shared, 6, "Blending Options…, Duplicate…, Delete, Group…, Convert to Smart Object and Rasterize Layer overlap the panel");
}

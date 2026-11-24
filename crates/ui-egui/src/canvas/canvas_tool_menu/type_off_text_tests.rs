//! #12042: a right-click with the Horizontal Type tool away from text opens the generic layer
//! menu at the pointer, with the reference application's entries and greyed rules (the same rows
//! the Layers panel menu enables through the main menus). With an active selection it opens the
//! selection menu the Red Eye and Object Selection tools open, inside the selection and outside
//! it. Over committed type the #7324 type layer menu stays, with or without a selection; while
//! editing, the type edit menu stays. The Vertical Type and Type Mask tools are not measured:
//! they keep today's no menu.

use serde_json::json;

use photocraft_doc::LayerContent;

use super::Tool;
use super::count_tests::{call, listed, right_click};
use super::tests::{labels, open_here, screen_of};
use super::{COUNT_MENU, MEASURED_SELECTION_MENU, MenuKind, available, entry_enabled, rows};
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

/// `type_app` with a rectangular selection at (x, y, width, height).
fn selected_type_app(rect: [i64; 4]) -> (Box<PhotocraftApp>, egui::Context) {
    let (mut app, ctx) = type_app();
    app.run("select.rect", json!({"x": rect[0], "y": rect[1], "width": rect[2], "height": rect[3]})).unwrap();
    app.sync_views();
    (app, ctx)
}

/// The labels of the open menu's greyed rows, in menu order.
fn greyed(app: &PhotocraftApp, menu: &super::CanvasToolMenu) -> Vec<String> {
    rows(menu).iter().flatten().filter(|(_, id)| !available(app, menu, id)).map(|(label, _)| (*label).to_string()).collect()
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
fn with_a_selection_off_text_right_click_opens_the_selection_menu_inside_and_outside() {
    // Selection at (40, 30), 10 × 10, pixel layer "Ink" active: (42, 32) is inside it, (30, 2)
    // outside it, both away from the glyphs.
    for (x, y, place) in [(42.0, 32.0, "inside"), (30.0, 2.0, "outside")] {
        let (mut app, ctx) = selected_type_app([40, 30, 10, 10]);
        let st = app.session.active().unwrap();
        let (doc, revision, steps) = (st.doc.clone(), st.revision, st.history.past_len());
        right_click(&mut app, x, y);
        let menu = open_menu(&app);
        assert_eq!(menu.tool, Tool::Type, "the menu is the Type tool's");
        assert!(matches!(menu.kind, MenuKind::Tool), "the selection menu, not a type menu: {:?}", menu.kind);
        assert!(menu.has_selection, "the right-click captured the selection, {place} it");
        assert_eq!(labels(rows(&menu)), labels(MEASURED_SELECTION_MENU), "{place} the selection: the Red Eye and Object Selection rows");
        assert_eq!(
            listed(&mut app, &ctx),
            [
                "select.deselect",
                "select.inverse",
                "select.modify.feather",
                "select.selectAndMask",
                "―",
                "select.saveSelection",
                "select.toWorkPath",
                "―",
                "layer.new.layerViaCopy",
                "layer.new.layerViaCut",
                "layer.new.layer",
                "―",
                "edit.freeTransform",
                "select.transformSelection",
                "―",
                "edit.fill",
                "edit.stroke",
                "edit.contentAwareFill",
                "edit.deleteAndFillSelection",
                "―",
                "filter.lastFilter",
                "―",
                "edit.fade",
            ],
            "over the control channel too, {place} the selection"
        );
        // Measured with the pixel layer active: only the two filter-history rows are greyed.
        assert_eq!(greyed(&app, &menu), ["Last Filter", "Fade…"], "{place} the selection");
        let st = app.session.active().unwrap();
        assert!(app.ui.text_edit.is_none(), "no text edit starts");
        assert_eq!(st.doc, doc, "the layers, the history and the selection stay as they were");
        assert_eq!(st.revision, revision, "no document revision is bumped");
        assert_eq!(st.history.past_len(), steps, "no history state is added");
    }
}

#[test]
fn with_a_selection_an_active_text_layer_greys_the_selection_menu_as_measured() {
    // The text layer active (no pixel layer above it) and a live selection.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.ui.tool = Tool::Type;
    app.run("type.create", json!({"text": "Hello", "size": 14, "x": 1, "y": 22})).unwrap();
    app.run("select.rect", json!({"x": 40, "y": 30, "width": 10, "height": 10})).unwrap();
    app.sync_views();
    right_click(&mut app, 50.0, 40.0);
    let menu = open_menu(&app);
    assert!(matches!(menu.kind, MenuKind::Tool), "the selection menu, got {:?}", menu.kind);
    assert_eq!(labels(rows(&menu)), labels(MEASURED_SELECTION_MENU), "the same menu as with a pixel layer active");
    // Measured with a text layer active: those rows grey. Free Transform is the one row the
    // reference greys here and this application does not: #2630 lets a text layer take Free
    // Transform and a selection does not narrow it, pinned by
    // `transform_type_tests::type_layers_offer_free_transform_but_not_distort_or_perspective`
    // (also for the marquee's menu) and `free_transform_with_a_selection_frames_and_moves_the_whole_type_layer`.
    assert_eq!(
        greyed(&app, &menu),
        ["Layer Via Cut", "Fill…", "Stroke…", "Content-Aware Fill…", "Delete and Fill Selection", "Last Filter", "Fade…"],
        "measured with a text layer active, except Free Transform (#2630)"
    );
    assert!(available(&app, &menu, "edit.freeTransform"), "Free Transform stays live for editable type (#2630)");
    for (label, id) in [
        ("Layer Via Copy", "layer.new.layerViaCopy"),
        ("New Layer…", "layer.new.layer"),
        ("Transform Selection", "select.transformSelection"),
        ("Deselect", "select.deselect"),
        ("Select Inverse", "select.inverse"),
    ] {
        assert!(available(&app, &menu, id), "{label} stays live with a text layer active");
    }
    // Every row greys exactly like its menu-bar twin: the Type tool adds no menu-local rule here.
    for (_, id) in rows(&menu).iter().flatten() {
        assert_eq!(entry_enabled(&app, &menu, id), crate::menus::is_enabled(&app, id), "{id}");
    }
}

#[test]
fn with_a_selection_right_click_over_glyphs_still_opens_the_type_layer_menu() {
    // A selection beside the glyphs and one covering them: over the glyphs the #7324 menu wins.
    for rect in [[40, 30, 10, 10], [4, 4, 30, 30]] {
        let (mut app, ctx) = selected_type_app(rect);
        right_click(&mut app, 16.0, 16.0);
        let menu = open_menu(&app);
        assert!(matches!(menu.kind, MenuKind::TypeLayer { .. }), "the type layer menu over the glyph, got {:?}", menu.kind);
        let ids = listed(&mut app, &ctx);
        assert!(ids.iter().any(|id| id == "type.editText"), "its rows, not the selection menu's: {ids:?}");
        assert!(!ids.iter().any(|id| id == "select.deselect"), "no selection rows over the glyph");
    }
}

#[test]
fn unmeasured_type_tools_keep_opening_nothing() {
    // Vertical Type and both Type Mask tools: not measured, they open nothing today and still do,
    // with no selection and with one.
    for tool in [Tool::VerticalType, Tool::HorizontalTypeMask, Tool::VerticalTypeMask] {
        for with_selection in [false, true] {
            let (mut app, _ctx) = if with_selection { selected_type_app([40, 30, 10, 10]) } else { type_app() };
            app.ui.tool = tool;
            right_click(&mut app, 30.0, 2.0);
            assert!(app.ui.canvas_tool_menu.is_none(), "{tool:?} keeps opening nothing (selection: {with_selection})");
        }
    }
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

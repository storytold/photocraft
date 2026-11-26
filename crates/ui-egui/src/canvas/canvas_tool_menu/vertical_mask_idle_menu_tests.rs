//! #12091: a right-click with the Vertical Type Mask tool while no mask is being edited opens a
//! menu: the measured selection menu ([`MEASURED_SELECTION_MENU`], the rows the Type, Red Eye and
//! Object Selection tools show) when the document has an active selection,
//! and the generic layer menu #12042 added for the Horizontal Type tool ([`COUNT_MENU`]) when it has
//! none. The same rows, the same enablement and the same commands as those tools show today: no new
//! table and no menu-local rule, which entries are greyed was not measured. The right-click starts
//! no mask and journals nothing; while a mask is being edited its own menu stays (#12084). The
//! Vertical Type and Horizontal Type Mask tools were not measured and keep opening nothing.

use egui::Modifiers;
use photocraft_doc::LayerContent;
use serde_json::json;

use super::count_tests::{call, listed, right_click};
use super::tests::{labels, screen_of};
use super::{COUNT_MENU, CanvasToolMenu, MEASURED_SELECTION_MENU, MenuKind, SELECTION_MENU, Tool, available, entry_enabled, rows};
use crate::PhotocraftApp;

/// The reference document, small: 64 × 48 with committed "Hello" at the left and a pixel layer
/// "Ink" active on top. Away from the glyphs: (30, 2) and (42, 32); over them: (16, 16).
const GLYPH: (f64, f64) = (16.0, 16.0);
const OFF_TEXT: (f64, f64) = (30.0, 2.0);
/// The selection of the "with a selection" runs: (42, 32) is inside it, (30, 2) outside it.
const RECT: [i64; 4] = [40, 30, 10, 10];
/// Away from the glyphs, where a click with the mask tool starts a new mask edit.
const EDIT_AT: [f64; 2] = [52.0, 40.0];

/// A 64 × 48 document with committed type, a pixel layer "Ink" active, and `tool` chosen.
fn mask_app(tool: Tool) -> (Box<PhotocraftApp>, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.ui.tool = Tool::Type;
    app.run("type.create", json!({"text": "Hello", "size": 14, "x": 1, "y": 22})).unwrap();
    assert!(app.ui.text_edit.is_none(), "committed: no edit running");
    app.run("layer.new.layer", json!({"name": "Ink"})).unwrap();
    app.ui.tool = tool;
    app.sync_views();
    let ctx = app.ctx().clone();
    (app, ctx)
}

/// `mask_app` with a rectangular selection at [`RECT`].
fn selected_mask_app(tool: Tool) -> (Box<PhotocraftApp>, egui::Context) {
    let (mut app, ctx) = mask_app(tool);
    app.run("select.rect", json!({"x": RECT[0], "y": RECT[1], "width": RECT[2], "height": RECT[3]})).unwrap();
    app.sync_views();
    (app, ctx)
}

/// The open menu, or panic with the app state.
fn open_menu(app: &PhotocraftApp) -> CanvasToolMenu {
    app.ui.canvas_tool_menu.clone().expect("the right-click opened a menu")
}

/// A table's rows as `ui.inspect` lists the open menu: ids, with `―` for a separator.
fn listed_ids(table: &[super::Row]) -> Vec<String> {
    table.iter().map(|row| row.map(|(_, id)| id.to_string()).unwrap_or_else(|| "―".to_string())).collect()
}

#[test]
fn with_a_selection_the_right_click_opens_the_measured_selection_menu() {
    // Inside the selection and outside it: the menu depends on the selection, not on the point.
    // The rows are the measured selection table (#12042 review on this PR): no Distort or
    // Perspective, and Delete and Fill Selection after the three fill rows.
    for (x, y, place) in [(42.0, 32.0, "inside"), (30.0, 2.0, "outside")] {
        let (mut app, ctx) = selected_mask_app(Tool::VerticalTypeMask);
        right_click(&mut app, x, y);
        let menu = open_menu(&app);
        assert_eq!(menu.tool, Tool::VerticalTypeMask, "the menu is the Vertical Type Mask tool's");
        assert!(matches!(menu.kind, MenuKind::Tool), "the tool menu, not a type menu: {:?}", menu.kind);
        assert!(menu.has_selection, "the right-click captured the selection, {place} it");
        assert_eq!(labels(rows(&menu)), labels(MEASURED_SELECTION_MENU), "{place} the selection: the measured selection rows in their order");
        // Reachable over the control channel: `ui.inspect` lists the same entries in the same order.
        assert_eq!(listed(&mut app, &ctx), listed_ids(MEASURED_SELECTION_MENU), "over the control channel, {place} the selection");
        // The menu sits at the pointer.
        let at = screen_of(&app, [x as f32, y as f32]);
        let (dx, dy) = (menu.pos[0] - at.x, menu.pos[1] - at.y);
        assert!(dx.abs() <= 0.01 && dy.abs() <= 0.01, "menu at {:?}, pointer at {at:?}", menu.pos);
    }
}

#[test]
fn the_selection_menu_is_the_table_the_measured_tools_share() {
    // The rows are the shared measured table, not the marquee's own menu: row for row the Type
    // tool's selection menu in the same document, and every row enabled the same way.
    let (mut app, _ctx) = selected_mask_app(Tool::VerticalTypeMask);
    right_click(&mut app, 42.0, 32.0);
    let mask = open_menu(&app);
    let (mut other, _ctx) = selected_mask_app(Tool::Type);
    right_click(&mut other, 42.0, 32.0);
    let type_menu = open_menu(&other);
    assert_eq!(labels(rows(&mask)), labels(rows(&type_menu)), "the same rows as the Type tool's selection menu");
    for (label, id) in rows(&mask).iter().flatten() {
        assert_eq!(available(&app, &mask, id), available(&other, &type_menu, id), "{label} is live as it is in the Type tool's selection menu");
        // Which entries are greyed was not measured here, so the tool adds no rule of its own:
        // every row follows the menu-bar / Layers-panel rule, as it does for the other tools.
        assert_eq!(entry_enabled(&app, &mask, id), crate::menus::is_enabled(&app, id), "{id} greys as the menu bar greys it");
    }
    assert!(available(&app, &mask, "select.deselect") && available(&app, &mask, "select.transformSelection"), "the selection rows are live with a selection");
    assert!(!available(&app, &mask, "filter.lastFilter") && !available(&app, &mask, "edit.fade"), "no filter has run, so Last Filter and Fade… are greyed");
    // The marquee keeps its own menu: Distort and Perspective are its rows, and this menu has none.
    let (mut marquee_app, _ctx) = selected_mask_app(Tool::RectMarquee);
    right_click(&mut marquee_app, 42.0, 32.0);
    let marquee = open_menu(&marquee_app);
    assert_eq!(labels(rows(&marquee)), labels(SELECTION_MENU), "the marquee keeps the selection tools' own table");
    for id in ["edit.transform.distort", "edit.transform.perspective"] {
        assert!(rows(&marquee).iter().flatten().any(|(_, row)| *row == id), "Distort and Perspective belong to the marquee's menu");
        assert!(!rows(&mask).iter().flatten().any(|(_, row)| *row == id), "the mask menu offers no {id}");
    }
}

#[test]
fn without_a_selection_the_right_click_opens_the_type_tools_layer_menu() {
    let (mut app, ctx) = mask_app(Tool::VerticalTypeMask);
    right_click(&mut app, OFF_TEXT.0, OFF_TEXT.1);
    let menu = open_menu(&app);
    assert_eq!(menu.tool, Tool::VerticalTypeMask, "the menu is the Vertical Type Mask tool's");
    assert!(matches!(menu.kind, MenuKind::Tool), "the generic layer menu, not a type menu: {:?}", menu.kind);
    assert!(!menu.has_selection, "nothing is selected");
    assert_eq!(labels(rows(&menu)), labels(COUNT_MENU), "the layer menu #12042 added for the Type tool, same entries and order");
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
        ],
        "and the same over the control channel"
    );
    // The rows are shared with #12042, not copied: the Type tool's menu in the same document is the
    // same table with the same enablement.
    let (mut type_app, _ctx) = mask_app(Tool::Type);
    right_click(&mut type_app, 30.0, 2.0);
    let type_menu = open_menu(&type_app);
    assert_eq!(labels(rows(&menu)), labels(rows(&type_menu)), "the Type tool's layer menu, row for row");
    for (label, id) in rows(&menu).iter().flatten() {
        assert_eq!(available(&app, &menu, id), available(&type_app, &type_menu, id), "{label} is live as it is in the Type tool's menu");
    }
}

#[test]
fn the_right_click_starts_no_mask_and_changes_nothing() {
    for (mut app, _ctx) in [mask_app(Tool::VerticalTypeMask), selected_mask_app(Tool::VerticalTypeMask)] {
        let st = app.session.active().unwrap();
        let (doc, revision, steps, active, selected, selection) =
            (st.doc.clone(), st.revision, st.history.past_len(), st.active_layer, st.selected_layers().to_vec(), st.doc.selection.clone());
        right_click(&mut app, OFF_TEXT.0, OFF_TEXT.1);
        let st = app.session.active().unwrap();
        assert!(app.ui.text_edit.is_none(), "no mask edit starts");
        assert_eq!(st.doc, doc, "the layers stay as they are");
        assert_eq!(st.revision, revision, "no document revision is bumped");
        assert_eq!(st.history.past_len(), steps, "no history state is added");
        assert_eq!(st.active_layer, active, "the active layer does not change");
        assert_eq!(st.selected_layers().to_vec(), selected, "the layer selection does not change");
        assert_eq!(st.doc.selection, selection, "the selection does not change");
        assert!(app.ui.canvas_tool_menu.is_some(), "only the menu opens");
    }
}

#[test]
fn picking_a_row_runs_its_command_on_the_active_layer() {
    // With a selection: Deselect, over the control channel.
    let (mut app, ctx) = selected_mask_app(Tool::VerticalTypeMask);
    right_click(&mut app, 42.0, 32.0);
    let reply = call(&mut app, &ctx, "ui.context.choose", json!({"id": "select.deselect"}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(app.session.active().unwrap().doc.selection.is_none(), "the selection is gone");
    assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("select.deselect"));

    // Without one: Delete Layer removes the active pixel layer and leaves the committed type.
    let (mut app, ctx) = mask_app(Tool::VerticalTypeMask);
    let layers_before = app.session.active().unwrap().doc.layers.len();
    right_click(&mut app, OFF_TEXT.0, OFF_TEXT.1);
    let reply = call(&mut app, &ctx, "ui.context.choose", json!({"id": "layer.delete"}));
    assert_eq!(reply["ok"], true, "{reply}");
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers.len(), layers_before - 1, "the active layer is deleted");
    assert!(st.doc.layers.iter().all(|l| l.name != "Ink"), "the pixel layer is gone");
    assert!(st.doc.layers.iter().any(|l| matches!(l.content, LayerContent::Text(_))), "the type layer stays");

    // A dialog row runs its menu item and edits nothing.
    let (mut app, ctx) = mask_app(Tool::VerticalTypeMask);
    let (revision, journal) = (app.session.active().unwrap().revision, app.session.journal.len());
    right_click(&mut app, OFF_TEXT.0, OFF_TEXT.1);
    let reply = call(&mut app, &ctx, "ui.context.choose", json!({"id": "select.colorRange"}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(!app.framework.dialogs.is_empty(), "the Color Range dialog opens");
    assert_eq!(app.session.active().unwrap().revision, revision, "opening the dialog edits nothing");
    assert_eq!(app.session.journal.len(), journal, "and runs no journaled command beyond the dialog");
}

#[test]
fn over_committed_type_the_mask_still_opens_the_layer_menu() {
    // To the Type Mask tools existing type is just pixels, so `type_layer_at` leaves them out
    // (#7324): over a glyph the idle right-click is the layer menu, not the type layer menu.
    let (mut app, ctx) = mask_app(Tool::VerticalTypeMask);
    right_click(&mut app, GLYPH.0, GLYPH.1);
    let menu = open_menu(&app);
    assert!(matches!(menu.kind, MenuKind::Tool), "the generic layer menu, got {:?}", menu.kind);
    assert_eq!(labels(rows(&menu)), labels(COUNT_MENU), "the layer menu, same as away from the glyphs");
    let listed = listed(&mut app, &ctx);
    assert!(!listed.iter().any(|id| id == "type.editText"), "no type layer rows for a mask tool: {listed:?}");
}

#[test]
fn while_a_mask_is_being_edited_its_own_menu_stays() {
    // A click with the Vertical Type Mask tool starts a mask edit; its right-click keeps the menu of
    // the running edit (#12084), not the idle one.
    let (mut app, ctx) = mask_app(Tool::VerticalTypeMask);
    crate::canvas::testing::click(&mut app, EDIT_AT, Modifiers::NONE);
    assert!(app.ui.text_edit.as_ref().is_some_and(|edit| edit.mask), "a mask edit is running");
    right_click(&mut app, EDIT_AT[0], EDIT_AT[1]);
    let menu = open_menu(&app);
    assert!(matches!(menu.kind, MenuKind::TypeEdit { .. }), "the running edit's menu, got {:?}", menu.kind);
    let listed = listed(&mut app, &ctx);
    assert!(listed.iter().any(|id| id == "type.warpText"), "the edit menu's rows: {listed:?}");
    for id in ["select.deselect", "select.transformSelection", "layer.delete", "layer.duplicate"] {
        assert!(!listed.iter().any(|row| row == id), "{id} belongs to the idle menu, not to the edit: {listed:?}");
    }
}

#[test]
fn the_other_type_tools_keep_their_routes() {
    // The Vertical Type tool and the Horizontal Type Mask tool were not measured: still nothing,
    // with no selection and with one.
    for tool in [Tool::VerticalType, Tool::HorizontalTypeMask] {
        for with_selection in [false, true] {
            let (mut app, _ctx) = if with_selection { selected_mask_app(tool) } else { mask_app(tool) };
            right_click(&mut app, OFF_TEXT.0, OFF_TEXT.1);
            assert!(app.ui.canvas_tool_menu.is_none(), "{tool:?} keeps opening nothing (selection: {with_selection})");
        }
    }
    // The Horizontal Type tool keeps what #12042 measured for it.
    let (mut app, _ctx) = mask_app(Tool::Type);
    right_click(&mut app, OFF_TEXT.0, OFF_TEXT.1);
    assert_eq!(labels(rows(&open_menu(&app))), labels(COUNT_MENU), "Type: the layer menu without a selection");
    let (mut app, _ctx) = selected_mask_app(Tool::Type);
    right_click(&mut app, 42.0, 32.0);
    assert_eq!(labels(rows(&open_menu(&app))), labels(MEASURED_SELECTION_MENU), "Type: the measured selection menu with one");
    // The selection tools, Count and the Brush are untouched.
    let (mut app, _ctx) = selected_mask_app(Tool::MagicWand);
    right_click(&mut app, 42.0, 32.0);
    assert_eq!(labels(rows(&open_menu(&app))), labels(SELECTION_MENU), "the wand keeps the selection tools' menu");
    let (mut app, ctx) = mask_app(Tool::Count);
    right_click(&mut app, OFF_TEXT.0, OFF_TEXT.1);
    assert_eq!(labels(rows(&open_menu(&app))), labels(COUNT_MENU), "Count keeps its menu");
    assert_eq!(listed(&mut app, &ctx).len(), 17);
    let (mut app, _ctx) = mask_app(Tool::Brush);
    right_click(&mut app, OFF_TEXT.0, OFF_TEXT.1);
    assert!(app.ui.canvas_tool_menu.is_none(), "the Brush still takes its own right-click");
    assert!(app.ui.brush_picker.is_some(), "the picker, not a menu");
    // And off the image the pasteboard menu comes first, as in #12042.
    let (mut app, _ctx) = mask_app(Tool::VerticalTypeMask);
    right_click(&mut app, -4.0, -4.0);
    assert!(matches!(open_menu(&app).kind, MenuKind::Pasteboard), "pasteboard outside the image");
}

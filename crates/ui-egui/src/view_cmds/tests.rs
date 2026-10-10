use super::*;
use crate::menus::{invoke as menu, is_live, menu_items};

fn app_with(n: usize) -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for i in 0..n {
        app.run("file.new", json!({"width": 100 + i as u32 * 10, "height": 80})).unwrap();
    }
    app.sync_views();
    (app, egui::Context::default())
}

fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    menu_items(app).into_iter().find(|i| i.id == id).and_then(|i| i.checked)
}

#[test]
fn show_snap_extras_and_screen_modes_are_state() {
    let (mut app, ctx) = app_with(1);
    for id in [
        "view.show.layerEdges",
        "view.show.pixelGrid",
        "view.snapTo.layers",
        "view.extras",
        "view.screenMode.fullScreen",
        "view.flipHorizontal",
        "window.arrange.fourUp",
        "type.fontPreviewSize.huge",
        "type.languageOptions.eastAsianFeatures",
    ] {
        assert!(is_live(id), "{id}");
    }
    assert_eq!(checked(&app, "view.show.layerEdges"), Some(false));
    menu(&mut app, &ctx, "view.show.layerEdges", json!({})).unwrap();
    assert!(app.ui.view.show.layer_edges);
    assert_eq!(checked(&app, "view.show.layerEdges"), Some(true));
    menu(&mut app, &ctx, "view.extras", json!({})).unwrap();
    assert!(!app.ui.view.extras && !app.ui.view.shows(app.ui.view.show.layer_edges));
    // Showing any item turns Extras back on.
    menu(&mut app, &ctx, "view.show.mesh", json!({})).unwrap();
    assert!(app.ui.view.extras);
    menu(&mut app, &ctx, "view.snapTo.none", json!({})).unwrap();
    assert!(!app.ui.view.snap_to.guides && !app.ui.view.snap_to.document_bounds);
    menu(&mut app, &ctx, "view.snapTo.grid", json!({})).unwrap();
    assert_eq!(checked(&app, "view.snapTo.grid"), Some(true));
    // F cycles the screen modes.
    for want in ["fullScreenWithMenuBar", "fullScreen", "standard"] {
        menu(&mut app, &ctx, "view.screenMode.cycle", json!({})).unwrap();
        assert_eq!(app.ui.view.screen_mode, want);
    }
    menu(&mut app, &ctx, "view.screenMode.fullScreen", json!({})).unwrap();
    assert!(app.ui.view.hides_chrome() && app.ui.view.hides_tabs());
    assert_eq!(checked(&app, "view.screenMode.fullScreen"), Some(true));
    menu(&mut app, &ctx, "view.pixelAspectRatio.anamorphic2To1", json!({})).unwrap();
    assert_eq!(pixel_aspect_ratio(&app.ui.view.pixel_aspect), 2.0);
    // State serializes with the rest of the UI (control channel).
    let v = serde_json::to_value(&app.ui).unwrap();
    assert_eq!(v["view"]["screen_mode"], "fullScreen");
    assert_eq!(v["view"]["show"]["layer_edges"], true);
}

#[test]
fn zoom_presets_and_fit_layers() {
    let (mut app, ctx) = app_with(1);
    menu(&mut app, &ctx, "view.twoHundredPercent", json!({})).unwrap();
    assert_eq!(app.ui.views[0].zoom, 2.0);
    app.run("image.imageSize", json!({"resolution": 144, "resample": "none"})).unwrap();
    menu(&mut app, &ctx, "view.printSize", json!({})).unwrap();
    assert_eq!(app.ui.views[0].zoom, 0.5);
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 10, "y": 20, "width": 10, "height": 10})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    menu(&mut app, &ctx, "view.fitLayersOnScreen", json!({})).unwrap();
    assert_eq!(app.ui.views[0].center, [15.0, 25.0]);
    assert!(app.ui.views[0].zoom > 10.0);
}

/// #1718 review: Print Size and Fit Layers on Screen go through `zoom_levels` too, so they reach
/// past the old 6400 % cap and stop at 12800 %.
#[test]
fn print_size_and_fit_layers_use_the_whole_zoom_range() {
    let (mut app, ctx) = app_with(1);
    // 72 / 1 ppi = 7200 %: within the range now (it was cut to 6400 %).
    app.run("image.imageSize", json!({"resolution": 1, "resample": "none"})).unwrap();
    menu(&mut app, &ctx, "view.printSize", json!({})).unwrap();
    assert_eq!(app.ui.views[0].zoom, 72.0);
    // A 1 px layer fills the view only far beyond 12800 %: Fit stops at the limit.
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 40, "y": 30, "width": 1, "height": 1})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    menu(&mut app, &ctx, "view.fitLayersOnScreen", json!({})).unwrap();
    assert_eq!(app.ui.views[0].zoom, crate::zoom_levels::MAX);
}

/// Print Size divides the document's ppi into the Screen Resolution preference
/// (Units & Rulers), not into a constant 72 ppi.
#[test]
fn print_size_uses_the_screen_resolution_preference() {
    let (mut app, ctx) = app_with(1);
    app.run("image.imageSize", json!({"resolution": 1, "resample": "none"})).unwrap();
    app.run("prefs.set", json!({"path": "unitsAndRulers.screenResolution", "value": 96.0})).unwrap();
    menu(&mut app, &ctx, "view.printSize", json!({})).unwrap();
    assert_eq!(app.ui.views[0].zoom, 96.0, "96 ppi screen / 1 ppi document = 9600 %");
}

#[test]
fn arrange_layouts_floating_windows_and_matching() {
    let (mut app, ctx) = app_with(3);
    assert!(cells("tabs", egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0)), 3).is_none());
    menu(&mut app, &ctx, "window.arrange.threeUpStacked", json!({})).unwrap();
    assert_eq!(checked(&app, "window.arrange.threeUpStacked"), Some(true));
    let c = cells(&app.ui.view.arrange, egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0)), 3).unwrap();
    assert_eq!(c.len(), 3);
    assert_eq!(c[0].height(), 600.0);
    assert_eq!(c[1].height(), 300.0);
    let six = cells("sixUp", egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0)), 3).unwrap();
    assert_eq!((six.len(), six[1].min.x), (6, 300.0));
    menu(&mut app, &ctx, "window.arrange.floatAllInWindows", json!({})).unwrap();
    assert_eq!(app.ui.windows.len(), 3);
    app.ui.views[2].zoom = 3.0;
    app.ui.views[2].center = [1.0, 2.0];
    menu(&mut app, &ctx, "window.arrange.matchAll", json!({})).unwrap();
    assert!(app.ui.views.iter().all(|v| v.zoom == 3.0 && v.center == [1.0, 2.0]));
    assert!(app.ui.windows.iter().all(|w| w.view.zoom == 3.0));
    menu(&mut app, &ctx, "window.arrange.consolidateAllToTabs", json!({})).unwrap();
    assert!(app.ui.windows.is_empty());
    assert_eq!(app.ui.view.arrange, "tabs");
    menu(&mut app, &ctx, "window.arrange.newWindowForDocument", json!({})).unwrap();
    assert_eq!(app.ui.windows.len(), 1);
}

#[test]
fn window_panels_select_dock_tabs() {
    let (mut app, ctx) = app_with(1);
    menu(&mut app, &ctx, "window.panel.info", json!({})).unwrap();
    assert!(app.ui.panels.navigator && app.ui.dock_tabs.navigator == 2);
    assert_eq!(checked(&app, "window.panel.info"), Some(true));
    assert_eq!(checked(&app, "window.panel.histogram"), Some(false));
    menu(&mut app, &ctx, "window.panel.histogram", json!({})).unwrap();
    assert_eq!(app.ui.dock_tabs.navigator, 1);
    menu(&mut app, &ctx, "window.panel.actions", json!({})).unwrap();
    assert!(app.ui.panels.history && app.ui.dock_tabs.history == 1);
    menu(&mut app, &ctx, "window.panel.paths", json!({})).unwrap();
    assert_eq!(app.ui.dock_tabs.layers, 2);
    // Again on the visible tab hides the panel.
    menu(&mut app, &ctx, "window.panel.paths", json!({})).unwrap();
    assert!(!app.ui.panels.layers);
    menu(&mut app, &ctx, "type.panels.character", json!({})).unwrap();
    assert!(app.ui.panels.properties && app.ui.dock_tabs.properties == 0);
    for id in ["window.panel.channels", "window.panel.swatches", "window.panel.adjustments", "window.panel.paragraph"] {
        assert!(is_live(id), "{id}");
    }
}

#[test]
fn type_menu_checkmarks_follow_the_layer() {
    let (mut app, ctx) = app_with(1);
    assert!(!crate::menus::is_enabled(&app, "type.antiAlias.crisp"));
    app.run("type.create", json!({"x": 10, "y": 50, "text": "Type", "size": 24})).unwrap();
    menu(&mut app, &ctx, "type.antiAlias.crisp", json!({})).unwrap();
    assert_eq!(checked(&app, "type.antiAlias.crisp"), Some(true));
    assert_eq!(checked(&app, "type.antiAlias.smooth"), Some(false));
    menu(&mut app, &ctx, "type.openType.fractions", json!({})).unwrap();
    assert_eq!(checked(&app, "type.openType.fractions"), Some(true));
    assert_eq!(checked(&app, "type.openType.standardLigatures"), Some(true));
    assert_eq!(checked(&app, "type.orientation.horizontal"), Some(true));
}

#[test]
fn engine_commands_open_their_dialogs() {
    let (mut app, ctx) = app_with(2);
    let d = menu(&mut app, &ctx, "file.automate.fitImage", json!({})).unwrap()["dialog"].as_u64().unwrap();
    let dlg = app.ui.dialog_mut(d).unwrap();
    assert_eq!(dlg.fields["width"], 110);
    dlg.fields.insert("width".into(), json!(55));
    dlg.fields.insert("height".into(), json!(1000));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(app.session.active().unwrap().doc.size.width, 55);
    let d = menu(&mut app, &ctx, "file.fileInfo", json!({})).unwrap()["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(d).unwrap().fields.insert("title".into(), json!("Wheat Field"));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(app.run("file.fileInfo", json!({})).unwrap()["title"], "Wheat Field");
    let d = menu(&mut app, &ctx, "view.newGuideLayout", json!({})).unwrap()["dialog"].as_u64().unwrap();
    assert_eq!(app.ui.dialog_mut(d).unwrap().fields["columns"], 8);
    app.ui.dialog_mut(d).unwrap().fields.insert("columns".into(), json!(2));
    app.ui.dialog_mut(d).unwrap().fields.insert("gutter".into(), json!(10));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(app.session.active().unwrap().doc.guides.vertical, vec![0.0, 22.5, 32.5, 55.0]);
    app.run("type.create", json!({"x": 5, "y": 40, "text": "Warp", "size": 20})).unwrap();
    let d = menu(&mut app, &ctx, "type.warpText", json!({})).unwrap()["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(d).unwrap().fields.insert("style".into(), json!("arc"));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert!(app.run("type.info", json!({})).unwrap()["warp"]["style"] == "warpArc");
    // Batch needs a recorded action.
    assert!(menu(&mut app, &ctx, "file.automate.batch", json!({})).is_err());
    // Close Others keeps the active document's view (the documents are dirty; the unsaved-changes prompt is covered in discard_ui).
    app.ui.views[1].zoom = 4.0;
    crate::menus::invoke_unguarded(&mut app, &ctx, "file.closeOthers", json!({})).unwrap();
    assert_eq!(app.ui.views.len(), 1);
    assert_eq!(app.ui.views[0].zoom, 4.0);
    crate::menus::invoke_unguarded(&mut app, &ctx, "file.closeAll", json!({})).unwrap();
    assert!(app.ui.views.is_empty() && app.session.documents().is_empty());
}

#[test]
fn apply_image_and_calculations_open_dialogs_without_editing() {
    for id in ["image.applyImage", "image.calculations"] {
        let (mut app, ctx) = app_with(1);
        let revision = app.session.active().unwrap().revision;
        let dialog = menu(&mut app, &ctx, id, json!({})).unwrap()["dialog"].as_u64().unwrap_or_else(|| panic!("{id} did not open a dialog"));
        let fields = &app.ui.dialog_mut(dialog).unwrap().fields;
        assert_eq!(fields["__command"], id, "{id} opened the wrong dialog");
        assert_eq!(app.session.active().unwrap().revision, revision, "{id} edited before confirmation");
    }
}

#[test]
fn new_guide_layout_remembers_successful_values_not_cancelled_edits() {
    let (mut app, ctx) = app_with(1);
    let first = menu(&mut app, &ctx, "view.newGuideLayout", json!({})).unwrap()["dialog"].as_u64().unwrap();
    assert_eq!(app.ui.dialog_mut(first).unwrap().fields["columns"], 8);
    assert_eq!(app.ui.dialog_mut(first).unwrap().fields["gutter"], 20);
    for (key, value) in [("columns", json!(2)), ("gutter", json!(0)), ("rows", json!(2)), ("rowGutter", json!(0))] {
        app.ui.dialog_mut(first).unwrap().fields.insert(key.into(), value);
    }
    crate::dialogs::confirm(&mut app, first).unwrap();

    let second = menu(&mut app, &ctx, "view.newGuideLayout", json!({})).unwrap()["dialog"].as_u64().unwrap();
    let fields = &app.ui.dialog_mut(second).unwrap().fields;
    assert_eq!(fields["columns"], 2);
    assert_eq!(fields["gutter"], 0);
    assert_eq!(fields["rows"], 2);
    assert_eq!(fields["rowGutter"], 0);
    assert_eq!(fields["clearExisting"], false);
    // A cancelled edit must not replace the last successfully used values.
    app.ui.dialog_mut(second).unwrap().fields.insert("columns".into(), json!(9));
    app.ui.close_dialog(second);
    let third = menu(&mut app, &ctx, "view.newGuideLayout", json!({})).unwrap()["dialog"].as_u64().unwrap();
    assert_eq!(app.ui.dialog_mut(third).unwrap().fields["columns"], 2);

    // The remembered state is part of the serializable UI state.
    let json_state = serde_json::to_value(&app.ui).unwrap();
    assert_eq!(json_state["view"]["guide_layout"]["columns"], 2);
    let restored: crate::state::UiState = serde_json::from_value(json_state).unwrap();
    assert_eq!(restored.view.guide_layout["rows"], 2);
}

#[test]
fn new_guide_layout_does_not_remember_a_failed_command() {
    let (mut app, _) = app_with(0);
    let id = front(&mut app, "view.newGuideLayout", &json!({})).unwrap().unwrap()["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("columns".into(), json!(3));
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_eq!(app.ui.view.guide_layout["columns"], 8);
}

#[test]
fn lookup_export_dialog_uses_the_current_adjustment_selection() {
    let (mut app, _ctx) = app_with(1);
    app.run("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let current = app.session.active().unwrap().active_layer.unwrap();
    let opened = front(&mut app, "file.export.colorLookupTables", &json!({})).unwrap().unwrap();
    let id = opened["dialog"].as_u64().unwrap();
    let dialog = &app.ui.dialogs.last().unwrap().fields;
    assert_eq!(dialog["scope"], "selected");
    assert_eq!(dialog["size"], 33);
    assert_eq!(dialog["__choices"]["scope"], json!(["all", "selected"]));
    app.ui.close_dialog(id);

    app.run("layer.new.layer", json!({"name": "Unrelated pixels"})).unwrap();
    let opened = front(&mut app, "file.export.colorLookupTables", &json!({})).unwrap().unwrap();
    let id = opened["dialog"].as_u64().unwrap();
    assert_eq!(app.ui.dialogs.last().unwrap().fields["scope"], "all");
    app.ui.close_dialog(id);

    app.run("layer.select", json!({"layer": current})).unwrap();
    let direct = app.run("file.export.colorLookupTables", json!({"size": 3, "scope": "selected"})).unwrap();
    assert_eq!(direct["layerCount"], 1);
    assert!(direct["cube"].as_str().unwrap().contains("LUT_3D_SIZE 3"));
}

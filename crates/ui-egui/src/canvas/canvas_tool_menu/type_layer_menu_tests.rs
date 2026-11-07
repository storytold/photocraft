//! #12043: The Type tool's right-click over committed text shows the reference application's
//! menu: Edit Type… on top, Faux Bold and Faux Italic acting on the whole layer, Layer Style…
//! last, and only the Convert row the text's shape allows. The menu shown while editing
//! (`TYPE_EDIT_MENU`) is unchanged.

use super::*;
use egui::Context;

/// A committed type layer reading "Hello" over the middle of a 32 px canvas, no edit running.
fn app_with_type() -> (PhotocraftApp, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.ui.tool = Tool::Type;
    let id = app.run("type.create", json!({"text": "Hello", "size": 14, "x": 1, "y": 22})).unwrap()["layer"].as_u64().unwrap();
    assert!(app.ui.text_edit.is_none(), "committed: no edit running");
    (app, id)
}

/// Right-click over the type: the menu opens as a type layer menu.
fn open_over_type(app: &mut PhotocraftApp) -> CanvasToolMenu {
    assert!(open_at(app, &Context::default(), Tool::Type, [10.0, 100.0], Some([16.0, 16.0])), "the canvas centre is on the type");
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(menu.type_layer && !menu.text);
    menu
}

fn row_ids(menu: &CanvasToolMenu) -> Vec<Option<&'static str>> {
    rows(menu).iter().map(|r| r.map(|(_, id)| id)).collect()
}

fn text_of(app: &PhotocraftApp, id: u64) -> photocraft_doc::TextLayer {
    match &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
        LayerContent::Text(t) => t.clone(),
        other => panic!("layer {id} is no longer text: {other:?}"),
    }
}

fn history_len(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

/// The measured entry order (#12043): Edit Type… on top, then the layer rows, Faux Bold and
/// Faux Italic above the Convert row, and Layer Style… last.
#[test]
fn point_text_menu_has_the_measured_rows_in_order() {
    let (mut app, _id) = app_with_type();
    let menu = open_over_type(&mut app);
    let want = vec![
        Some("type.editText"),
        None,
        Some("edit.checkSpelling"),
        Some("edit.findAndReplaceText"),
        None,
        Some("type.rasterizeTypeLayer"),
        Some("type.createWorkPath"),
        Some("type.convertToShape"),
        None,
        Some("type.orientation.horizontal"),
        Some("type.orientation.vertical"),
        None,
        Some("type.antiAlias.none"),
        Some("type.antiAlias.sharp"),
        Some("type.antiAlias.crisp"),
        Some("type.antiAlias.strong"),
        Some("type.antiAlias.smooth"),
        Some("type.antiAlias.windowsLcd"),
        Some("type.antiAlias.windows"),
        None,
        Some(FAUX_BOLD),
        Some(FAUX_ITALIC),
        None,
        Some("type.convertToParagraphText"),
        None,
        Some("type.warpText"),
        None,
        Some("layer.layerStyle.blendingOptions"),
    ];
    assert_eq!(row_ids(&menu), want);
}

/// Only one Convert entry: Convert to Paragraph Text on point text, Convert to Point Text on
/// paragraph text — the row that does not apply is absent, not greyed.
#[test]
fn only_the_convert_row_that_applies_is_listed() {
    let (mut app, id) = app_with_type();
    let menu = open_over_type(&mut app);
    let ids: Vec<&str> = rows(&menu).iter().flatten().map(|(_, id)| *id).collect();
    assert!(ids.contains(&"type.convertToParagraphText") && !ids.contains(&"type.convertToPointText"), "point text lists its one row: {ids:?}");
    app.run("type.convertToParagraphText", json!({"layer": id})).unwrap();
    let menu = open_over_type(&mut app);
    let ids: Vec<&str> = rows(&menu).iter().flatten().map(|(_, id)| *id).collect();
    assert!(ids.contains(&"type.convertToPointText") && !ids.contains(&"type.convertToParagraphText"), "paragraph text lists its one row: {ids:?}");
}

/// Faux Bold and Faux Italic toggle the style for the whole layer as one history state, and
/// the row shows a check while the layer uses it.
#[test]
fn faux_rows_toggle_the_whole_layer_in_one_history_state() {
    let (mut app, id) = app_with_type();
    let menu = open_over_type(&mut app);
    assert!(entry_enabled(&app, &menu, FAUX_BOLD), "Faux Bold is a live row of the layer menu");
    assert!(entry_enabled(&app, &menu, FAUX_ITALIC));
    assert!(!mode_checked(&app, &menu, FAUX_BOLD), "off to start");
    let before = history_len(&app);
    choose(&mut app, &Context::default(), FAUX_BOLD);
    assert!(text_of(&app, id).char_runs().iter().all(|r| r.style.faux_bold), "the whole layer takes Faux Bold");
    assert_eq!(history_len(&app), before + 1, "one history state for the toggle");
    let menu = open_over_type(&mut app);
    assert!(mode_checked(&app, &menu, FAUX_BOLD), "the row shows a check while the layer uses it");
    choose(&mut app, &Context::default(), FAUX_BOLD);
    assert!(text_of(&app, id).char_runs().iter().all(|r| !r.style.faux_bold), "and toggles back off");
    open_over_type(&mut app);
    choose(&mut app, &Context::default(), FAUX_ITALIC);
    assert!(text_of(&app, id).char_runs().iter().all(|r| r.style.faux_italic), "Faux Italic likewise");
}

/// Edit Type… starts editing the layer that was right-clicked, even inside a multiselection.
#[test]
fn edit_type_starts_editing_the_right_clicked_layer() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool = Tool::Type;
    let top = app.run("type.create", json!({"text": "Top", "size": 30, "x": 10, "y": 40})).unwrap()["layer"].as_u64().unwrap();
    let bottom = app.run("type.create", json!({"text": "Low", "size": 30, "x": 10, "y": 90})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.select", json!({"layer": bottom})).unwrap();
    app.run("layer.select", json!({"layer": top, "mode": "add"})).unwrap();
    let at = [25.0, 80.0];
    assert_eq!(crate::type_tool::hit_layer(&mut app, at[0], at[1]).map(|l| l.0), Some(bottom), "the point is on the bottom layer");
    let ctx = Context::default();
    assert!(open_at(&mut app, &ctx, Tool::Type, [10.0, 10.0], Some(at)));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(available(&app, &menu, "type.editText"), "Edit Type… is a live row over committed type");
    choose(&mut app, &ctx, "type.editText");
    assert_eq!(app.ui.text_edit.as_ref().map(|e| e.layer), Some(bottom), "Edit Type… edits the clicked layer");
}

/// Layer Style… opens the Layer Style dialog for that layer.
#[test]
fn layer_style_opens_the_layer_style_dialog() {
    let (mut app, _id) = app_with_type();
    let menu = open_over_type(&mut app);
    assert!(available(&app, &menu, "layer.layerStyle.blendingOptions"), "Layer Style… is a live row over committed type");
    choose(&mut app, &Context::default(), "layer.layerStyle.blendingOptions");
    let d = app.ui.dialogs.last().expect("Layer Style… opens the Layer Style dialog");
    assert_eq!(d.kind, crate::state::DialogKind::LayerStyle);
}

/// What must not change: the menu shown while editing keeps its Undo/Cut group, both Convert
/// rows (the one that doesn't apply stays greyed) and no Edit Type… or Layer Style… row.
#[test]
fn the_type_edit_menu_is_unchanged() {
    assert_eq!(TYPE_EDIT_MENU.first().copied().flatten(), Some(("Undo", "edit.undo")));
    let has = |id: &str| TYPE_EDIT_MENU.iter().flatten().any(|(_, i)| *i == id);
    assert!(has("type.convertToParagraphText") && has("type.convertToPointText"), "while editing, both Convert rows stay");
    assert!(!has("type.editText") && !has("layer.layerStyle.blendingOptions"));
}

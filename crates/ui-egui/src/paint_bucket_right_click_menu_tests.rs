//! #12530: with the Paint Bucket a plain right-click on the image opens its Mode list at the
//! pointer, the options bar's entries with none checked; picking one sets the Mode, adds no history
//! and fills nothing. The pasteboard keeps its colour menu, Shift+right-click keeps its checked
//! menu and other tools keep their right-click. A Free Transform box keeps its transform menu.

use egui::Modifiers;
use photocraft_doc::BlendMode;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::canvas_tool_menu::{self, MenuKind};
use crate::state::Tool;

fn bucket() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = Tool::PaintBucket;
    app.sync_views();
    app
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// The options bar's Mode list for the Paint Bucket, Clear included.
fn options_bar_modes() -> Vec<&'static str> {
    crate::blend_keys::tool_modes(Tool::PaintBucket).unwrap().into_iter().map(|m| m.map_or("Clear", BlendMode::label)).collect()
}

/// The open Mode menu's rows (ids) and the ids of the checked ones; `None` when it is not open.
fn mode_menu(app: &PhotocraftApp) -> Option<(Vec<String>, Vec<String>)> {
    let menu = app.ui.canvas_tool_menu.as_ref().filter(|m| matches!(m.kind, MenuKind::BrushMode { .. }))?;
    let rows: Vec<_> = canvas_tool_menu::entries(app, menu).into_iter().filter(|e| !e.is_separator()).collect();
    let id = |e: &craft::ui::contract::Entry<String>| e.value().cloned().unwrap_or_default();
    Some((rows.iter().map(id).collect(), rows.iter().filter(|e| e.is_checked()).map(id).collect()))
}

#[test]
fn a_plain_right_click_lists_the_options_bar_modes_with_none_checked() {
    let mut app = bucket();
    testing::secondary_click(&mut app, [32.0, 32.0], Modifiers::NONE);
    let (rows, checked) = mode_menu(&app).expect("the Mode list opens");
    assert_eq!(rows, options_bar_modes());
    assert!(checked.is_empty(), "no entry is checked: {checked:?}");
    assert!(crate::brush_picker::opened_at(&app).is_none(), "not the preset picker");
    assert_eq!(
        crate::brush_mode_menu::current(&app, Tool::PaintBucket),
        Some(crate::brush_mode_menu::Item::Blend(BlendMode::Normal)),
        "the tool's mode is unchanged"
    );
}

#[test]
fn picking_an_entry_sets_the_mode_and_adds_no_history() {
    let mut app = bucket();
    let (n, journal) = (steps(&app), app.session.journal.len());
    testing::secondary_click(&mut app, [32.0, 32.0], Modifiers::NONE);
    assert!(mode_menu(&app).is_some());
    assert_eq!(testing::control(&mut app, "ui.context.choose", json!({"id": "Multiply"}))["ok"], true);
    assert_eq!(app.ui.tool_options.bucket_blend_mode, BlendMode::Multiply);
    assert!(!app.ui.tool_options.bucket_clear);
    assert!(mode_menu(&app).is_none(), "choosing closes the menu");
    assert_eq!((steps(&app), app.session.journal.len()), (n, journal), "nothing is filled");
}

#[test]
fn shift_right_click_still_checks_the_current_mode() {
    let mut app = bucket();
    app.ui.tool_options.bucket_blend_mode = BlendMode::Screen;
    testing::secondary_click(&mut app, [32.0, 32.0], Modifiers::SHIFT);
    let (_, checked) = mode_menu(&app).expect("the Mode list opens");
    assert_eq!(checked, ["Screen"]);
}

#[test]
fn other_tools_keep_their_plain_right_click() {
    for tool in [Tool::Brush, Tool::Pencil, Tool::CloneStamp, Tool::Gradient, Tool::Eraser] {
        let mut app = bucket();
        app.ui.tool = tool;
        testing::secondary_click(&mut app, [32.0, 32.0], Modifiers::NONE);
        assert!(mode_menu(&app).is_none(), "{tool:?}");
    }
}

#[test]
fn a_right_click_on_the_pasteboard_opens_its_colour_menu() {
    let mut app = bucket();
    testing::secondary_click(&mut app, [-8.0, 32.0], Modifiers::NONE);
    assert!(app.ui.canvas_tool_menu.as_ref().is_some_and(|m| matches!(m.kind, MenuKind::Pasteboard)), "the pasteboard colour menu");
    assert!(mode_menu(&app).is_none(), "not the Mode list");
}

/// An open Free Transform box keeps its transform menu: the Mode list does not open over it.
#[test]
fn a_right_click_on_a_free_transform_box_opens_only_the_transform_menu() {
    let mut app = bucket();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 4, "y": 6, "width": 10, "height": 8})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let ctx = testing::ctx(&app);
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    testing::secondary_click(&mut app, [32.0, 32.0], Modifiers::NONE);
    // The box's own menu is the framework transform overlay's context menu, not a canvas tool menu.
    assert!(app.ui.canvas_tool_menu.is_none(), "no canvas tool menu over the box");
    assert!(mode_menu(&app).is_none(), "not the Mode list");
}

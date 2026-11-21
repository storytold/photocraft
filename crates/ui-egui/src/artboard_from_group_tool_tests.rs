//! #14563: after Layer › New › Artboard from Group from the menu, the Artboard tool is current and
//! its options bar shows the new board's size. Run as an engine command the tool stays, and a
//! refused command leaves it too.
//!
//! The document is 800 × 600 with a group "Cards" holding two pixel layers that together span
//! 100,100 to 600,450, and the Move tool current.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;

const FROM_GROUP: &str = "layer.new.artboardFromGroup";

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    let mut layers = Vec::new();
    for (name, rect, color) in [("Left", [100, 100, 200, 150], "#c0392b"), ("Right", [350, 250, 250, 200], "#2471a3")] {
        let id = app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap();
        app.run("select.rect", json!({"x": rect[0], "y": rect[1], "width": rect[2], "height": rect[3]})).unwrap();
        app.run("edit.fill", json!({"color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        layers.push(id);
    }
    app.run("layer.select", json!({"layer": layers[0]})).unwrap();
    app.run("layer.select", json!({"layer": layers[1], "mode": "add"})).unwrap();
    app.run("layer.groupLayers", json!({"name": "Cards"})).unwrap();
    app.ui.tool = Tool::Move;
    app.sync_views();
    app
}

/// Layer › New › Artboard from Group, clicked in the menu.
fn from_menu(app: &mut PhotocraftApp) -> Result<Value, String> {
    let ctx = app.ctx().clone();
    let r = crate::menus::invoke(app, &ctx, FROM_GROUP, json!({}));
    app.sync_views();
    r
}

fn has_board(app: &PhotocraftApp) -> bool {
    app.session.active().unwrap().doc.has_artboards()
}

#[test]
fn from_the_menu_the_artboard_tool_is_current_with_the_new_boards_size() {
    let mut app = app();
    from_menu(&mut app).unwrap();
    assert!(has_board(&app));
    assert_eq!(app.ui.tool, Tool::Artboard);
    assert_eq!(crate::tools::artboard_tool::bar::bar_size(&app), [500, 350], "the bar shows the new board's size");
}

#[test]
fn run_as_an_engine_command_the_tool_stays() {
    let mut app = app();
    app.run(FROM_GROUP, json!({})).unwrap();
    assert!(has_board(&app));
    assert_eq!(app.ui.tool, Tool::Move);
}

#[test]
fn a_refused_command_leaves_the_tool() {
    let mut app = app();
    // A pixel layer active, no group: the command is refused.
    let left = app.session.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l).find(|l| l.name == "Left").unwrap().id.0;
    app.run("layer.select", json!({"layer": left})).unwrap();
    app.sync_views();
    assert!(from_menu(&mut app).is_err());
    assert!(!has_board(&app));
    assert_eq!(app.ui.tool, Tool::Move);
}

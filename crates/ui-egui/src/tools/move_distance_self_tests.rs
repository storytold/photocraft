//! ⌘-hover over the selection itself measures nothing (#16861).

use craft::behavior::kurbo::Rect;
use serde_json::json;

use super::current;
use crate::PhotocraftApp;
use crate::state::Tool;

/// The issue's document: 1000×800 with a white Background and layers A (100,100 to 300,260),
/// C (400,100 to 500,200) and B (600,400 to 750,520, top and selected).
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 1000, "height": 800})).unwrap();
    for (name, x, y, width, height) in [("A", 100, 100, 200, 160), ("C", 400, 100, 100, 100), ("B", 600, 400, 150, 120)] {
        app.run("layer.new.layer", json!({"name": name})).unwrap();
        app.run("select.rect", json!({"x": x, "y": y, "width": width, "height": height})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    }
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.view.extras = true;
    app.ui.view.show.smart_guides = true;
    app
}

const CANVAS: Rect = Rect::new(0.0, 0.0, 1000.0, 800.0);

#[test]
fn command_over_the_selected_layer_shows_nothing() {
    let mut app = app();
    let b = Rect::new(600.0, 400.0, 750.0, 520.0);
    assert_eq!(current(&mut app, Some([675.0, 460.0]), true, false), None, "over B, B selected");
    // The cached answer for the same pixel doesn't outlive a selection change.
    app.run("layer.select", json!({"name": "A"})).unwrap();
    assert_eq!(current(&mut app, Some([675.0, 460.0]), true, false).map(|(_, to)| to), Some(b), "B is another layer now");
    // Empty canvas (the Background) still measures to the canvas edges, and another layer to it.
    app.run("layer.select", json!({"name": "B"})).unwrap();
    assert_eq!(current(&mut app, Some([900.0, 100.0]), true, false), Some((b, CANVAS)));
    assert_eq!(current(&mut app, Some([450.0, 150.0]), true, false), Some((b, Rect::new(400.0, 100.0, 500.0, 200.0))));
}

#[test]
fn command_over_a_layer_in_a_selected_group_shows_nothing() {
    let mut app = app();
    app.run("layer.select", json!({"name": "A"})).unwrap();
    app.run("layer.select", json!({"name": "C", "mode": "add"})).unwrap();
    app.run("layer.groupLayers", json!({"name": "G"})).unwrap();
    for target in ["layer", "group"] {
        app.ui.tool_options.move_target = target.into();
        assert_eq!(current(&mut app, Some([200.0, 180.0]), true, false), None, "over A, {target}");
        assert_eq!(current(&mut app, Some([450.0, 150.0]), true, false), None, "over C, {target}");
        let g = Rect::new(100.0, 100.0, 500.0, 260.0);
        assert_eq!(current(&mut app, Some([675.0, 460.0]), true, false), Some((g, Rect::new(600.0, 400.0, 750.0, 520.0))), "over B, {target}");
    }
}

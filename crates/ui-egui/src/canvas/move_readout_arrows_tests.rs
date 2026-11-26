//! Regression tests for the Move tool's directional drag readout (#16859).

use serde_json::json;

use crate::canvas::{ToolEvent, tool_event_response};
use crate::state::Tool;
use crate::{PhotocraftApp, Services};

fn app_with_layer() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(16, 16, 32, 32), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app
}

fn readout(app: &mut PhotocraftApp, x: f64, y: f64) -> Vec<(String, String)> {
    tool_event_response(app, ToolEvent::Move { x, y, pressure: 1.0 }, egui::Modifiers::NONE)
        .readout
        .expect("a moved layer has a drag readout")
        .rows
        .into_iter()
        .map(|(label, value)| (label.as_str().to_owned(), value))
        .collect()
}

#[test]
fn move_readout_uses_direction_arrows_and_absolute_distances() {
    let mut app = app_with_layer();
    let _ = tool_event_response(&mut app, ToolEvent::Down { x: 24.0, y: 24.0, pressure: 1.0 }, egui::Modifiers::NONE);

    assert_eq!(readout(&mut app, 34.4, 20.8), [("→".to_string(), "10 px".to_string()), ("↑".to_string(), "3 px".to_string())]);
    assert_eq!(readout(&mut app, 13.6, 27.2), [("←".to_string(), "10 px".to_string()), ("↓".to_string(), "3 px".to_string())]);
}

#[test]
fn move_readout_uses_down_arrow_for_zero_vertical_distance() {
    let mut app = app_with_layer();
    let _ = tool_event_response(&mut app, ToolEvent::Down { x: 24.0, y: 24.0, pressure: 1.0 }, egui::Modifiers::NONE);

    assert_eq!(readout(&mut app, 34.4, 24.0), [("→".to_string(), "10 px".to_string()), ("↓".to_string(), "0 px".to_string())]);
}

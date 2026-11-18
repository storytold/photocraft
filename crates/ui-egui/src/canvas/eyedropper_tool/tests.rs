use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "transparent"})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [0, 0, 200, 300], "fill": "#ff0000"})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [200, 0, 200, 300], "fill": "#00ff00"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 1.0, 1.0], "background": [1.0, 1.0, 1.0, 1.0]})).unwrap();
    app.ui.tool = tool;
    app.sync_views();
    app
}

/// Every drag pick applies, not only the release: the swatch follows the pointer.
#[test]
fn eyedropper_drag_updates_the_sampled_colour() {
    let mut app = app(Tool::Eyedropper);
    let none = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, none);
    assert_eq!(app.session.tools.foreground, [1.0, 0.0, 0.0, 1.0], "the press picked the red half");
    tool_event(&mut app, ToolEvent::Move { x: 300.0, y: 100.0, pressure: 1.0 }, none);
    assert_eq!(app.session.tools.foreground, [0.0, 1.0, 0.0, 1.0], "the drag picked the green half before the release");
    tool_event(&mut app, ToolEvent::Up { x: 300.0, y: 100.0 }, none);
    assert_eq!(app.session.tools.foreground, [0.0, 1.0, 0.0, 1.0]);
    // Alt picks the background, and leaves the foreground.
    tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, egui::Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 100.0 }, egui::Modifiers::ALT);
    assert_eq!(app.session.tools.background, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(app.session.tools.foreground, [0.0, 1.0, 0.0, 1.0]);
}

/// #11524: what the Eyedropper picks is remembered, as Photoshop's Recently Used Colors:
/// newest first, every colour once, at most [`crate::paint_picker::MAX_RECENT`] of them, ⌥
/// (background) samples included, and a sample off the image records nothing.
#[test]
fn eyedropper_samples_join_the_recently_used_colors() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 220, "height": 20, "background": "transparent"})).unwrap();
    for (x, fill) in [(0.0, "#ff0000"), (20.0, "#00ff00"), (40.0, "#0000ff")] {
        app.run("shape.create", json!({"kind": "rect", "rect": [x, 0.0, 20.0, 20.0], "fill": fill})).unwrap();
    }
    // Fifteen more bands in single red steps, to push the row past its cap.
    for i in 0..15u8 {
        let x = 60.0 + f64::from(i) * 10.0;
        app.run("shape.create", json!({"kind": "rect", "rect": [x, 0.0, 10.0, 20.0], "fill": format!("#{i:02x}0000")})).unwrap();
    }
    app.ui.tool = Tool::Eyedropper;
    app.sync_views();
    let hex = |c: [f32; 4]| crate::color_picker_ui::hex([c[0], c[1], c[2]]);
    let recent = |app: &PhotocraftApp| crate::paint_picker::recent_colors(&app.session).into_iter().map(crate::color_picker_ui::hex).collect::<Vec<String>>();
    let none = egui::Modifiers::NONE;
    assert!(recent(&app).is_empty(), "nothing has been sampled yet");
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, none);
    let red = hex(app.session.tools.foreground);
    assert_eq!(recent(&app), [red.as_str()], "the first sample, remembered as it was applied");
    // The drag keeps recording, newest first; coming back to a colour moves it to the front
    // instead of listing it twice.
    tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 10.0, pressure: 1.0 }, none);
    let green = hex(app.session.tools.foreground);
    tool_event(&mut app, ToolEvent::Move { x: 10.0, y: 10.0, pressure: 1.0 }, none);
    assert_eq!(recent(&app), [red.as_str(), green.as_str()], "{red} before {green}");
    tool_event(&mut app, ToolEvent::Up { x: 10.0, y: 10.0 }, none);
    // A ⌥ sample feeds the background and the history alike.
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Up { x: 50.0, y: 10.0 }, egui::Modifiers::ALT);
    assert_eq!(recent(&app), [hex(app.session.tools.background), red.clone(), green.clone()], "a background sample is just as recent");
    // Off the image there is nothing to sample and nothing to remember.
    tool_event(&mut app, ToolEvent::Down { x: 300.0, y: 10.0, pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Up { x: 300.0, y: 10.0 }, none);
    assert_eq!(recent(&app).len(), 3, "a failed sample adds nothing");
    // Fifteen further picks keep the row at its cap and push the first colours out.
    for i in 0..15u8 {
        let x = 65.0 + f64::from(i) * 10.0;
        tool_event(&mut app, ToolEvent::Down { x, y: 10.0, pressure: 1.0 }, none);
        tool_event(&mut app, ToolEvent::Up { x, y: 10.0 }, none);
    }
    let recent = recent(&app);
    assert_eq!(recent.len(), crate::paint_picker::MAX_RECENT, "the row keeps at most fifteen colours");
    assert_eq!(recent[0], "#0e0000", "newest first");
    assert!(!recent.contains(&red), "{red} was pushed off the row");
}

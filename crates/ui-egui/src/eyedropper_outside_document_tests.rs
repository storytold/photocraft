//! #11755: the Eyedropper samples the pasteboard too. A click on the area around the image sets the foreground to
//! the pasteboard's colour as drawn, whatever the Sample mode, and a drag that leaves the image takes the colour at
//! the point it reached. On the image nothing changes. Clicks and drags go through the canvas tool host, as the mouse
//! does.

use craft::color_ui::ui::SampleSource;
use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// A 400 × 300 document filled red, blue foreground, the Eyedropper.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#0000ff", "background": "#ffffff"})).unwrap();
    app.ui.tool = Tool::Eyedropper;
    app.sync_views();
    app
}

/// The pasteboard in a custom colour of its own (Preferences › Interface › Canvas color).
fn custom_pasteboard(app: &mut PhotocraftApp) {
    app.run("prefs.set", json!({"values": {"interface.canvasColor": "custom", "interface.canvasCustomColor": "#7a4f21"}})).unwrap();
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64, mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x, y }, mods);
}

fn rgb(c: [f32; 4]) -> [u8; 3] {
    [c[0], c[1], c[2]].map(|v| (v * 255.0).round() as u8)
}

fn foreground(app: &PhotocraftApp) -> [u8; 3] {
    rgb(app.session.tools.foreground)
}

const CUSTOM: [u8; 3] = [0x7a, 0x4f, 0x21];
const BLUE: [u8; 3] = [0, 0, 255];
const RED: [u8; 3] = [255, 0, 0];

#[test]
fn a_click_on_the_pasteboard_takes_its_colour_in_every_sample_mode() {
    for source in SampleSource::ALL {
        let mut app = app();
        custom_pasteboard(&mut app);
        app.ui.tool_options.eyedropper.source = source;
        click(&mut app, -40.0, 150.0, Modifiers::NONE);
        assert_eq!(foreground(&app), CUSTOM, "{source:?}: the pasteboard as drawn");
    }
}

#[test]
fn the_themes_pasteboard_is_taken_as_drawn_too() {
    let mut app = app();
    click(&mut app, 200.0, -30.0, Modifiers::NONE);
    let canvas = app.ui.theme.tokens().canvas;
    assert_eq!(foreground(&app), [canvas.r(), canvas.g(), canvas.b()], "the theme's canvas surface");
    assert_ne!(foreground(&app), BLUE, "the click changed the foreground");
}

#[test]
fn a_drag_from_the_image_onto_the_pasteboard_ends_with_the_pasteboard_colour() {
    let mut app = app();
    custom_pasteboard(&mut app);
    tool_event(&mut app, ToolEvent::Down { x: 200.0, y: 150.0, pressure: 1.0 }, Modifiers::NONE);
    assert_eq!(foreground(&app), RED, "the press samples the image");
    tool_event(&mut app, ToolEvent::Move { x: 300.0, y: 150.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 440.0, y: 150.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 440.0, y: 150.0 }, Modifiers::NONE);
    assert_eq!(foreground(&app), CUSTOM, "the point the drag reached wins");
}

#[test]
fn on_the_image_the_eyedropper_still_samples_the_pixels() {
    let mut app = app();
    custom_pasteboard(&mut app);
    click(&mut app, 200.0, 150.0, Modifiers::NONE);
    assert_eq!(foreground(&app), RED);
}

/// Alt (the background) outside the image is not measured: it keeps picking nothing there.
#[test]
fn an_alt_click_on_the_pasteboard_keeps_the_background() {
    let mut app = app();
    custom_pasteboard(&mut app);
    click(&mut app, -40.0, 150.0, Modifiers::ALT);
    assert_eq!(rgb(app.session.tools.background), [255, 255, 255]);
    assert_eq!(foreground(&app), BLUE);
}

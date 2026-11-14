//! #12534: with the Paint Bucket, ⇧-click on the pasteboard sets the pasteboard colour to the
//! foreground colour, as the reference application does: the custom colour becomes the foreground
//! and the pasteboard right-click menu shows Custom checked. The press adds no history state and
//! leaves the document alone. ⇧-click on the image still fills, and the gestures the issue did not
//! measure (other modifiers held, a selection active) keep today's behaviour.

use egui::Modifiers;
use photocraft_engine::prefs::CanvasColor;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::canvas_tool_menu::{mode_checked, open_pasteboard};
use crate::state::Tool;

/// The foreground the tests pick: not the default black, so a fill and the pasteboard colour show it.
const FOREGROUND: &str = "#3366cc";

/// A 200 x 150 white document with the Paint Bucket and a blue foreground.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.sync_views();
    app.run("tools.setColors", json!({"foreground": FOREGROUND})).unwrap();
    app.ui.tool = Tool::PaintBucket;
    app
}

fn click(app: &mut PhotocraftApp, p: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: p[0], y: p[1] }, mods);
}

fn pasteboard(app: &PhotocraftApp) -> (CanvasColor, String) {
    let i = &app.session.prefs().interface;
    (i.canvas_color, i.canvas_custom_color.clone())
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn revision(app: &PhotocraftApp) -> u64 {
    app.session.active().unwrap().revision
}

fn pixel(app: &PhotocraftApp, x: i32, y: i32) -> [u8; 3] {
    let st = app.session.active().unwrap();
    let c = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y);
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as u8)
}

#[test]
fn shift_click_on_the_pasteboard_sets_the_custom_colour_to_the_foreground() {
    // Above-left of the image and past its bottom-right corner: both are pasteboard.
    for p in [[-30.0, -30.0], [230.0, 170.0]] {
        let mut app = app();
        assert_eq!(pasteboard(&app).0, CanvasColor::Default);
        click(&mut app, p, Modifiers::SHIFT);
        assert_eq!(pasteboard(&app), (CanvasColor::Custom, FOREGROUND.to_string()), "at {p:?}");
        // The pasteboard menu then shows Custom checked, and only Custom.
        assert!(open_pasteboard(&mut app, &egui::Context::default(), Tool::PaintBucket, [10.0, 10.0]));
        let menu = app.ui.canvas_tool_menu.clone().unwrap();
        assert!(mode_checked(&app, &menu, "pasteboard.custom"));
        assert!(!mode_checked(&app, &menu, "pasteboard.default"));
    }
}

#[test]
fn shift_click_on_the_pasteboard_adds_no_history_state_and_keeps_the_document() {
    let mut app = app();
    let (before_steps, before_revision) = (steps(&app), revision(&app));
    click(&mut app, [-30.0, -30.0], Modifiers::SHIFT);
    assert_eq!(steps(&app), before_steps, "no history state");
    assert_eq!(revision(&app), before_revision, "the document did not change");
    assert!(!app.session.active().unwrap().is_dirty());
    assert_eq!(pixel(&app, 0, 0), [255, 255, 255]);
}

#[test]
fn shift_click_on_the_image_fills_as_a_plain_click_does() {
    let mut app = app();
    let before = steps(&app);
    click(&mut app, [50.0, 50.0], Modifiers::SHIFT);
    assert_eq!(steps(&app), before + 1, "the fill was applied");
    assert_eq!(pixel(&app, 50, 50), [0x33, 0x66, 0xcc]);
    assert_eq!(pasteboard(&app).0, CanvasColor::Default, "the pasteboard colour is untouched");
}

#[test]
fn unmeasured_pasteboard_gestures_keep_the_pasteboard_colour() {
    // A plain click, ⇧ with another modifier, and ⇧ with a selection active are not this gesture.
    for (mods, select) in
        [(Modifiers::NONE, false), (Modifiers::SHIFT | Modifiers::ALT, false), (Modifiers::SHIFT | Modifiers::COMMAND, false), (Modifiers::SHIFT, true)]
    {
        let mut app = app();
        if select {
            app.run("select.rect", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
        }
        click(&mut app, [-30.0, -30.0], mods);
        assert_eq!(pasteboard(&app).0, CanvasColor::Default, "{mods:?}, selection {select}");
    }
}

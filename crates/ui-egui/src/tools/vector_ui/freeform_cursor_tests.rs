use craft::behavior::{Cursor, PaintedCursor};
use craft::ui::runtime::windows::WindowHost;
use egui::Modifiers;
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::guide_cursor_tests::{harness, hover};
use crate::state::Tool;

fn canvas_cursor(app: &mut Harness<'static, Box<PhotocraftApp>>) -> Option<Cursor> {
    hover(app, 100.0, 150.0, Modifiers::NONE);
    app.state().canvas_tools.last_response.as_ref()?.cursor.clone()
}

#[test]
fn freeform_hover_switches_between_its_nib_and_the_precise_crosshair() {
    let mut app = harness(Tool::FreeformPen);
    let nib = Some(Cursor::Painted(PaintedCursor::tool(crate::icons::tool_icon(Tool::FreeformPen), egui::vec2(3.0, 5.0), None)));
    let crosshair = Some(Cursor::Painted(PaintedCursor::Crosshair));
    assert_eq!(canvas_cursor(&mut app), nib);

    WindowHost::of(&app.ctx).set_caps_lock(Some(true));
    assert_eq!(canvas_cursor(&mut app), crosshair);
    WindowHost::of(&app.ctx).set_caps_lock(Some(false));
    assert_eq!(canvas_cursor(&mut app), nib);

    app.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "precise"})).unwrap();
    assert_eq!(canvas_cursor(&mut app), crosshair);
}

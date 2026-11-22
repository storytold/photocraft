//! #12578: over the canvas the Red Eye cursor is a small crosshair with an eye to its lower right,
//! the hotspot on the cross centre, and no pupil search circle at any Pupil Size. Precise Other
//! Cursors and Caps Lock keep the plain crosshair. The toolbar glyph is the eye with a small "+"
//! at its top left. Driven through real egui input on the real canvas; the assertions read the
//! cursor the tool asked the tool host for.

use craft::behavior::{Cursor, IconId, PaintedCursor, PointerGlyph};
use craft::ui::runtime::windows::WindowHost;
use egui::Modifiers;
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::guide_cursor_tests::{harness, hover};
use crate::state::Tool;

/// A point away from the harness's guide at x = 200.
const AT: [f32; 2] = [100.0, 150.0];

/// The cursor the tool host was asked for at `AT`.
fn cursor(h: &mut Harness<'static, Box<PhotocraftApp>>) -> Option<Cursor> {
    hover(h, AT[0], AT[1], Modifiers::NONE);
    h.state().canvas_tools.last_response.as_ref().and_then(|r| r.cursor.clone())
}

/// Holds Caps Lock off, so the keyboard's own Caps Lock doesn't swap the cursor for the precise one.
fn caps_lock_off(h: &Harness<'static, Box<PhotocraftApp>>) {
    WindowHost::of(&h.ctx).set_caps_lock(Some(false));
}

fn crosshair_with_eye() -> Cursor {
    Cursor::Painted(PaintedCursor::pointer_tool(PointerGlyph::Crosshair, IconId::new("eye")))
}

#[test]
fn the_cursor_is_the_crosshair_with_an_eye_at_any_pupil_size() {
    for size in [1.0, 50.0, 100.0] {
        let mut h = harness(Tool::RedEye);
        caps_lock_off(&h);
        h.state_mut().ui.tool_options.red_eye_pupil_size = size;
        h.run_steps(2);
        assert_eq!(cursor(&mut h), Some(crosshair_with_eye()), "Pupil Size {size}");
    }
}

#[test]
fn precise_other_cursors_and_caps_lock_keep_the_crosshair() {
    let mut h = harness(Tool::RedEye);
    caps_lock_off(&h);
    let crosshair = Some(Cursor::Painted(PaintedCursor::Crosshair));
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "precise"})).unwrap();
    assert_eq!(cursor(&mut h), crosshair, "Precise: the crosshair");
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "standard"})).unwrap();
    WindowHost::of(&h.ctx).set_caps_lock(Some(true));
    assert_eq!(cursor(&mut h), crosshair, "Caps Lock: the same crosshair");
}

#[test]
fn the_toolbar_glyph_is_the_red_eye_icon() {
    assert_eq!(crate::icons::tool_icon(Tool::RedEye), IconId::new("red-eye"));
    assert!(crate::icon_data::ICONS.iter().any(|(name, _)| *name == "red-eye"), "the glyph is in PhotoCraft's icon set");
}

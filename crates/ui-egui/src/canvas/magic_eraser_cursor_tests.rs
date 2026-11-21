//! #12001: the Magic Eraser's cursor is its own toolbar icon with the hotspot on the eraser's end at the lower left,
//! also with Shift held; Precise Other Cursors and Caps Lock give the crosshair. Driven through real egui input on
//! the real canvas; the assertions read the cursor the tool asked the tool host for.

use craft::behavior::{Cursor, PaintedCursor};
use craft::ui::runtime::windows::WindowHost;
use egui::Modifiers;
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::guide_cursor_tests::{harness, hover};
use crate::state::Tool;

/// A point away from the harness's guide at x = 200.
const AT: [f32; 2] = [100.0, 150.0];

fn cursor_with(h: &mut Harness<'static, Box<PhotocraftApp>>, mods: Modifiers) -> Option<Cursor> {
    hover(h, AT[0], AT[1], mods);
    h.state().canvas_tools.last_response.as_ref().and_then(|r| r.cursor.clone())
}

#[test]
fn the_standard_cursor_is_the_magic_eraser_icon_with_its_tip_at_the_lower_left() {
    let mut h = harness(Tool::MagicEraser);
    let Some(Cursor::Painted(PaintedCursor::Tool { icon, hotspot, badge, .. })) = cursor_with(&mut h, Modifiers::NONE) else {
        panic!("an icon cursor, not the crosshair");
    };
    assert_eq!(icon, crate::icons::tool_icon(Tool::MagicEraser), "the toolbar's own icon");
    // The eraser end of the glyph, at the lower left of its 24-unit box: pinned by value so a moved constant fails.
    assert_eq!(hotspot, egui::vec2(4.6, 18.4), "the eraser tip");
    assert!(badge.is_none());
    assert_eq!(cursor_with(&mut h, Modifiers::SHIFT), cursor_with(&mut h, Modifiers::NONE), "Shift keeps it");
}

#[test]
fn precise_other_cursors_and_caps_lock_give_the_crosshair() {
    let mut h = harness(Tool::MagicEraser);
    let crosshair = Some(Cursor::Painted(PaintedCursor::Crosshair));
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "precise"})).unwrap();
    assert_eq!(cursor_with(&mut h, Modifiers::NONE), crosshair, "Precise: the crosshair");
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "standard"})).unwrap();
    assert_ne!(cursor_with(&mut h, Modifiers::NONE), crosshair, "Standard again: the icon");
    WindowHost::of(&h.ctx).set_caps_lock(Some(true));
    assert_eq!(cursor_with(&mut h, Modifiers::NONE), crosshair, "Caps Lock: the crosshair");
}

//! The Paint Bucket cursor (#12539): with Standard Other Cursors it is the arrow with the bucket
//! below right of it, the hotspot on the arrow tip, and a click fills the pixel under that tip.
//! Precise Other Cursors and Caps Lock keep the crosshair; ⌥ and ⌘ keep the Eyedropper and Move
//! cursors. Driven through real egui input on the real canvas; the assertions read the cursor the
//! tool (or its stand-in) asked the tool host for.

use craft::behavior::{Cursor, PaintedCursor, PointerGlyph};
use craft::ui::runtime::windows::WindowHost;
use egui::{Modifiers, PointerButton};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::guide_cursor_tests::{harness, hover, screen};
use crate::state::Tool;

/// A point away from the harness's guide at x = 200.
const AT: [f32; 2] = [100.0, 150.0];

/// The cursor the tool host was asked for at `AT` with `mods` held.
fn cursor_with(h: &mut Harness<'static, Box<PhotocraftApp>>, mods: Modifiers) -> Option<Cursor> {
    hover(h, AT[0], AT[1], mods);
    h.state().canvas_tools.last_response.as_ref().and_then(|r| r.cursor.clone())
}

fn pixel(app: &PhotocraftApp, x: i32, y: i32) -> [u8; 3] {
    let st = app.session.active().unwrap();
    let c = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y);
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as u8)
}

#[test]
fn the_standard_cursor_is_the_arrow_with_the_bucket() {
    let mut h = harness(Tool::PaintBucket);
    let bucket = Cursor::Painted(PaintedCursor::pointer_tool(PointerGlyph::Arrow, crate::icons::tool_icon(Tool::PaintBucket)));
    assert_eq!(cursor_with(&mut h, Modifiers::NONE), Some(bucket));
}

#[test]
fn a_click_fills_the_pixel_under_the_arrow_tip() {
    let mut h = harness(Tool::PaintBucket);
    let (x, y) = (AT[0] as i32, AT[1] as i32);
    // Black left of and above the pointer's pixel: a fill there stops at the edges, a fill one
    // pixel off to the left or up would turn the black red.
    let app = h.state_mut();
    for rect in [[0, 0, x, 300], [0, 0, 400, y]] {
        app.run("select.rect", json!({"x": rect[0], "y": rect[1], "width": rect[2], "height": rect[3]})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#000000"})).unwrap();
    }
    app.run("select.deselect", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
    h.run_steps(2);
    // The pixel's centre, as the pointer sits on the arrow tip.
    let p = screen(&h, AT[0] + 0.5, AT[1] + 0.5);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
    assert_eq!(pixel(h.state(), x, y), [255, 0, 0], "the pixel under the tip is filled");
    assert_eq!(pixel(h.state(), x - 1, y), [0, 0, 0], "the pixel left of it is not");
    assert_eq!(pixel(h.state(), x, y - 1), [0, 0, 0], "the pixel above it is not");
}

#[test]
fn precise_other_cursors_and_caps_lock_keep_the_crosshair() {
    let mut h = harness(Tool::PaintBucket);
    let crosshair = Some(Cursor::Painted(PaintedCursor::Crosshair));
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "precise"})).unwrap();
    assert_eq!(cursor_with(&mut h, Modifiers::NONE), crosshair, "Precise: the crosshair");
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "standard"})).unwrap();
    WindowHost::of(&h.ctx).set_caps_lock(Some(true));
    assert_eq!(cursor_with(&mut h, Modifiers::NONE), crosshair, "Caps Lock: the same crosshair");
}

#[test]
fn alt_and_command_keep_the_eyedropper_and_move_cursors() {
    let mut bucket = harness(Tool::PaintBucket);
    let standard = cursor_with(&mut bucket, Modifiers::NONE);
    let eyedropper = cursor_with(&mut harness(Tool::Eyedropper), Modifiers::NONE);
    let alt = cursor_with(&mut bucket, Modifiers::ALT);
    assert_eq!(alt, eyedropper, "⌥: the Eyedropper cursor");
    assert_ne!(alt, standard);
    let mv = cursor_with(&mut harness(Tool::Move), Modifiers::NONE);
    let command = cursor_with(&mut bucket, Modifiers::COMMAND);
    assert_eq!(command, mv, "⌘: the Move cursor");
    assert_ne!(command, standard);
}

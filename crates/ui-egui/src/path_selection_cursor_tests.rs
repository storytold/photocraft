//! #13295: Path Selection's pointer over the canvas is its own arrow with the tip on the pointer,
//! not the crosshair: the light arrow idle and with ⌘, the solid one with ⌥ over a component (of
//! the target path or of a shape layer). Driven through real egui input on the real canvas; the
//! assertions read the cursor the tool asked the tool host for.

use craft::behavior::{Cursor, PaintedCursor};
use craft::keymap::Mods;
use craft::ui::runtime::windows::WindowHost;
use egui::{Modifiers, PointerButton};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::guide_cursor_tests::{harness, hover, screen};
use crate::state::Tool;

const LIGHT: Option<Cursor> = Some(Cursor::Painted(PaintedCursor::arrow(true)));
const SOLID: Option<Cursor> = Some(Cursor::Painted(PaintedCursor::arrow(false)));
const CROSSHAIR: Option<Cursor> = Some(Cursor::Painted(PaintedCursor::Crosshair));

/// Path Selection over a work path with one square component, 60–160 on both axes (away from the
/// harness's guide at x = 200).
fn square() -> Harness<'static, PhotocraftApp> {
    let mut h = harness(Tool::PathSelection);
    h.state_mut()
        .run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": [[60, 60], [160, 60], [160, 160], [60, 160]]}]}}))
        .unwrap();
    h.run_steps(1);
    h
}

/// The cursor the selected tool asked for in the last pass.
fn tool_cursor(h: &Harness<'static, PhotocraftApp>) -> Option<Cursor> {
    h.state().canvas_tools.last_response.as_ref().and_then(|r| r.cursor.clone())
}

/// The tool's cursor at doc point (`x`, `y`) with `m` held.
fn pointer(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32, m: Modifiers) -> Option<Cursor> {
    hover(h, x, y, m);
    tool_cursor(h)
}

/// Two shape layers, 40,40–140,120 and 230,180–350,260, the second one active.
fn two_shapes(h: &mut Harness<'static, PhotocraftApp>) {
    let app = h.state_mut();
    app.run("shape.create", json!({"kind": "rect", "rect": [40, 40, 100, 80], "fill": "#e02020"})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [230, 180, 120, 80], "fill": "#2050e0"})).unwrap();
}

#[test]
fn alt_over_a_component_is_the_solid_arrow() {
    let mut h = square();
    assert_eq!(pointer(&mut h, 60.0, 110.0, Modifiers::ALT), SOLID, "⌥ over the subpath's outline");
    assert_eq!(pointer(&mut h, 110.0, 110.0, Modifiers::ALT), SOLID, "⌥ inside the component");
    assert_eq!(pointer(&mut h, 300.0, 250.0, Modifiers::ALT), LIGHT, "⌥ off every component was not measured: the light arrow");
    // Window › Modifier Keys: a latched ⌥ is ⌥ too.
    h.state_mut().ui.shell.latched = Mods { alt: true, ..Mods::NONE };
    assert_eq!(pointer(&mut h, 110.0, 110.0, Modifiers::NONE), SOLID, "latched ⌥ inside the component");
    h.state_mut().ui.shell.latched = Mods::NONE;
    assert_eq!(pointer(&mut h, 110.0, 110.0, Modifiers::ALT | Modifiers::COMMAND), LIGHT, "⌘⌥ was not measured: the light arrow");
}

/// An ⌥-drag that starts on the component keeps the solid arrow once the pointer leaves the
/// component's stored outline: it moves only on release.
#[test]
fn an_alt_drag_keeps_the_solid_arrow_off_the_component() {
    let mut h = square();
    hover(&mut h, 110.0, 110.0, Modifiers::ALT);
    let p = screen(&h, 110.0, 110.0);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::ALT });
    h.run_steps(1);
    for k in 1..=4 {
        let t = k as f32 / 4.0;
        h.event(egui::Event::PointerMoved(screen(&h, 110.0 + 190.0 * t, 110.0 + 140.0 * t)));
        h.run_steps(1);
    }
    h.run_steps(1);
    assert!(crate::canvas::testing::in_gesture(h.state()), "still dragging");
    assert_eq!(tool_cursor(&h), SOLID, "off the component, mid-drag");
    let end = screen(&h, 300.0, 250.0);
    h.event(egui::Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::ALT });
    h.run_steps(2);
}

/// A press with ⌥ over another shape layer selects that layer and lands on its component, so the
/// cursor shows the solid arrow there too.
#[test]
fn alt_over_an_inactive_shape_layer_is_the_solid_arrow() {
    let mut h = harness(Tool::PathSelection);
    two_shapes(&mut h);
    assert_eq!(pointer(&mut h, 90.0, 80.0, Modifiers::ALT), SOLID, "⌥ inside the inactive shape");
    assert_eq!(pointer(&mut h, 290.0, 220.0, Modifiers::ALT), SOLID, "⌥ inside the active shape");
    assert_eq!(pointer(&mut h, 300.0, 80.0, Modifiers::ALT), LIGHT, "⌥ off both shapes");
}

#[test]
fn precise_other_cursors_and_caps_lock_keep_the_crosshair() {
    let mut h = square();
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "precise"})).unwrap();
    assert_eq!(pointer(&mut h, 110.0, 110.0, Modifiers::NONE), CROSSHAIR, "Precise: the crosshair");
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "standard"})).unwrap();
    WindowHost::of(&h.ctx).set_caps_lock(Some(true));
    assert_eq!(pointer(&mut h, 110.0, 110.0, Modifiers::NONE), CROSSHAIR, "Caps Lock: the crosshair");
}

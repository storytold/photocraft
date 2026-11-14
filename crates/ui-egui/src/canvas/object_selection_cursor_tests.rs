//! #12865: the Object Selection cursor stays the same with ⇧, ⌥ or ⇧⌥ held: no + / − / × badge,
//! though the keys still add, subtract and intersect. In Rectangle mode it is a crosshair with a
//! small dotted rectangle at its lower right. The other selection tools keep their badges. Tests
//! drive the real canvas (`document_area`) and read the OS cursor bitmap it hands the platform.

use egui::{CursorIcon, Modifiers};
use egui_kittest::Harness;
use serde_json::json;

use super::remove_cursor_tests::{Cursor, cursor, harness, set_pref};
use crate::PhotocraftApp;
use crate::state::Tool;
use crate::tool_feedback::{Badge, badge, selection_mode};

const SHIFT_ALT: Modifiers = Modifiers { alt: true, shift: true, ctrl: false, mac_cmd: false, command: false };

/// The cursor with `mods` held over the canvas centre.
fn cursor_with(h: &mut Harness<'_, PhotocraftApp>, mods: Modifiers) -> Cursor {
    let p = h.state().last_canvas_rect.center();
    h.event(egui::Event::ModifiersChanged(mods));
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(3);
    cursor(h)
}

#[test]
fn object_selection_shows_no_badge_for_shift_alt_or_both() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    for selected in [false, true] {
        if selected {
            app.run("select.rect", json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
        }
        app.ui.selection_mode = 0;
        for m in [Modifiers::NONE, Modifiers::SHIFT, Modifiers::ALT, SHIFT_ALT] {
            assert_eq!(badge(&app, Tool::ObjectSelection, m), None, "{m:?}, selection {selected}");
        }
        // The keys still choose what the gesture does.
        assert_eq!(selection_mode(Tool::ObjectSelection, 0, Modifiers::SHIFT), "add");
        assert_eq!(selection_mode(Tool::ObjectSelection, 0, Modifiers::ALT), "subtract");
        assert_eq!(selection_mode(Tool::ObjectSelection, 0, SHIFT_ALT), "intersect");
        // The options bar's own mode still shows (#12853), whatever key is held.
        app.ui.selection_mode = 1;
        assert_eq!(badge(&app, Tool::ObjectSelection, Modifiers::NONE), Some(Badge::Add));
        assert_eq!(badge(&app, Tool::ObjectSelection, Modifiers::ALT), Some(Badge::Add));
    }
}

#[test]
fn the_other_selection_tools_keep_their_badges() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("select.rect", json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    app.ui.selection_mode = 0;
    for tool in [Tool::RectMarquee, Tool::EllipseMarquee, Tool::Lasso, Tool::PolygonLasso, Tool::MagicWand] {
        assert_eq!(badge(&app, tool, Modifiers::SHIFT), Some(Badge::Add), "{tool:?}");
        assert_eq!(badge(&app, tool, Modifiers::ALT), Some(Badge::Subtract), "{tool:?}");
        assert_eq!(badge(&app, tool, SHIFT_ALT), Some(Badge::Intersect), "{tool:?}");
    }
    assert_eq!(badge(&app, Tool::QuickSelection, Modifiers::ALT), Some(Badge::Subtract));
    assert_eq!(badge(&app, Tool::Patch, Modifiers::SHIFT), Some(Badge::Add));
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn the_cursor_is_a_crosshair_with_a_dotted_rectangle_at_its_lower_right() {
    let h = harness(Tool::ObjectSelection);
    let (_, image) = cursor(&h);
    let (size, [hx, hy], rgba) = image.expect("Object Selection's OS cursor bitmap");
    let w = usize::from(size[0]);
    let opaque = |x: usize, y: usize| rgba.get((y * w + x) * 4 + 3).is_some_and(|a| *a > 200);
    let (hx, hy) = (usize::from(hx), usize::from(hy));
    let scale = (size[0] as f32 / 30.0).max(1.0);
    let at = |d: f32| (d * scale).round() as usize;
    // The arms of the crosshair, left and below the hotspot.
    assert!((1..=at(6.0)).any(|d| opaque(hx - d, hy)), "a left arm");
    assert!((1..=at(6.0)).any(|d| opaque(hx, hy + d)), "a lower arm");
    // Something drawn in the lower right quadrant, off both arms: the dotted rectangle.
    let quadrant = (at(3.0)..at(13.0)).any(|dx| (at(3.0)..at(11.0)).any(|dy| opaque(hx + dx, hy + dy)));
    assert!(quadrant, "the dotted rectangle at the lower right");
    // The plain crosshair the marquee shows leaves that quadrant empty.
    let marquee = cursor(&harness(Tool::RectMarquee));
    assert_ne!(marquee, cursor(&h), "not the marquee's crosshair");
}

#[test]
fn the_cursor_stays_the_same_with_shift_alt_or_both() {
    let mut h = harness(Tool::ObjectSelection);
    let plain = cursor_with(&mut h, Modifiers::NONE);
    for m in [Modifiers::SHIFT, Modifiers::ALT, SHIFT_ALT] {
        assert_eq!(cursor_with(&mut h, m), plain, "{m:?}");
    }
}

#[test]
fn precise_other_cursors_keep_the_plain_crosshair() {
    let mut h = harness(Tool::ObjectSelection);
    let mut marquee = harness(Tool::RectMarquee);
    set_pref(&mut h, "cursors.other", json!("precise"));
    set_pref(&mut marquee, "cursors.other", json!("precise"));
    assert_eq!(cursor(&h), cursor(&marquee));
    assert_eq!(cursor(&h).0, CursorIcon::Crosshair);
}

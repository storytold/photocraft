//! #12610: with Painting Cursors = Normal Brush Tip, holding ⌥ with the Clone Stamp shows the fixed
//! sample target the Healing Brush shows (#12113), centred on the pointer whatever the tip size,
//! instead of the tip with a centre mark; releasing ⌥ brings the tip outline back. Standard and
//! Precise keep the cursors they had. Driven through real egui input on the real canvas; the
//! assertions read the cursor the tool asked the tool host for.

use craft::behavior::{Cursor, PaintedCursor};
use craft::keymap::Mods;
use egui::Modifiers;
use egui_kittest::Harness;
use serde_json::json;

use super::sample_target;
use crate::PhotocraftApp;
use crate::guide_cursor_tests::{harness, hover};
use crate::state::Tool;

/// A point away from the harness's guide at x = 200.
const AT: [f32; 2] = [100.0, 150.0];

/// The cursor the tool host was asked for at `AT` with `m` held.
fn cursor(h: &mut Harness<'static, Box<PhotocraftApp>>, m: Modifiers) -> Option<Cursor> {
    hover(h, AT[0], AT[1], m);
    h.state().canvas_tools.last_response.as_ref().and_then(|r| r.cursor.clone())
}

/// Whether a brush-tip cursor draws its centre mark; `None` for any other cursor.
fn centre_marked(c: &Option<Cursor>) -> Option<bool> {
    match c {
        Some(Cursor::Painted(PaintedCursor::Circle { centre, .. } | PaintedCursor::Outline { centre, .. })) => Some(*centre),
        _ => None,
    }
}

fn tip_size(h: &mut Harness<'static, Box<PhotocraftApp>>, size: f32) {
    h.state_mut().session.tools.brush.size = size;
}

fn painting(h: &mut Harness<'static, Box<PhotocraftApp>>, value: &str) {
    h.state_mut().run("prefs.set", json!({"path": "cursors.painting", "value": value})).unwrap();
}

#[test]
fn alt_shows_the_target_at_any_tip_size() {
    let mut h = harness(Tool::CloneStamp);
    for size in [10.0, 40.0, 300.0] {
        tip_size(&mut h, size);
        assert_eq!(cursor(&mut h, Modifiers::ALT), Some(sample_target()), "{size} px: ⌥ shows the target");
    }
    h.state_mut().ui.shell.latched = Mods { alt: true, ..Mods::NONE };
    assert_eq!(cursor(&mut h, Modifiers::NONE), Some(sample_target()), "latched ⌥");
    h.state_mut().ui.shell.latched = Mods::NONE;
}

#[test]
fn releasing_alt_brings_the_tip_outline_back() {
    let mut h = harness(Tool::CloneStamp);
    tip_size(&mut h, 40.0);
    let plain = cursor(&mut h, Modifiers::NONE);
    assert_eq!(centre_marked(&plain), Some(false), "the tip outline, no centre mark");
    assert_eq!(cursor(&mut h, Modifiers::ALT), Some(sample_target()), "⌥");
    assert_eq!(cursor(&mut h, Modifiers::NONE), plain, "released: the tip outline again");
}

/// Standard and Precise were not measured with the Clone Stamp's ⌥, so ⌥ shows what it did; Full
/// Size Brush Tip keeps the tip with its centre mark.
#[test]
fn unmeasured_painting_cursors_keep_their_alt_cursor() {
    let mut h = harness(Tool::CloneStamp);
    tip_size(&mut h, 100.0);
    for value in ["standard", "precise"] {
        painting(&mut h, value);
        let plain = cursor(&mut h, Modifiers::NONE);
        assert_eq!(cursor(&mut h, Modifiers::ALT), plain, "{value}: ⌥ changes nothing");
    }
    painting(&mut h, "fullSizeTip");
    let full = cursor(&mut h, Modifiers::ALT);
    assert_ne!(full, Some(sample_target()), "fullSizeTip: not the target");
    assert_eq!(centre_marked(&full), Some(true), "fullSizeTip: the tip with its centre mark");
}

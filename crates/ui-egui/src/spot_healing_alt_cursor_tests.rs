//! #12561: with Painting Cursors = Normal Brush Tip, holding ⌥ with the Spot Healing Brush shows the
//! same fixed sample target as the Healing Brush (#12113), whatever the tip size, though its
//! ⌥-click only alerts; releasing ⌥ brings the tip outline back. Ctrl+⌥ (the brush resize drag),
//! the unmeasured Painting Cursors settings and the other brushes keep the cursors they had. Driven
//! through real egui input on the real canvas; the assertions read the cursor the tool asked the
//! tool host for.

use craft::behavior::{Cursor, CursorIcon, PaintedCursor};
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

/// Ctrl+⌥: the brush resize drag's chord.
const CTRL_ALT: Modifiers = Modifiers { alt: true, ctrl: true, shift: false, mac_cmd: false, command: false };

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
fn alt_shows_the_healing_brushs_target_at_any_tip_size_and_releasing_brings_the_tip_back() {
    let mut h = harness(Tool::SpotHealing);
    for size in [10.0, 40.0, 200.0] {
        tip_size(&mut h, size);
        let plain = cursor(&mut h, Modifiers::NONE);
        assert_ne!(plain, Some(sample_target()), "{size} px: not the target without ⌥");
        assert_eq!(cursor(&mut h, Modifiers::ALT), Some(sample_target()), "{size} px: ⌥ shows the target");
        assert_eq!(cursor(&mut h, Modifiers::NONE), plain, "{size} px: releasing ⌥ brings the tip back");
    }
    tip_size(&mut h, 40.0);
    assert_eq!(centre_marked(&cursor(&mut h, Modifiers::NONE)), Some(false), "40 px: the tip outline, no centre mark");
    // The very cursor the Healing Brush shows with ⌥.
    h.state_mut().ui.tool = Tool::Healing;
    assert_eq!(cursor(&mut h, Modifiers::ALT), Some(sample_target()), "the Healing Brush's ⌥ cursor");
}

#[test]
fn latched_alt_shows_the_target_too() {
    let mut h = harness(Tool::SpotHealing);
    tip_size(&mut h, 120.0);
    h.state_mut().ui.shell.latched = Mods { alt: true, ..Mods::NONE };
    assert_eq!(cursor(&mut h, Modifiers::NONE), Some(sample_target()), "latched ⌥");
    h.state_mut().ui.shell.latched = Mods::NONE;
    assert_ne!(cursor(&mut h, Modifiers::NONE), Some(sample_target()), "off again: the tip");
}

/// Ctrl+⌥ is the brush resize drag's chord: the tip stays as it is today.
#[test]
fn ctrl_alt_keeps_the_tip() {
    let mut h = harness(Tool::SpotHealing);
    tip_size(&mut h, 40.0);
    let plain = cursor(&mut h, Modifiers::NONE);
    let chord = cursor(&mut h, CTRL_ALT);
    assert_ne!(chord, Some(sample_target()), "Ctrl+⌥: not the target");
    // Only on macOS is Ctrl a key of its own. Elsewhere Ctrl is the command key, so Ctrl+⌥ is ⌘⌥: the quick pick to the
    // Move tool, whose ⌥ cursor is the copy cursor (CI on Linux and Windows showed `System(Copy)`).
    if cfg!(target_os = "macos") {
        assert_eq!(chord, plain, "Ctrl+⌥: the tip outline as today");
    } else {
        assert_eq!(chord, Some(Cursor::System(CursorIcon::Copy)), "Ctrl+⌥ is ⌘⌥: the Move tool's copy cursor");
    }
}

/// Standard and Precise were not measured with the Spot Healing Brush's ⌥, so ⌥ changes nothing.
#[test]
fn unmeasured_painting_cursors_keep_their_cursor() {
    let mut h = harness(Tool::SpotHealing);
    tip_size(&mut h, 100.0);
    for value in ["standard", "precise", "fullSizeTip"] {
        painting(&mut h, value);
        let plain = cursor(&mut h, Modifiers::NONE);
        assert_eq!(cursor(&mut h, Modifiers::ALT), plain, "{value}: ⌥ changes nothing");
    }
}

#[test]
fn other_brushes_keep_their_alt_cursor() {
    for tool in [Tool::Brush, Tool::PatternStamp, Tool::Eraser, Tool::Remove] {
        let mut h = harness(tool);
        tip_size(&mut h, 40.0);
        assert_ne!(cursor(&mut h, Modifiers::ALT), Some(sample_target()), "{tool:?}: ⌥ is not the target");
    }
}

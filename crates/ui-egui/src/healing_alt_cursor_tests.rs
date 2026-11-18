//! #12113: with Painting Cursors = Normal Brush Tip, holding ⌥ with the Healing Brush shows the
//! reference's fixed sample target whatever the tip size, and releasing ⌥ brings the tip outline
//! back. The Clone Stamp keeps its tip with a centre mark, and the unmeasured Painting Cursors
//! settings keep the cursors they had. Driven through real egui input on the real canvas; the
//! assertions read the cursor the tool (or its ⌥ stand-in) asked the tool host for.

use craft::behavior::{Cursor, PaintedCursor};
use craft::keymap::Mods;
use egui::{Modifiers, PointerButton};
use egui_kittest::Harness;
use serde_json::json;

use super::sample_target;
use crate::PhotocraftApp;
use crate::guide_cursor_tests::{harness, hover, screen};
use crate::state::Tool;

/// A point away from the harness's guide at x = 200.
const AT: [f32; 2] = [100.0, 150.0];

/// The cursor the tool host was asked for at `AT` with `m` held.
fn cursor(h: &mut Harness<'static, PhotocraftApp>, m: Modifiers) -> Option<Cursor> {
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

/// A brush-tip cursor without its centre mark and badge: the tip's shape alone.
fn tip(c: &Option<Cursor>) -> Option<PaintedCursor> {
    match c {
        Some(Cursor::Painted(p @ (PaintedCursor::Circle { .. } | PaintedCursor::Outline { .. }))) => Some(p.clone().marked(false, None)),
        _ => None,
    }
}

fn tip_size(h: &mut Harness<'static, PhotocraftApp>, size: f32) {
    h.state_mut().session.tools.brush.size = size;
}

fn painting(h: &mut Harness<'static, PhotocraftApp>, value: &str) {
    h.state_mut().run("prefs.set", json!({"path": "cursors.painting", "value": value})).unwrap();
}

#[test]
fn latched_alt_shows_the_target_too() {
    let mut h = harness(Tool::Healing);
    tip_size(&mut h, 120.0);
    h.state_mut().ui.shell.latched = Mods { alt: true, ..Mods::NONE };
    assert_eq!(cursor(&mut h, Modifiers::NONE), Some(sample_target()), "latched ⌥");
    h.state_mut().ui.shell.latched = Mods::NONE;
    assert_ne!(cursor(&mut h, Modifiers::NONE), Some(sample_target()), "off again: the tip");
}

#[test]
fn the_target_stays_while_an_alt_click_is_held() {
    let mut h = harness(Tool::Healing);
    tip_size(&mut h, 120.0);
    assert_eq!(cursor(&mut h, Modifiers::ALT), Some(sample_target()), "⌥ hover");
    let p = screen(&h, AT[0], AT[1]);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::ALT });
    h.run_steps(2);
    let held = h.state().canvas_tools.last_response.as_ref().and_then(|r| r.cursor.clone());
    assert_eq!(held, Some(sample_target()), "the target stays while the ⌥-click is held");
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::ALT });
    h.run_steps(2);
}

#[test]
fn the_clone_stamp_keeps_its_tip_with_a_centre_mark() {
    let mut h = harness(Tool::CloneStamp);
    tip_size(&mut h, 100.0);
    let plain = cursor(&mut h, Modifiers::NONE);
    assert_eq!(centre_marked(&plain), Some(false), "no centre mark without ⌥");
    let alt = cursor(&mut h, Modifiers::ALT);
    assert_eq!(centre_marked(&alt), Some(true), "⌥ adds the centre mark");
    assert_eq!(tip(&alt), tip(&plain), "the tip keeps the tip size");
}

/// Standard, Precise and Full Size Brush Tip were not measured with the Healing Brush's ⌥, so ⌥
/// shows what it did: the standard pointer, the precise crosshair, or the full-size tip with its
/// centre mark.
#[test]
fn unmeasured_painting_cursors_keep_their_alt_cursor() {
    let mut h = harness(Tool::Healing);
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

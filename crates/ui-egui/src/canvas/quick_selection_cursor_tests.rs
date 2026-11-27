//! #16822: with a selection, holding ⌥ with Quick Selection puts a "−" at the centre of its ring;
//! ⇧, or no selection, keeps the plain ring. Driven through real egui input on the real canvas; the
//! assertions read the cursor the tool asked the tool host for.

use craft::behavior::{Cursor, CursorBadge, PaintedCursor};
use egui::Modifiers;
use egui_kittest::Harness;
use serde_json::json;

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

/// The ring's badge, `Some(None)` for a ring without one; `None` for any other cursor.
fn ring_badge(c: &Option<Cursor>) -> Option<Option<CursorBadge>> {
    match c {
        Some(Cursor::Painted(PaintedCursor::Circle { badge, .. } | PaintedCursor::Outline { badge, .. })) => Some(*badge),
        _ => None,
    }
}

/// The ring without its centre mark and badge: the tip's shape alone.
fn ring(c: &Option<Cursor>) -> Option<PaintedCursor> {
    match c {
        Some(Cursor::Painted(p @ (PaintedCursor::Circle { .. } | PaintedCursor::Outline { .. }))) => Some(p.clone().marked(false, None)),
        _ => None,
    }
}

/// Quick Selection with a 100 px tip, over a document with a selection when `selected`.
fn quick_selection(selected: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = harness(Tool::QuickSelection);
    h.state_mut().session.tools.brush.size = 100.0;
    if selected {
        h.state_mut().run("select.rect", json!({"x": 20, "y": 20, "width": 120, "height": 120})).unwrap();
    }
    h
}

#[test]
fn alt_over_a_selection_marks_the_centre_of_the_ring() {
    let mut h = quick_selection(true);
    let plain = cursor(&mut h, Modifiers::NONE);
    assert_eq!(ring_badge(&plain), Some(None), "no key: the plain ring");
    let alt = cursor(&mut h, Modifiers::ALT);
    assert_eq!(ring_badge(&alt), Some(Some(CursorBadge::Subtract)), "⌥: the ring with a − at its centre");
    assert_eq!(ring(&alt), ring(&plain), "the ring itself is unchanged");
    assert_eq!(ring_badge(&cursor(&mut h, Modifiers::NONE)), Some(None), "⌥ released: the plain ring again");
}

/// ⇧ with a selection, and ⌥ with none, were measured to show the plain ring; ⇧⌥ was not measured and
/// keeps it.
#[test]
fn shift_or_no_selection_keeps_the_plain_ring() {
    let mut h = quick_selection(true);
    let plain = cursor(&mut h, Modifiers::NONE);
    for (m, what) in [(Modifiers::SHIFT, "⇧"), (Modifiers::SHIFT | Modifiers::ALT, "⇧⌥")] {
        assert_eq!(cursor(&mut h, m), plain, "{what} with a selection");
    }
    let mut h = quick_selection(false);
    let plain = cursor(&mut h, Modifiers::NONE);
    assert_eq!(ring_badge(&plain), Some(None), "the plain ring");
    assert_eq!(cursor(&mut h, Modifiers::ALT), plain, "⌥ with nothing selected");
}

/// The Standard and Precise painting cursors are no ring and were not measured: ⌥ leaves them as
/// they were.
#[test]
fn cursors_without_a_ring_keep_their_pointer() {
    let mut h = quick_selection(true);
    for value in ["standard", "precise"] {
        h.state_mut().run("prefs.set", json!({"path": "cursors.painting", "value": value})).unwrap();
        let plain = cursor(&mut h, Modifiers::NONE);
        assert_eq!(ring_badge(&plain), None, "{value}: no ring");
        assert_eq!(cursor(&mut h, Modifiers::ALT), plain, "{value}: ⌥ changes nothing");
    }
}

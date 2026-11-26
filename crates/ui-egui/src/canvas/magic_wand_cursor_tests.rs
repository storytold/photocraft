//! #16817: Magic Wand uses its wand-with-sparkles pointer, shows selection-operation badges only
//! when a selection exists, and switches to the precise crosshair with Precise Other Cursors or Caps Lock.

use craft::behavior::{Cursor, CursorBadge, PaintedCursor, PointerGlyph};
use craft::ui::runtime::windows::WindowHost;
use egui::Modifiers;
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::guide_cursor_tests::{harness, hover};
use crate::state::Tool;

const AT: [f32; 2] = [100.0, 150.0];

fn cursor(h: &mut Harness<'static, Box<PhotocraftApp>>, mods: Modifiers) -> Option<Cursor> {
    hover(h, AT[0], AT[1], mods);
    h.state().canvas_tools.last_response.as_ref().and_then(|response| response.cursor.clone())
}

fn wand_cursor(c: &Option<Cursor>) -> Option<(PointerGlyph, String, Option<CursorBadge>)> {
    match c {
        Some(Cursor::Painted(PaintedCursor::PointerTool { pointer, icon, badge, .. })) => Some((*pointer, icon.name().to_owned(), *badge)),
        _ => None,
    }
}

#[test]
fn wand_pointer_and_operation_badges_follow_selection_state() {
    let mut h = harness(Tool::MagicWand);
    assert_eq!(wand_cursor(&cursor(&mut h, Modifiers::NONE)), Some((PointerGlyph::Arrow, "wand_sparkles".to_owned(), None)));
    assert_eq!(
        wand_cursor(&cursor(&mut h, Modifiers::SHIFT)),
        Some((PointerGlyph::Arrow, "wand_sparkles".to_owned(), None)),
        "no selection: Shift has no combine badge"
    );

    h.state_mut().run("select.rect", json!({"x": 20, "y": 20, "width": 120, "height": 120})).unwrap();
    assert_eq!(wand_cursor(&cursor(&mut h, Modifiers::SHIFT)), Some((PointerGlyph::Arrow, "wand_sparkles".to_owned(), Some(CursorBadge::Add))));
    assert_eq!(wand_cursor(&cursor(&mut h, Modifiers::ALT)), Some((PointerGlyph::Arrow, "wand_sparkles".to_owned(), Some(CursorBadge::Subtract))));
    assert_eq!(
        wand_cursor(&cursor(&mut h, Modifiers::SHIFT | Modifiers::ALT)),
        Some((PointerGlyph::Arrow, "wand_sparkles".to_owned(), Some(CursorBadge::Intersect)))
    );
}

#[test]
fn precise_other_cursors_and_caps_lock_use_the_crosshair() {
    let mut h = harness(Tool::MagicWand);
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "precise"})).unwrap();
    assert_eq!(cursor(&mut h, Modifiers::NONE), Some(Cursor::Painted(PaintedCursor::Crosshair)));
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "standard"})).unwrap();
    assert_ne!(cursor(&mut h, Modifiers::NONE), Some(Cursor::Painted(PaintedCursor::Crosshair)));
    WindowHost::of(&h.ctx).set_caps_lock(Some(true));
    assert_eq!(cursor(&mut h, Modifiers::NONE), Some(Cursor::Painted(PaintedCursor::Crosshair)));
}

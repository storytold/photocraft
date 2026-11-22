//! #12744: the Convert Point tool's right-click opens the same path menu as the Delete Anchor
//! Point tool's (#12771), with no Convert Point entry. The shared steps live in
//! `delete_anchor_menu_tests`.

use egui::Modifiers;

use super::delete_anchor_menu_tests::{saved, setup};
use super::tests::{harness, right_click, screen_of};
use crate::canvas::{ToolEvent, tool_event};
use crate::direct_select::PathRef;
use crate::state::Tool;

/// Every anchor selected (a Direct Selection marquee), then Delete Anchor Point from the menu on
/// (300,300): the anchors shift, so no old anchor index stays selected, as after a Delete Anchor
/// Point click: `app.run` drops a Direct Selection whose revision moved on (`cancel_stale`).
#[test]
fn deleting_from_the_menu_leaves_no_shifted_anchor_selected() {
    let mut app = setup(Tool::DirectSelection);
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 250.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 750.0, y: 550.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 750.0, y: 550.0 }, Modifiers::NONE);
    assert!(app.ui.direct_selection.anchors.len() > 3, "every anchor is selected");
    app.ui.tool = Tool::ConvertPoint;
    let mut h = harness(app);
    assert!(!h.state().ui.direct_selection.anchors.is_empty(), "Convert Point keeps the selection");
    let p = screen_of(h.state(), [300.0, 300.0]);
    right_click(&mut h, p, Modifiers::NONE);
    // Chosen without a frame after it, which would drop a selection with no owner, as for a click.
    let ctx = h.ctx.clone();
    super::choose(h.state_mut(), &ctx, "path.deleteAnchor");
    assert_eq!(saved(h.state()).subpaths[0].knots.len(), 3, "the anchor is gone");
    let ds = &h.state().ui.direct_selection;
    assert!(ds.target.is_none_or(|t| t == PathRef::Saved(0)), "the selection is on no other path");
    assert!(ds.anchors.is_empty(), "no stale anchor index: {:?}", ds.anchors);
}

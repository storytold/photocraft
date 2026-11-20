//! #14895: the options bar shows how to reposition a Drop Shadow.
use super::gradient_overlay_hint_tests::{app_with_tool, assert_move_hint, click_label};
use crate::dialogs::canvas_input::{self, DialogPick};
use crate::state::Tool;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;

#[test]
fn drop_shadow_hint_replaces_the_hand_options() {
    const HINT: &str = "Click and drag to reposition the effect.";
    for tool in [Tool::Hand, Tool::Move] {
        let mut h = app_with_tool(tool);
        super::open(h.state_mut(), Some("dropShadow")).unwrap();
        h.run_steps(8);
        assert_move_hint(&h, HINT);
        assert!(matches!(canvas_input::armed(h.state()), Some(DialogPick::ShadowDrag)));

        click_label(&mut h, Role::Button, "Cancel");
        assert!(h.query_by_label(HINT).is_none());
        assert_eq!(h.state().ui.tool, tool, "the prior tool is preserved after Cancel");
        if tool == Tool::Hand {
            assert!(h.query_by_label("Scroll All Windows").is_some());
        } else {
            assert!(h.query_all_by_label_contains("Auto-Select").next().is_some());
        }
    }
}

#[test]
fn drop_shadow_keeps_the_brush_options() {
    let mut h = app_with_tool(Tool::Brush);
    super::open(h.state_mut(), Some("dropShadow")).unwrap();
    h.run_steps(8);
    assert!(h.query_by_label("Click and drag to reposition the effect.").is_none());
    assert!(h.query_all_by_label("Brush Tool").any(|node| node.rect().center().y < 80.0), "the Brush icon remains in the options bar");
}

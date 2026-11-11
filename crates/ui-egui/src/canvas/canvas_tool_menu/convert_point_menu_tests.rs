//! #12744: the Convert Point tool's right-click opens the same path menu as the Delete Anchor
//! Point tool's (#12771), with no Convert Point entry. The shared steps live in
//! `delete_anchor_menu_tests`.

use egui_kittest::kittest::Queryable;

use super::delete_anchor_menu_tests::{add_on_the_edge, delete_the_corner, empty_canvas, opening_changes_nothing, right_clicked, saved, setup};
use crate::state::Tool;

#[test]
fn right_click_on_an_anchor_offers_delete_anchor_point_on_top_and_it_deletes() {
    delete_the_corner(Tool::ConvertPoint);
}

#[test]
fn right_click_on_a_segment_offers_add_anchor_point_on_top() {
    add_on_the_edge(Tool::ConvertPoint);
}

#[test]
fn right_click_on_empty_canvas_opens_the_path_menu_without_a_top_entry() {
    empty_canvas(Tool::ConvertPoint);
}

#[test]
fn the_menu_has_no_convert_point_entry_and_converts_nothing() {
    let before = saved(&setup(Tool::ConvertPoint));
    for at in [[300.0, 300.0], [200.0, 300.0], [500.0, 100.0]] {
        let h = right_clicked(Tool::ConvertPoint, at);
        assert!(h.state().ui.canvas_tool_menu.is_some(), "the menu opened at {at:?}");
        assert!(h.query_by_label("Convert Point").is_none(), "no Convert Point entry at {at:?}");
        assert_eq!(saved(h.state()), before, "no point converted at {at:?}");
    }
}

#[test]
fn opening_the_menu_adds_no_history_step_and_converts_no_point() {
    opening_changes_nothing(Tool::ConvertPoint);
}

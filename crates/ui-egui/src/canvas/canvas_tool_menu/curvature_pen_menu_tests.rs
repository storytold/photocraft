//! #12753: with the Curvature Pen a right-click on the canvas opens the Pen's path menu, headed on an
//! anchor or a segment by the same point entry the anchor tools get.

use super::delete_anchor_menu_tests::{add_on_the_edge, delete_the_corner, right_clicked};
use super::{PEN_MENU, rows};
use crate::state::Tool;

#[test]
fn empty_canvas_opens_the_pens_path_menu() {
    let h = right_clicked(Tool::CurvaturePen, [600.0, 100.0]);
    let menu = h.state().ui.canvas_tool_menu.as_ref().expect("a menu");
    assert_eq!((menu.tool, rows(menu)), (Tool::CurvaturePen, PEN_MENU));
    assert!(menu.point.is_none(), "no point entry on empty canvas");
}

#[test]
fn an_anchor_heads_it_with_delete_anchor_point() {
    delete_the_corner(Tool::CurvaturePen);
}

#[test]
fn a_segment_heads_it_with_add_anchor_point() {
    add_on_the_edge(Tool::CurvaturePen);
}

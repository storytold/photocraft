//! #12837: with the Freeform Pen a right-click on the canvas opens the Pen's path menu, headed on an
//! anchor or a segment by the same point entry the anchor tools get.

use super::delete_anchor_menu_tests::{add_on_the_edge, delete_the_corner, right_clicked, setup};
use super::pen_free_transform_label_tests::transform_row;
use super::tests::{app, harness, pen_clicks, right_click};
use super::{PEN_MENU, rows};
use crate::state::Tool;

#[test]
fn empty_canvas_opens_the_pens_path_menu() {
    let h = right_clicked(Tool::FreeformPen, [600.0, 100.0]);
    let menu = h.state().ui.canvas_tool_menu.as_ref().expect("a menu");
    assert_eq!((menu.tool, rows(menu)), (Tool::FreeformPen, PEN_MENU));
    assert!(menu.point.is_none(), "no point entry on empty canvas");
}

#[test]
fn an_anchor_heads_it_with_delete_anchor_point() {
    delete_the_corner(Tool::FreeformPen);
}

#[test]
fn a_segment_heads_it_with_add_anchor_point() {
    add_on_the_edge(Tool::FreeformPen);
}

#[test]
fn shape_mode_keeps_opening_no_menu() {
    let mut app = setup(Tool::FreeformPen);
    app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Shape;
    let mut h = harness(app);
    let center = h.state().last_canvas_rect.center();
    right_click(&mut h, center, egui::Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "Shape mode was not measured: no menu");
}

#[test]
fn the_transform_row_is_named_after_what_it_acts_on() {
    assert_eq!(transform_row(app(), Tool::FreeformPen), ("Free Transform", false));
    let mut drawing = app();
    pen_clicks(&mut drawing, &[(2.0, 2.0), (20.0, 2.0), (20.0, 20.0)]);
    assert_eq!(transform_row(drawing, Tool::FreeformPen), ("Free Transform Points", true));
}

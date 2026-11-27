//! #13367: a right-click on the canvas with the Triangle tool opens the shape and path menu, as
//! measured in the reference application: in Shape mode over a pixel layer and over a triangle
//! shape layer (the same menu on empty canvas and inside the triangle), and in Path mode over a
//! pixel layer with a Work Path. Pixels mode opens nothing, as before. The checks are the
//! Rectangle's (#13341), which measured the same three menus.

use super::rectangle_menu_tests::{
    EMPTY, INSIDE, check_pixel_layer_menu, check_rows_run_as_elsewhere, check_shape_layer_menu, check_unmeasured_cases_open_nothing, check_work_path_menu,
    shape_layer,
};
use crate::state::Tool;

#[test]
fn on_a_pixel_layer_the_menu_lists_the_reference_rows() {
    check_pixel_layer_menu(Tool::Triangle);
}

#[test]
fn over_a_triangle_layer_the_menu_is_the_same_off_and_inside_the_triangle() {
    let app = shape_layer(Tool::Triangle);
    let st = app.session.active().unwrap();
    let tol = crate::path_components::press_tolerance(&app);
    let inside = [f64::from(INSIDE[0]), f64::from(INSIDE[1])];
    assert_eq!(crate::vector_ui::pick_shape_at(&st.doc, inside, tol), st.active_layer, "{INSIDE:?} is on the triangle");
    check_shape_layer_menu(Tool::Triangle, EMPTY);
    check_shape_layer_menu(Tool::Triangle, INSIDE);
}

#[test]
fn in_path_mode_over_a_work_path_the_menu_lists_the_reference_rows() {
    check_work_path_menu(Tool::Triangle, EMPTY);
    check_work_path_menu(Tool::Triangle, INSIDE);
}

#[test]
fn the_rows_run_the_pen_menu_and_menu_bar_commands() {
    check_rows_run_as_elsewhere(Tool::Triangle);
}

#[test]
fn pixels_mode_and_unmeasured_cases_open_nothing() {
    check_unmeasured_cases_open_nothing(Tool::Triangle);
}

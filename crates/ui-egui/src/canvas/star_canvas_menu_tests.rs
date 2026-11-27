//! #16874: the Star shares the shape tools' canvas menu in Shape mode.

use craft::draw::ui::VectorMode;
use egui::Modifiers;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{LayerContent, LiveShape};

use super::line_menu_tests::{greyed, menu, shown};
use super::rectangle_menu_tests::{EMPTY, INSIDE, check_pixel_layer_menu, entries_at, opens_nothing, pixel_layer, with_tool, work_path};
use super::tests::harness;
use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

fn star_layer() -> Box<PhotocraftApp> {
    let mut app = pixel_layer(Tool::Star);
    app.ui.tool_options.shape_fill = true;
    app.ui.tool_options.stroke_width = 2.0;
    testing::press(&mut app, [40.0, 40.0], Modifiers::NONE);
    testing::move_to(&mut app, [120.0, 120.0], Modifiers::NONE);
    testing::release(&mut app, [120.0, 120.0], Modifiers::NONE);
    let state = app.session.active().unwrap();
    let layer = state.doc.layer(state.active_layer.unwrap()).unwrap();
    assert!(
        matches!(&layer.content, LayerContent::Shape(shape) if matches!(shape.live.as_ref(), Some(LiveShape::Polygon { star_ratio, .. }) if *star_ratio < 1.0))
    );
    app
}

#[test]
fn star_canvas_menu_over_a_pixel_layer_uses_the_shared_shape_menu() {
    check_pixel_layer_menu(Tool::Star);
}

#[test]
fn star_canvas_menu_over_a_star_layer_lists_the_path_rows_and_uses_the_style_clipboard() {
    for point in [EMPTY, INSIDE] {
        let mut h = harness(star_layer());
        let entries = entries_at(&mut h, Tool::Star, point);
        assert_eq!(
            shown(&entries),
            menu(
                "Define Custom Shape… ― Blending Options… ― Make Selection… · New Guides From Shape · Fill Path… · Stroke Path… ― \
                 Rasterize Layer · Rasterize Layer Style · Rasterize Vector Mask ― Free Transform Path ― Unite Shapes · Subtract Front Shape · \
                 Unite Shapes at Overlap · Subtract Shapes at Overlap ― Copy Fill · Copy Complete Stroke ― Paste Fill · Paste Complete Stroke ― \
                 Isolate Layers"
            )
        );
        assert_eq!(
            greyed(&entries),
            [
                "Fill Path…",
                "Stroke Path…",
                "Rasterize Layer Style",
                "Rasterize Vector Mask",
                "Unite Shapes",
                "Subtract Front Shape",
                "Unite Shapes at Overlap",
                "Subtract Shapes at Overlap",
                "Paste Fill",
                "Paste Complete Stroke",
            ]
        );

        h.get_by_label("Copy Fill").click();
        h.run_steps(2);
        assert!(h.state().ui.canvas_tool_menu.is_none());
        assert!(h.state().session.path_fill_clipboard.is_some());
        assert!(!greyed(&entries_at(&mut h, Tool::Star, point)).iter().any(|row| row == "Paste Fill"));
    }
}

#[test]
fn star_canvas_menu_stays_closed_in_path_and_pixels_modes() {
    for (app, mode, case) in [
        (pixel_layer(Tool::Star), VectorMode::Path, "Path mode without a path"),
        (work_path(Tool::Star), VectorMode::Path, "Path mode with a work path"),
        (star_layer(), VectorMode::Path, "Path mode with a star layer"),
        (pixel_layer(Tool::Star), VectorMode::Pixels, "Pixels mode with a pixel layer"),
        (star_layer(), VectorMode::Pixels, "Pixels mode with a star layer"),
    ] {
        let mut h = harness(with_tool(app, Tool::Star, mode));
        opens_nothing(&mut h, INSIDE, case);
    }
}

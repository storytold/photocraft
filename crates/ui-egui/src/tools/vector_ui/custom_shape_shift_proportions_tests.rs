//! #13464: ⇧ with the Custom Shape tool keeps the shape's own proportions, as large as fits in the
//! dragged box and anchored at the press corner (⌥: centred on the press point), as Photoshop
//! does, instead of fitting the shape into a square on the drag's longer side. The drag preview
//! shows the same outline as the layer, and the other Shape tools keep their own ⇧ rule.

use egui::Modifiers;
use photocraft_doc::LayerContent;
use serde_json::json;

use super::{aspect, chosen_shape, drag_geometry, shape_geometry, shape_preview};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// A new 800 × 600 document with the Custom Shape tool and its default shape (Heart) picked.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::CustomShape;
    app
}

/// A drag from `from` to `to`, with `at_press` held at the press and `later` from the first move.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], at_press: Modifiers, later: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, at_press);
    tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, later);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, later);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, later);
}

/// The active shape layer's outline.
fn outline(app: &PhotocraftApp) -> photocraft_doc::vector::Path {
    let st = app.session.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        LayerContent::Shape(s) => s.path.clone(),
        _ => panic!("a shape layer"),
    }
}

/// The active shape layer's outline box as [x, y, w, h].
fn bounds(app: &PhotocraftApp) -> [f64; 4] {
    let (x0, y0, x1, y1) = outline(app).control_bounds().expect("an outline");
    [x0, y0, x1 - x0, y1 - y0]
}

/// The default shape's width over height.
fn heart_aspect(app: &PhotocraftApp) -> f64 {
    let a = aspect(&chosen_shape(app).expect("the Heart").path).expect("an aspect");
    assert!((a - 1.0).abs() > 0.01, "the test needs a shape that is not square, got {a}");
    a
}

/// Whether box `got` is `want` within the half pixel the drag box rounds to.
fn near(got: [f64; 4], want: [f64; 4]) -> bool {
    got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 0.75)
}

/// A wide ⇧-drag keeps the drag's height and the shape's aspect, from the press corner.
#[test]
fn a_wide_shift_drag_keeps_the_height_and_the_shape_aspect() {
    let mut app = app();
    let a = heart_aspect(&app);
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::SHIFT, Modifiers::SHIFT);
    let got = bounds(&app);
    assert!(near(got, [400.0, 300.0, 100.0 * a, 100.0]), "got {got:?}, aspect {a}");
}

/// A tall ⇧-drag keeps the drag's width, from the press corner.
#[test]
fn a_tall_shift_drag_keeps_the_width() {
    let mut app = app();
    let a = heart_aspect(&app);
    drag(&mut app, [400.0, 300.0], [450.0, 500.0], Modifiers::SHIFT, Modifiers::SHIFT);
    let got = bounds(&app);
    assert!(near(got, [400.0, 300.0, 50.0, 50.0 / a]), "got {got:?}, aspect {a}");
}

/// ⇧⌥ pressed after the start: proportional and centred on the press point.
#[test]
fn shift_alt_centres_the_proportional_shape_on_the_press() {
    let mut app = app();
    let a = heart_aspect(&app);
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE, Modifiers::SHIFT | Modifiers::ALT);
    let got = bounds(&app);
    let (w, h) = (200.0 * a, 200.0);
    assert!(near(got, [400.0 - w / 2.0, 300.0 - h / 2.0, w, h]), "got {got:?}, aspect {a}");
}

/// An up-left ⇧-drag anchors the shape at the press corner too, which is its lower right.
#[test]
fn an_up_left_shift_drag_ends_at_the_press_corner() {
    let mut app = app();
    let a = heart_aspect(&app);
    drag(&mut app, [600.0, 400.0], [400.0, 300.0], Modifiers::SHIFT, Modifiers::SHIFT);
    let got = bounds(&app);
    let w = 100.0 * a;
    assert!(near(got, [600.0 - w, 300.0, w, 100.0]), "got {got:?}, aspect {a}");
}

/// The drag preview's outline is the outline of the layer the drag makes.
#[test]
fn the_preview_shows_the_shape_the_drag_makes() {
    for (to, mods) in [([600.0, 400.0], Modifiers::SHIFT), ([450.0, 500.0], Modifiers::SHIFT), ([600.0, 400.0], Modifiers::SHIFT | Modifiers::ALT)] {
        let mut app = app();
        let preview = shape_preview(&app, Tool::CustomShape, [400.0, 300.0], to, mods).expect("a preview");
        drag(&mut app, [400.0, 300.0], to, Modifiers::NONE, mods);
        assert_eq!(preview.path, outline(&app), "preview of → {to:?} with {mods:?}");
    }
}

/// The other Shape tools keep their own ⇧ rule: the chosen custom shape plays no part in it.
#[test]
fn other_shape_tools_keep_their_shift_rule() {
    let app = app();
    for tool in [Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Star] {
        for mods in [Modifiers::SHIFT, Modifiers::SHIFT | Modifiers::ALT] {
            let own = shape_geometry(&app.ui.tool_options, tool, [400.0, 300.0], [600.0, 400.0], mods);
            assert_eq!(drag_geometry(&app, tool, [400.0, 300.0], [600.0, 400.0], mods), own, "{tool:?} with {mods:?}");
        }
    }
}

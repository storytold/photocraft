//! #13471: in Path mode the Custom Shape tool's options bar shows Make: Selection… · Mask · Shape, greyed with no path;
//! with a path just drawn they are enabled and Shape turns the path into a shape layer. Shape mode has no Make row.

use egui::{Modifiers, PointerButton};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::super::test_input::harness;
use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;
use craft::draw::ui::VectorMode;

type H = Harness<'static, Box<PhotocraftApp>>;

const MAKE: [&str; 3] = ["Selection…", "Mask", "Shape"];

fn custom_shape(mode: VectorMode) -> H {
    let mut h = harness(Tool::CustomShape);
    h.state_mut().ui.tool_options.shape_mode = mode;
    h.run_steps(3);
    h
}

/// The button labelled `label`: the bar's Shape picker shares the Make button's name, so only buttons count.
fn button<'a>(h: &'a H, label: &'a str) -> Option<egui_kittest::Node<'a>> {
    h.query_all_by_label(label).find(|n| n.accesskit_node().role() == egui::accesskit::Role::Button)
}

/// Whether each Make button is there and enabled; `None` when it isn't shown.
fn make_buttons(h: &H) -> Vec<Option<bool>> {
    MAKE.iter().map(|label| button(h, label).map(|n| !n.accesskit_node().is_disabled())).collect()
}

/// A drag on the canvas from `a` to `b` (document points).
fn drag(h: &mut H, a: [f64; 2], b: [f64; 2]) {
    let p = testing::to_screen(h.state_mut(), a);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for i in 1..=6 {
        let t = f64::from(i) / 6.0;
        let q = testing::to_screen(h.state_mut(), [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        h.event(egui::Event::PointerMoved(q));
        h.run_steps(1);
    }
    let q = testing::to_screen(h.state_mut(), b);
    h.event(egui::Event::PointerButton { pos: q, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

fn shape_layers(h: &H) -> usize {
    let doc = &h.state().session.active().unwrap().doc;
    doc.walk().into_iter().filter(|(_, _, l)| matches!(l.content, photocraft_doc::LayerContent::Shape(_))).count()
}

#[test]
fn path_mode_shows_the_make_buttons_greyed_without_a_path() {
    let h = custom_shape(VectorMode::Path);
    assert!(h.query_by_label("Make:").is_some(), "the Make: label");
    assert_eq!(make_buttons(&h), vec![Some(false); 3], "all three greyed with no path");
}

#[test]
fn a_drawn_path_enables_them_and_shape_makes_a_shape_layer() {
    let mut h = custom_shape(VectorMode::Path);
    drag(&mut h, [40.0, 20.0], [140.0, 80.0]);
    assert!(h.state().session.active().unwrap().doc.work_path.is_some(), "the Custom Shape tool drew a path");
    assert_eq!(make_buttons(&h), vec![Some(true); 3], "enabled with a path");
    let before = shape_layers(&h);
    button(&h, "Shape").expect("the Make: Shape button").click();
    h.run_steps(3);
    assert_eq!(shape_layers(&h), before + 1, "Make: Shape made a shape layer from the path");
}

#[test]
fn shape_mode_has_no_make_row() {
    let h = custom_shape(VectorMode::Shape);
    assert_eq!(make_buttons(&h), vec![None; 3]);
}

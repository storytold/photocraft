//! #13877: the Frame shape buttons use the measured tooltips, and Polygon shows a `#` caption.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::i18n::Lang;
use crate::state::Tool;

const SHAPES: [(&str, &str); 5] = [
    ("rectangle", "Use the mouse to create a new rectangular Frame"),
    ("ellipse", "Use the mouse to create a new elliptical Frame"),
    ("triangle", "Use the mouse to create a new triangular Frame"),
    ("polygon", "Use the mouse to create a new polygonal Frame"),
    ("custom", "Use the mouse to create a new custom Frame"),
];

fn bar() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 80, "height": 60})).unwrap();
    app.ui.tool = Tool::Frame;
    let mut h =
        Harness::builder().with_size(egui::vec2(900.0, 180.0)).with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, Default::default())).build_ui_state(
            |ui, app| {
                let _language = crate::i18n::language_scope(Lang::EN);
                ui.horizontal(|ui| super::options_bar(app, ui, Tool::Frame));
            },
            app,
        );
    h.run_steps(4);
    h
}

fn drawn(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    fn collect(shape: &egui::Shape, text: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(shape) => text.push(shape.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, text)),
            _ => {}
        }
    }
    let mut text = Vec::new();
    for shape in &h.output().shapes {
        collect(&shape.shape, &mut text);
    }
    text
}

#[test]
fn shape_buttons_show_the_exact_tooltips_in_order_and_still_select_their_shapes() {
    let mut h = bar();
    let lefts = SHAPES.map(|(_, tip)| h.get_by_role_and_label(Role::Button, tip).rect().left());
    assert!(lefts.windows(2).all(|w| w[0] < w[1]), "shape buttons remain in reference order");
    for (shape, tip) in SHAPES {
        h.get_by_role_and_label(Role::Button, tip).hover();
        h.run_steps(60);
        assert!(drawn(&h).iter().any(|text| text == tip), "hover must show {tip:?}: {:?}", drawn(&h));
        h.get_by_role_and_label(Role::Button, tip).click();
        h.run_steps(4);
        assert_eq!(h.state().ui.frame_tool.shape, shape);
        assert_eq!(h.state().ui.frame_tool.ellipse, shape == "ellipse");
    }
}

#[test]
fn polygon_shows_the_hash_caption_and_keeps_the_default_and_accessible_name() {
    let mut h = bar();
    h.state_mut().ui.frame_tool.shape = "polygon".into();
    h.run_steps(4);
    let text = drawn(&h);
    assert!(text.iter().any(|text| text == "#"), "polygon caption: {text:?}");
    assert!(!text.iter().any(|text| text == "Sides:"), "old caption: {text:?}");
    assert_eq!(h.state().ui.frame_tool.sides, 5);
    assert!(text.iter().any(|text| text == "5"), "the field displays the default: {text:?}");
    assert!(h.query_by_role_and_label(Role::SpinButton, "Sides:").is_some(), "the field keeps its descriptive accessible name");
}

#[test]
fn polygon_sides_field_keeps_the_three_to_one_hundred_range() {
    let mut h = bar();
    h.state_mut().ui.frame_tool.shape = "polygon".into();
    h.run_steps(4);
    for (typed, expected) in [("1", 3), ("3", 3), ("100", 100), ("101", 100)] {
        h.get_by_role_and_label(Role::SpinButton, "Sides:").click();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text(typed.into()));
        h.run_steps(2);
        h.key_press(egui::Key::Enter);
        h.run_steps(4);
        assert_eq!(h.state().ui.frame_tool.sides, expected, "typed {typed}");
    }
}

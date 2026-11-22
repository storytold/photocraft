//! The app's mapping of the gradient editor's edits onto commands, on a layer target.
//! The editor's own rows, fields, selection and tear-off are the framework's and tested there.

use super::*;
use egui::accesskit::Role;
use egui::{Pos2, Rect, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_doc::Fill;

/// A harness around `gradient_ui::editor::show`. With `layer` a Gradient Fill layer is selected
/// (three colour stops, three opacity stops), else the same gradient is the tool's current one.
pub(super) fn editor(layer: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
    let mut params = json!({"stops": [[0, "#ff0000"], [0.5, "#00ff00"], [1, "#0000ff"]], "transparency": [[0, 25], [0.4, 60], [1, 100]]});
    if layer {
        params["from"] = json!([20, 60]);
        params["to"] = json!([180, 60]);
        app.run(cmds::CREATE, params).unwrap();
    } else {
        app.run("gradient.presets.select", params).unwrap();
    }
    let ctx = app.ctx().clone();
    let mut h = Harness::builder()
        .with_size(vec2(1000.0, 900.0))
        .with_context(ctx)
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::gradient_ui::editor::show(app, ui), app);
    h.run_steps(4);
    h
}

/// The gradient of the active layer.
fn layer_fill(h: &Harness<Box<PhotocraftApp>>) -> Fill {
    let st = h.state().session.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        photocraft_doc::LayerContent::Fill(f @ Fill::Gradient { .. }) => f.clone(),
        other => panic!("{}", other.kind_name()),
    }
}

fn history(h: &Harness<Box<PhotocraftApp>>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

fn click(h: &mut Harness<Box<PhotocraftApp>>, pos: Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
}

/// The rect of the first marker whose accessible name or value contains `text`: the bar's markers
/// are named with their location, and their order is the bar's geometry order, not location order.
fn marker(h: &Harness<Box<PhotocraftApp>>, text: &str) -> Rect {
    h.get_all_by_role(Role::GraphicsObject)
        .find(|n| {
            let n = n.accesskit_node();
            n.label().is_some_and(|l| l.contains(text)) || n.value().is_some_and(|v| v.contains(text))
        })
        .unwrap_or_else(|| panic!("no marker named {text:?}"))
        .rect()
}

/// Selects the marker at `text` (a location such as "50%") with a click.
fn select(h: &mut Harness<Box<PhotocraftApp>>, text: &str) {
    let at = marker(h, text).center();
    click(h, at);
}

/// Types `value` into the number field named `name` of the selected marker, and commits it with
/// Enter when `commit`.
fn type_number(h: &mut Harness<Box<PhotocraftApp>>, name: &str, value: &str, commit: bool) {
    h.get_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no number field {name:?}"))
        .click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(value.into()));
    h.run_steps(2);
    if commit {
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
    }
}

/// Adds a colour stop: selects a colour marker, then Insert adds one halfway in its row.
pub(super) fn add_stop(h: &mut Harness<Box<PhotocraftApp>>) {
    let at = h
        .get_all_by_role(Role::GraphicsObject)
        .find(|n| n.accesskit_node().label().is_some_and(|l| l.to_lowercase().contains("color")))
        .or_else(|| h.get_all_by_role(Role::GraphicsObject).next())
        .expect("the bar has markers")
        .rect()
        .center();
    click(h, at);
    h.key_press(egui::Key::Insert);
    h.run_steps(3);
}

/// Presses on the marker at `text`, drags it `dx` points along the bar in two steps, runs
/// `during` with the button still down, then releases.
fn drag_marker(h: &mut Harness<Box<PhotocraftApp>>, text: &str, dx: f32, during: impl FnOnce(&mut Harness<Box<PhotocraftApp>>)) {
    let from = marker(h, text).center();
    let to = from + vec2(dx, 0.0);
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerMoved(from + (to - from) * 0.5));
    h.run_steps(1);
    h.event(egui::Event::PointerMoved(to));
    h.run_steps(1);
    during(h);
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
}

/// On a layer target a typed opacity and a stop drag are one `gradient.fill.set` history step
/// each; the drag adds no step before the release, and undo restores each state.
#[test]
fn a_typed_opacity_and_a_stop_drag_are_one_history_step_each() {
    let mut h = editor(true);
    let start = layer_fill(&h);
    let steps = history(&h);

    select(&mut h, "40%");
    type_number(&mut h, "Opacity", "35", true);
    let typed = layer_fill(&h);
    assert!(
        matches!(&typed, Fill::Gradient { opacity_stops, .. } if opacity_stops.iter().any(|s| (s.1 - 0.35).abs() < 1e-5)),
        "the typed opacity reached the fill"
    );
    assert_eq!(history(&h), steps + 1, "one step for the typed value");

    drag_marker(&mut h, "40%", 80.0, |h| {
        assert_eq!(layer_fill(h), typed, "nothing is committed while the button is down");
        assert_eq!(history(h), steps + 1, "no step before the release");
    });
    let dragged = layer_fill(&h);
    assert_ne!(dragged, typed, "the drag moved the stop");
    assert_eq!(history(&h), steps + 2, "one step for the drag");

    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(layer_fill(&h), typed, "undo restores the state before the drag");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(layer_fill(&h), start, "undo restores the state before the typed value");
}

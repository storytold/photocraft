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
    let app = editor_app(layer);
    let ctx = app.ctx().clone();
    let mut h = Harness::builder()
        .with_size(vec2(1000.0, 900.0))
        .with_context(ctx)
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::gradient_ui::editor::show(app, ui), app);
    h.run_steps(4);
    h
}

/// The app of [`editor`]: a document, and the gradient on a Gradient Fill layer (`layer`) or as the tool's.
fn editor_app(layer: bool) -> Box<PhotocraftApp> {
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
    app
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

/// The Gradient Editor panel, open in the workspace, beside the Color panel (Hue Cube) and the Swatches panel, over the
/// gradient of [`editor`].
fn editor_with_panels(layer: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = editor_app(layer);
    crate::panel_docking::show_panel(&mut app, EDITOR_PANEL, Some(true)).unwrap();
    let ctx = app.ctx().clone();
    let mut h = Harness::builder().with_size(vec2(1000.0, 900.0)).with_context(ctx).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            let room = |x: f32, y: f32, w: f32, h: f32| egui::UiBuilder::new().max_rect(Rect::from_min_size(egui::pos2(x, y), vec2(w, h)));
            ui.scope_builder(room(0.0, 0.0, 560.0, 900.0), |ui| crate::gradient_ui::editor::show(app, ui));
            ui.scope_builder(room(580.0, 0.0, 300.0, 200.0), |ui| crate::panels::color::panel(app, ui));
            ui.scope_builder(room(580.0, 220.0, 400.0, 600.0), |ui| crate::swatches_ui::panel(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::studio());
    h.run_steps(4);
    h
}

fn hex3(c: [f32; 3]) -> String {
    let [r, g, b] = c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The colour stops of the edited gradient (the layer's, or the tool's), in location order, as 8-bit hex.
fn stop_hexes(h: &Harness<Box<PhotocraftApp>>, layer: bool) -> Vec<String> {
    let app = h.state();
    let shown = if layer {
        stops::of_fill(&layer_fill(h))
    } else {
        stops::of_preset(&app.session.presets.gradient, app.session.tools.foreground, app.session.tools.background)
    };
    shown.color.iter().map(|s| hex3([s.color.r, s.color.g, s.color.b])).collect()
}

fn foreground_hex(h: &Harness<Box<PhotocraftApp>>) -> String {
    let fg = h.state().session.tools.foreground;
    hex3([fg[0], fg[1], fg[2]])
}

/// The colour the Color panel shows and edits as the foreground.
fn panel_hex(h: &Harness<Box<PhotocraftApp>>) -> String {
    let c = crate::panels::color::edited_foreground(h.state());
    hex3([c[0], c[1], c[2]])
}

/// A swatch's name and its colour as `swatches.use` sets it: the first of the first group whose colour is none of the
/// gradient's, nor the foreground.
fn first_swatch(h: &Harness<Box<PhotocraftApp>>) -> (String, String) {
    let app = h.state();
    let conv = photocraft_engine::presets::swatches::SwatchConv::new(&app.session);
    let taken = ["#ff0000".to_owned(), "#00ff00".to_owned(), "#0000ff".to_owned(), foreground_hex(h)];
    app.session.presets.swatches[0]
        .entries
        .iter()
        .map(|e| (e.name.clone(), hex3(conv.rgb(&e.color))))
        .find(|(_, hex)| !taken.contains(hex))
        .expect("a swatch of another colour")
}

/// Clicks the Swatches panel's cell of swatch `name` (named "name, value").
fn click_swatch(h: &mut Harness<Box<PhotocraftApp>>, name: &str) {
    let prefix = format!("{name}, ");
    let at = h.get_all_by_label_contains(&prefix).next().unwrap_or_else(|| panic!("no swatch cell {name:?}")).rect().center();
    click(h, at);
}

/// A point of the Color panel's Hue Cube field, at fractions `x`, `y` of its width and height.
fn field_at(h: &Harness<Box<PhotocraftApp>>, x: f32, y: f32) -> Pos2 {
    h.get_by_label("Saturation and brightness").rect().lerp_inside(vec2(x, y))
}

/// Drags in the Color panel's field from `from` to `to` (fractions of the field) in four steps, and returns the middle
/// colour stop's colour after the press and after each step, before the release.
fn drag_field(h: &mut Harness<Box<PhotocraftApp>>, layer: bool, from: [f32; 2], to: [f32; 2]) -> Vec<String> {
    let at = |h: &Harness<Box<PhotocraftApp>>, t: f32| field_at(h, from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t);
    let start = at(h, 0.0);
    h.event(egui::Event::PointerMoved(start));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    let mut seen = vec![stop_hexes(h, layer)[1].clone()];
    for i in 1..=4 {
        let p = at(h, i as f32 / 4.0);
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(1);
        seen.push(stop_hexes(h, layer)[1].clone());
    }
    let end = at(h, 1.0);
    h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
    seen
}

/// #10783: with the Gradient Editor open on a colour stop, a click in the Color panel's field and a click on a swatch
/// cell recolour that stop, as the editor's own Color Picker does, and leave the foreground alone, as in Photoshop; the
/// Color panel shows the stop's colour. On a layer each pick is one undo step and Undo restores the gradient; the tool
/// gradient is not in the document, so its picks leave the document's history alone.
#[test]
fn the_color_panel_and_a_swatch_recolour_the_selected_editor_stop() {
    for layer in [true, false] {
        let mut h = editor_with_panels(layer);
        select(&mut h, "Color stop 2 of 3");
        let sink = if layer { EditorSink::Layer(h.state().session.active().unwrap().active_layer.unwrap()) } else { EditorSink::Tool };
        assert_eq!(h.state().gradient.editor_stop.map(|s| (s.sink, s.index)), Some((sink, 1)), "layer {layer}: the middle colour stop");
        let foreground = foreground_hex(&h);
        let steps = history(&h);
        let before = stop_hexes(&h, layer);
        assert_eq!(panel_hex(&h), "#00ff00", "layer {layer}: the Color panel shows the stop");

        let at = field_at(&h, 0.3, 0.6);
        click(&mut h, at);
        let picked = stop_hexes(&h, layer);
        assert_eq!((picked[0].as_str(), picked[2].as_str()), ("#ff0000", "#0000ff"), "layer {layer}: only the selected stop changes");
        assert_ne!(picked[1], "#00ff00", "layer {layer}: the field click recolours the stop");
        assert_eq!(foreground_hex(&h), foreground, "layer {layer}: the foreground stays");
        assert_eq!(panel_hex(&h), picked[1], "layer {layer}: the Color panel shows the new stop colour");

        let (name, swatch) = first_swatch(&h);
        click_swatch(&mut h, &name);
        assert_eq!(stop_hexes(&h, layer), ["#ff0000", swatch.as_str(), "#0000ff"], "layer {layer}: the swatch {name}");
        assert_eq!(foreground_hex(&h), foreground, "layer {layer}: a swatch leaves the foreground too");

        if layer {
            assert_eq!(history(&h), steps + 2, "one step per pick");
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(stop_hexes(&h, layer), picked, "Undo takes back the swatch");
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(stop_hexes(&h, layer), before, "Undo restores the gradient");
        } else {
            assert_eq!(history(&h), steps, "the tool gradient is not in the document");
        }
    }
}

/// A drag in the Color panel recolours the selected stop as it goes and is one history step; a second drag on that stop
/// is a step of its own, and one Undo takes back only it.
#[test]
fn a_color_panel_drag_on_an_editor_stop_is_one_step() {
    for layer in [true, false] {
        let mut h = editor_with_panels(layer);
        select(&mut h, "Color stop 2 of 3");
        let foreground = foreground_hex(&h);
        let steps = history(&h);
        let mut seen = drag_field(&mut h, layer, [0.2, 0.2], [0.8, 0.7]);
        seen.dedup();
        assert!(seen.len() >= 3, "layer {layer}: the stop follows the drag: {seen:?}");
        assert_eq!(foreground_hex(&h), foreground, "layer {layer}: the foreground stays");
        let first = stop_hexes(&h, layer);
        assert_eq!(history(&h), if layer { steps + 1 } else { steps }, "layer {layer}: one drag, one step");

        drag_field(&mut h, layer, [0.7, 0.3], [0.3, 0.9]);
        assert_ne!(stop_hexes(&h, layer), first, "layer {layer}: the second drag recolours the stop");
        if layer {
            assert_eq!(history(&h), steps + 2, "two drags, two steps");
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(stop_hexes(&h, layer), first, "Undo takes back the second drag only");
        } else {
            assert_eq!(history(&h), steps);
        }
    }
}

/// Without a selected colour stop (an opacity stop selected, or the editor closed) the Color panel and a swatch set
/// the foreground as before and the gradient doesn't change.
#[test]
fn without_a_selected_editor_color_stop_the_panels_set_the_foreground() {
    for layer in [true, false] {
        let mut h = editor_with_panels(layer);
        let before = stop_hexes(&h, layer);
        select(&mut h, "Opacity stop 2 of 3");
        assert_eq!(h.state().gradient.editor_stop, None, "layer {layer}: an opacity stop takes no colour");
        let foreground = foreground_hex(&h);
        let at = field_at(&h, 0.3, 0.6);
        click(&mut h, at);
        assert_ne!(foreground_hex(&h), foreground, "layer {layer}: the Color panel sets the foreground");
        assert_eq!(panel_hex(&h), foreground_hex(&h));
        let (name, swatch) = first_swatch(&h);
        click_swatch(&mut h, &name);
        assert_eq!(foreground_hex(&h), swatch, "layer {layer}: the swatch sets the foreground");
        assert_eq!(stop_hexes(&h, layer), before, "layer {layer}: the gradient doesn't change");

        // A colour stop selected, then the panel closed: its recorded stop no longer takes the colour.
        select(&mut h, "Color stop 2 of 3");
        assert!(h.state().gradient.editor_stop.is_some());
        crate::panel_docking::show_panel(h.state_mut(), EDITOR_PANEL, Some(false)).unwrap();
        h.run_steps(2);
        assert_eq!(panel_hex(&h), foreground_hex(&h), "layer {layer}: the Color panel shows the foreground again");
        let at = field_at(&h, 0.8, 0.2);
        click(&mut h, at);
        assert_eq!(stop_hexes(&h, layer), before, "layer {layer}: the closed editor's stop keeps its colour");
        assert!(!panel_color_picked(h.state_mut(), [0.0; 3], None));
    }
}

/// A selected colour midpoint takes no colour either: the Color panel sets the foreground and the gradient keeps its own.
#[test]
fn a_selected_midpoint_leaves_the_panels_on_the_foreground() {
    let mut h = editor_with_panels(true);
    let before = layer_fill(&h);
    select(&mut h, "Midpoint 1");
    assert!(h.get_all_by_role(Role::SpinButton).any(|n| n.accesskit_node().label().as_deref() == Some("Midpoint")), "a midpoint is selected");
    assert_eq!(h.state().gradient.editor_stop, None);
    let foreground = foreground_hex(&h);
    let at = field_at(&h, 0.3, 0.6);
    click(&mut h, at);
    assert_ne!(foreground_hex(&h), foreground, "the Color panel sets the foreground");
    assert_eq!(layer_fill(&h), before, "the gradient doesn't change");
}

/// The picks of one gesture on two different stops are two history steps, so one Undo never reverts another stop's
/// colour.
#[test]
fn one_gesture_on_two_editor_stops_is_two_steps() {
    let mut h = editor_with_panels(true);
    select(&mut h, "Color stop 2 of 3");
    let steps = history(&h);
    assert!(panel_color_picked(h.state_mut(), [0.2; 3], Some("color-panel:7")));
    let stop = h.state().gradient.editor_stop.unwrap();
    h.state_mut().gradient.editor_stop = Some(EditorStop { index: 2, ..stop });
    assert!(panel_color_picked(h.state_mut(), [0.4; 3], Some("color-panel:7")));
    assert_eq!(stop_hexes(&h, true), ["#ff0000".to_owned(), hex3([0.2; 3]), hex3([0.4; 3])]);
    assert_eq!(history(&h), steps + 2);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(stop_hexes(&h, true), ["#ff0000".to_owned(), hex3([0.2; 3]), "#0000ff".to_owned()], "Undo takes back one stop's colour");
}

/// The stop the panel recorded is dropped when another layer is selected or another document is active before the
/// panel draws again: a pick then sets the foreground and never recolours the same-numbered stop of another gradient.
#[test]
fn a_recorded_editor_stop_is_dropped_for_another_layer_or_document() {
    let mut h = editor_with_panels(true);
    select(&mut h, "Color stop 2 of 3");
    let first = h.state().session.active().unwrap().active_layer.unwrap();
    let before = layer_fill(&h);
    // Another gradient fill layer is selected in the same document: the recorded stop still exists on the first layer,
    // but the panel now shows the second.
    let params = json!({"stops": [[0, "#ff0000"], [0.5, "#00ff00"], [1, "#0000ff"]], "from": [20, 60], "to": [180, 60]});
    h.state_mut().run(cmds::CREATE, params.clone()).unwrap();
    let second = layer_fill(&h);
    assert!(!panel_color_picked(h.state_mut(), [1.0, 1.0, 0.0], None), "another layer is selected");
    assert_eq!(layer_fill(&h), second);
    h.state_mut().run("layer.select", json!({"layer": first.0})).unwrap();
    assert_eq!(layer_fill(&h), before, "the first gradient keeps its stop");
    assert!(editor_stop_color(h.state()).is_some(), "the first gradient is selected again");
    // Another document whose selected gradient has the recorded layer's id (ids repeat across documents): the stop was
    // recorded in the first document, so it is not this one's.
    let first_doc = h.state().session.active().unwrap().doc.id;
    h.state_mut().run("file.new", json!({"width": 200, "height": 120})).unwrap();
    h.state_mut().run(cmds::CREATE, params).unwrap();
    let other = h.state().session.active().unwrap().active_layer.unwrap();
    h.state_mut().gradient.editor_stop = Some(EditorStop { doc: Some(first_doc), sink: EditorSink::Layer(other), index: 1 });
    let other_before = layer_fill(&h);
    assert!(!panel_color_picked(h.state_mut(), [1.0, 1.0, 0.0], None), "another document is active");
    assert_eq!(layer_fill(&h), other_before);
}

//! Exercise the actual editor window, including its layer and tool-preset targets.

use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

fn editor(layer: bool) -> (Harness<'static, PhotocraftApp>, Sink) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
    let mut params = json!({"stops": [[0, "#ff0000"], [0.5, "#00ff00"], [1, "#0000ff"]], "transparency": [[0, 25], [0.5, 60], [1, 100]]});
    let sink = if layer {
        params["from"] = json!([20, 60]);
        params["to"] = json!([180, 60]);
        app.run(cmds::CREATE, params).unwrap();
        Sink::Layer(app.session.active().unwrap().active_layer.unwrap())
    } else {
        app.run("gradient.presets.select", params).unwrap();
        Sink::Preset
    };
    let mut h = Harness::builder().with_size(vec2(1000.0, 900.0)).build_ui_state(|ui, app: &mut PhotocraftApp| editor_window(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run_steps(2);
    h.state_mut().ui.panels.gradient_editor = true;
    h.run_steps(4);
    (h, sink)
}

fn click(h: &mut Harness<PhotocraftApp>, pos: Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
}

fn select(h: &mut Harness<PhotocraftApp>, location: f32, opacity: bool) {
    let rect = h.get_by_label("Gradient stops").rect();
    click(h, pos2(rect.left() + 8.0 + location * (rect.width() - 16.0), rect.top() + if opacity { 6.0 } else { 50.0 }));
}

fn type_number(h: &mut Harness<PhotocraftApp>, index: usize, value: &str, commit: bool) {
    h.get_all_by_role(egui::accesskit::Role::SpinButton).nth(index).unwrap().click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(value.into()));
    h.run_steps(2);
    if commit {
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
    }
}

#[test]
fn top_opacity_and_bottom_color_select_their_own_fields_in_both_editor_modes() {
    for layer in [true, false] {
        let (mut h, sink) = editor(layer);
        let before = sink.fill(h.state()).unwrap();
        assert!(h.query_by_label("Color stops").is_none(), "editor has one combined strip");
        assert!(h.query_by_label("Opacity stops").is_none());
        assert!(h.get_all_by_label("Color").any(|n| n.accesskit_node().role() == egui::accesskit::Role::Button));
        select(&mut h, 0.5, true);
        let key = strip_key(h.state(), sink).with("selection");
        assert_eq!(h.ctx.data(|d| d.get_temp::<Marker>(key)), Some(Marker::Opacity(1)));
        assert!(h.query_by_label("Color").is_none(), "opacity selection hides color controls");
        select(&mut h, 0.5, false);
        assert_eq!(h.ctx.data(|d| d.get_temp::<Marker>(key)), Some(Marker::Color(1)));
        assert!(h.get_all_by_label("Color").any(|n| n.accesskit_node().role() == egui::accesskit::Role::Button));
        assert_eq!(sink.fill(h.state()).unwrap(), before, "selecting stops never edits the gradient");
    }
}

#[test]
fn selected_editor_opacity_commits_once_and_delete_preserves_minimum() {
    let (mut h, sink) = editor(true);
    select(&mut h, 0.5, true);
    let before = sink.fill(h.state()).unwrap();
    let history = h.state().session.active().unwrap().history.past_len();
    type_number(&mut h, 0, "35", false);
    assert_eq!(sink.fill(h.state()).unwrap(), before, "typing previews until committed");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let after = sink.fill(h.state()).unwrap();
    assert!(matches!(&after, Fill::Gradient { opacity_stops, .. } if (opacity_stops[1].1 - 0.35).abs() < 1e-5));
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
    h.get_by_label("Delete").click();
    h.run_steps(3);
    let Fill::Gradient { stops, opacity_stops, .. } = sink.fill(h.state()).unwrap() else { panic!() };
    assert_eq!(stops.len(), 3);
    assert_eq!(opacity_stops.len(), 2);
    select(&mut h, 0.0, true);
    assert!(h.get_by_label("Delete").accesskit_node().is_disabled());
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(sink.fill(h.state()).unwrap(), after);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(sink.fill(h.state()).unwrap(), before);
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(sink.fill(h.state()).unwrap(), after);
}

#[test]
fn editor_location_follows_reordered_color_and_opens_existing_picker() {
    for layer in [true, false] {
        let (mut h, sink) = editor(layer);
        select(&mut h, 0.0, false);
        type_number(&mut h, 0, "70", true);
        let key = strip_key(h.state(), sink).with("selection");
        assert_eq!(h.ctx.data(|d| d.get_temp::<Marker>(key)), Some(Marker::Color(1)));
        assert!(matches!(sink.fill(h.state()).unwrap(), Fill::Gradient { stops, .. } if (stops[1].0 - 0.7).abs() < 1e-5));
        h.get_all_by_label("Color").find(|n| n.accesskit_node().role() == egui::accesskit::Role::Button).unwrap().click();
        h.run_steps(3);
        assert!(!h.state().ui.dialogs.is_empty());
    }
}

#[test]
fn editor_opacity_location_changes_only_opacity_in_both_modes() {
    for layer in [true, false] {
        let (mut h, sink) = editor(layer);
        let Fill::Gradient { stops: before, .. } = sink.fill(h.state()).unwrap() else { panic!() };
        select(&mut h, 0.0, true);
        type_number(&mut h, 1, "70", true);
        let Fill::Gradient { stops, opacity_stops, .. } = sink.fill(h.state()).unwrap() else { panic!() };
        assert_eq!(stops, before, "opacity location must not move a color stop");
        assert!((opacity_stops[1].0 - 0.7).abs() < 1e-5);
        let key = strip_key(h.state(), sink).with("selection");
        assert_eq!(h.ctx.data(|d| d.get_temp::<Marker>(key)), Some(Marker::Opacity(1)));
    }
}

#[test]
fn preset_editor_opacity_fields_do_not_edit_document_history() {
    let (mut h, sink) = editor(false);
    let before = h.state().session.active().unwrap().doc.clone();
    let history = h.state().session.active().unwrap().history.past_len();
    select(&mut h, 0.5, true);
    type_number(&mut h, 0, "35", true);
    assert!(matches!(sink.fill(h.state()).unwrap(), Fill::Gradient { opacity_stops, .. } if (opacity_stops[1].1 - 0.35).abs() < 1e-5));
    h.get_by_label("Delete").click();
    h.run_steps(3);
    assert!(matches!(sink.fill(h.state()).unwrap(), Fill::Gradient { stops, opacity_stops, .. } if stops.len() == 3 && opacity_stops.len() == 2));
    assert_eq!(h.state().session.active().unwrap().doc, before);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
}

/// The sorted colour stops of the sink's gradient, as 8-bit hex.
fn stop_hexes(app: &PhotocraftApp, sink: Sink) -> Vec<String> {
    let Some(Fill::Gradient { mut stops, .. }) = sink.fill(app) else { panic!("a gradient") };
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    stops.iter().map(|(_, c)| hex3(c.to_rgb())).collect()
}

fn hex3(c: [f32; 3]) -> String {
    let [r, g, b] = c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// A swatch's name and its colour as `swatches.use` sets it: the first that differs from the
/// stops', the foreground's and the Color panel picks' test colours.
fn first_swatch(app: &PhotocraftApp) -> (String, [f32; 3]) {
    let conv = photocraft_engine::presets::swatches::SwatchConv::new(&app.session);
    let taken = ["#ff0000", "#00ff00", "#0000ff", "#ffff00", "#000000"];
    let sw = app.session.presets.swatches[0].items.iter().find(|s| !taken.contains(&hex3(conv.rgb(&s.color)).as_str())).unwrap();
    (sw.name.clone(), conv.rgb(&sw.color))
}

/// #10783: with the Gradient Editor open on a colour stop, a Color panel pick and a swatch click
/// recolour that stop (the editor shows it at once), as the editor's own Color Picker does, and
/// leave the foreground alone, as in Photoshop; the Color panel shows the stop's colour. A layer's
/// stop is one undo step per pick and Undo restores the gradient.
#[test]
fn color_and_swatches_panels_recolour_the_selected_editor_stop() {
    for layer in [true, false] {
        let (mut h, sink) = editor(layer);
        select(&mut h, 0.5, false);
        assert_eq!(h.state().gradient.editor_stop, Some((sink, 1)));
        let foreground = h.state().session.tools.foreground;
        let history = h.state().session.active().unwrap().history.past_len();
        let before = sink.fill(h.state()).unwrap();
        assert_eq!(hex3(crate::color_panel_ui::active(h.state())), "#00ff00", "the Color panel shows the stop");
        let ctx = h.ctx.clone();
        crate::color_panel_ui::set_color(h.state_mut(), &ctx, false, [1.0, 1.0, 0.0]);
        h.run_steps(2);
        assert_eq!(stop_hexes(h.state(), sink), ["#ff0000", "#ffff00", "#0000ff"]);
        assert_eq!(h.state().session.tools.foreground, foreground, "the foreground stays");
        assert_eq!(hex3(crate::color_panel_ui::active(h.state())), "#ffff00");
        let (name, rgb) = first_swatch(h.state());
        crate::swatches_ui::use_swatch(h.state_mut(), &name, false);
        h.run_steps(2);
        assert_eq!(stop_hexes(h.state(), sink), ["#ff0000".to_string(), hex3(rgb), "#0000ff".to_string()]);
        assert_eq!(h.state().session.tools.foreground, foreground, "a swatch leaves the foreground too");
        if layer {
            assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 2, "one step per pick");
            h.state_mut().run("edit.undo", json!({})).unwrap();
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(sink.fill(h.state()).unwrap(), before, "Undo restores the gradient");
        } else {
            assert_eq!(h.state().session.active().unwrap().history.past_len(), history, "the tool gradient is not in the document");
        }
    }
}

/// The picks of one Color panel drag are one history step.
#[test]
fn a_color_panel_drag_on_an_editor_stop_is_one_step() {
    let (mut h, sink) = editor(true);
    select(&mut h, 1.0, false);
    let history = h.state().session.active().unwrap().history.past_len();
    for g in [0.2, 0.4, 0.6] {
        assert!(panel_color_picked(h.state_mut(), [g, g, g], Some("press-1".into())));
    }
    assert_eq!(stop_hexes(h.state(), sink)[2], hex3([0.6; 3]));
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
}

/// Without a selected colour stop (an opacity stop selected, or the editor closed) the Color and
/// Swatches panels set the foreground as before and the gradient doesn't change.
#[test]
fn without_a_selected_editor_color_stop_the_panels_set_the_foreground() {
    for layer in [true, false] {
        let (mut h, sink) = editor(layer);
        let before = sink.fill(h.state()).unwrap();
        select(&mut h, 0.5, true);
        assert_eq!(h.state().gradient.editor_stop, None, "an opacity stop takes no colour");
        let ctx = h.ctx.clone();
        crate::color_panel_ui::set_color(h.state_mut(), &ctx, false, [1.0, 1.0, 0.0]);
        h.run_steps(2);
        assert_eq!(hex3(crate::color_panel_ui::active(h.state())), "#ffff00");
        let fg = h.state().session.tools.foreground;
        assert_eq!(hex3([fg[0], fg[1], fg[2]]), "#ffff00", "the Color panel sets the foreground");
        assert_eq!(sink.fill(h.state()).unwrap(), before, "the gradient doesn't change");
        // Closed editor: a colour stop was selected, but nothing in the gradient changes.
        select(&mut h, 0.5, false);
        h.state_mut().ui.panels.gradient_editor = false;
        h.run_steps(2);
        assert_eq!(h.state().gradient.editor_stop, None);
        let (name, rgb) = first_swatch(h.state());
        crate::swatches_ui::use_swatch(h.state_mut(), &name, false);
        let fg = h.state().session.tools.foreground;
        assert_eq!(hex3([fg[0], fg[1], fg[2]]), hex3(rgb), "the swatch sets the foreground");
        assert!(!panel_color_picked(h.state_mut(), [0.0, 0.0, 0.0], None));
        assert_eq!(sink.fill(h.state()).unwrap(), before, "the gradient doesn't change");
    }
}

//! #12549: the Remove tool's Overlay options. Its options bar carries a gear button that opens an
//! "Overlay options" popup: a Color well that opens the Color Picker and an Opacity field held to
//! 1-100 % with the range alert. What the popup sets reaches the trail of the live stroke and of the
//! waiting strokes, is kept in the tool options, and Reset Tool gives back the values a fresh start
//! has - which is still the theme's own overlay colour and the trail tone's own opacity.

use egui::Color32;
use egui::accesskit::Role;
use egui::epaint::Shape;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::app::framework::BuildApp as _;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{Tool, ToolOptions};
use crate::stroke_trail::OverlayTint;

const GEAR: &str = "Set additional options";
/// A colour the popup can hold: a quarter, a half and three quarters on.
const CHOSEN: [f32; 3] = [0.25, 0.5, 0.75];

/// A settled options bar with `tool`, plus the dialog host that draws a field's range alert.
fn bar(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    crate::theme::install_fonts(app.ctx());
    crate::theme::apply(app.ctx(), &crate::theme::AppTheme::pro());
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut Box<PhotocraftApp>| {
            crate::panels::options_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// The bar's control with `role` and `label`, found by role so tooltip text never matches.
fn control<'h>(h: &'h Harness<'static, Box<PhotocraftApp>>, role: Role, label: &str) -> egui_kittest::Node<'h> {
    h.query_all_by_role(role)
        .find(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.trim_end_matches(':') == label))
        .unwrap_or_else(|| panic!("missing {role:?} {label:?}"))
}

/// Opens Remove's Overlay options popup.
fn open_gear(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    control(h, Role::Button, GEAR).click();
    h.run_steps(3);
}

/// Types `text` into the number field named `name` and commits it with Enter.
fn type_into(h: &mut Harness<'static, Box<PhotocraftApp>>, name: &str, text: &str) {
    control(h, Role::SpinButton, name).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

/// The messages of the open framework alerts.
fn alerts(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<String> {
    h.state().framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_string)).collect()
}

/// Whether two channel triples are the same colour, within one level per channel.
fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1.0 / 255.0)
}

#[test]
fn the_gear_button_opens_a_popup_with_a_colour_well_and_an_opacity_field() {
    let mut h = bar(Tool::Remove);
    assert!(
        h.query_all_by_role(Role::SpinButton).all(|n| n.accesskit_node().label().as_deref().is_none_or(|l| !l.starts_with("Opacity"))),
        "no Opacity field before the gear"
    );
    open_gear(&mut h);
    control(&h, Role::ColorWell, "Color");
    control(&h, Role::SpinButton, "Opacity");
}

#[test]
fn no_other_retouch_bar_grows_the_overlay_gear() {
    for tool in [Tool::Blur, Tool::SpotHealing, Tool::CloneStamp] {
        let h = bar(tool);
        assert!(h.query_all_by_role(Role::Button).all(|n| n.accesskit_node().label().as_deref() != Some(GEAR)), "{tool:?} has no Overlay options gear");
    }
}

#[test]
fn an_opacity_outside_its_range_keeps_the_closest_value_and_alerts_once() {
    for (typed, kept, wants_alert) in [("150", 100.0, true), ("0", 1.0, true), ("30", 30.0, false)] {
        let mut h = bar(Tool::Remove);
        open_gear(&mut h);
        type_into(&mut h, "Opacity", typed);
        let what = format!("Opacity {typed}");
        assert_eq!(h.state().ui.tool_options.remove_overlay_opacity, kept, "{what}: the closest value is kept");
        let shown = alerts(&h);
        assert_eq!(!shown.is_empty(), wants_alert, "{what}: {shown:?}");
        if wants_alert {
            assert_eq!(shown.len(), 1, "{what}: one alert");
            assert!(shown[0].contains("Closest value inserted"), "{what}: {shown:?}");
            h.get_by_label("OK").click();
            h.run_steps(4);
            assert!(alerts(&h).is_empty(), "{what}: OK closes the alert");
            assert_eq!(h.state().ui.tool_options.remove_overlay_opacity, kept, "{what}: the value stays after OK");
        }
    }
}

#[test]
fn the_colour_well_opens_the_color_picker_and_ok_keeps_what_it_answers() {
    let mut h = bar(Tool::Remove);
    open_gear(&mut h);
    control(&h, Role::ColorWell, "Color").click();
    h.run_steps(4);
    let id = crate::color_picker_ui::top(h.state()).expect("the well opens the Color Picker");
    assert!(h.query_all_by_label("Color Picker").next().is_some(), "the picker is titled \"Color Picker\"");
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, "color", json!("#336699")).unwrap();
    crate::dialogs::testing::confirm(h.state_mut(), id).unwrap();
    h.run_steps(3);
    assert!(crate::color_picker_ui::top(h.state()).is_none(), "OK closes the picker");
    let app = h.state();
    let rgb = app.ui.tool_options.remove_overlay_color.expect("the picked colour is kept in the tool options");
    assert!(close(rgb, [0x33 as f32 / 255.0, 0x66 as f32 / 255.0, 0x99 as f32 / 255.0]), "the picked colour: {rgb:?}");
    let (color, share) = OverlayTint::of(app, Tool::Remove).props();
    assert_eq!(color.map(|c| c.0), Some(egui::Color32::from_rgb(0x33, 0x66, 0x99)), "the trail takes the picked colour");
    assert_eq!(share, None, "an unset opacity leaves the tone's own share in place");
    assert!(OverlayTint::of(app, Tool::Blur).props() == (None, None), "another tool's overlay is untouched by the choice");
}

#[test]
fn the_tint_holds_the_opacity_as_a_share_and_a_fresh_start_holds_neither() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = Tool::Remove;
    assert!(OverlayTint::of(&app, Tool::Remove).is_plain(), "a fresh Remove overlay shows the tone's own colour and share");
    app.ui.tool_options.remove_overlay_color = Some([0x20 as f32 / 255.0, 0x80 as f32 / 255.0, 0x60 as f32 / 255.0]);
    app.ui.tool_options.remove_overlay_opacity = 30.0;
    let (color, share) = OverlayTint::of(&app, Tool::Remove).props();
    assert_eq!(color.map(|c| c.0), Some(egui::Color32::from_rgb(0x20, 0x80, 0x60)), "the colour reaches the trail");
    assert_eq!(share, Some(0.3), "30 % is a share of 0.3");
    assert_eq!(OverlayTint::share(150.0), 1.0, "no share shows more than the colour itself");
    assert_eq!(OverlayTint::share(0.0), 0.0, "no share at all shows no footprint");
    app.ui.tool_options.remove_overlay_color = None;
    assert!(!OverlayTint::of(&app, Tool::Remove).is_plain(), "an opacity alone still tints the overlay");
}

#[test]
fn the_two_values_are_kept_with_the_tool_options_and_old_settings_stay_plain() {
    let o = ToolOptions { remove_overlay_color: Some([0.2, 0.6, 0.9]), remove_overlay_opacity: 30.0, ..ToolOptions::default() };
    let back: ToolOptions = serde_json::from_value(serde_json::to_value(&o).unwrap()).unwrap();
    assert_eq!(back, o, "the pair travels with the tool options");
    let seen = serde_json::to_value(&o).unwrap();
    assert!(seen.get("remove_overlay_color").is_some() && seen.get("remove_overlay_opacity").is_some(), "the control channel reads both by name");
    let older: ToolOptions = serde_json::from_value(json!({ "tolerance": 40.0 })).unwrap();
    assert_eq!(older.remove_overlay_color, None, "options saved before this hold no overlay colour");
    assert_eq!(older.remove_overlay_opacity, ToolOptions::default().remove_overlay_opacity, "…nor an opacity of their own");
    use craft::ui::canvas_controls::brush::TrailTone;
    assert_eq!(ToolOptions::default().remove_overlay_opacity, TrailTone::Danger.share() * 100.0, "the default opacity is the share the tool's own tone paints");
    assert!(OverlayTint::default().is_plain(), "and a fresh start tints nothing");
}

#[test]
fn the_chosen_pair_colours_the_live_and_the_waiting_overlay_at_once() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Remove;
    app.ui.tool_options.remove_after_each_stroke = false;
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(900.0, 600.0)).build_app(
        |ui, app: &mut PhotocraftApp| {
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    h.run_steps(4);
    let m = egui::Modifiers::NONE;
    let state = h.state_mut();
    tool_event(state, ToolEvent::Down { x: 30.0, y: 40.0, pressure: 1.0 }, m);
    tool_event(state, ToolEvent::Move { x: 60.0, y: 40.0, pressure: 1.0 }, m);
    h.run_steps(2);
    assert!(h.state().trail.is_some(), "the stroke being dragged has an overlay to show");
    let plain = overlay_inks(&h);
    h.state_mut().ui.tool_options.remove_overlay_color = Some(CHOSEN);
    h.state_mut().ui.tool_options.remove_overlay_opacity = 20.0;
    h.run_steps(3);
    let want = chosen_ink(0.2);
    assert_eq!(changed_inks(&h, &plain), vec![want], "the live overlay takes the chosen colour at the chosen opacity");
    // The finished stroke waits for Enter, and its overlay carries the same pair.
    let state = h.state_mut();
    tool_event(state, ToolEvent::Up { x: 60.0, y: 40.0 }, m);
    h.run_steps(3);
    assert!(h.state().remove_trail.is_some(), "the finished stroke waits, tinted");
    assert!(overlay_inks(&h).contains(&want), "the waiting overlay shows the same pair: {:?}", overlay_inks(&h));
    // Another opacity while the stroke waits reaches that overlay without a new stroke.
    h.state_mut().ui.tool_options.remove_overlay_opacity = 80.0;
    h.run_steps(3);
    assert!(overlay_inks(&h).contains(&chosen_ink(0.8)), "the waiting overlay follows a new opacity at once: {:?}", overlay_inks(&h));
    let state = h.state_mut();
    tool_event(state, ToolEvent::Down { x: 20.0, y: 60.0, pressure: 1.0 }, m);
    h.run_steps(2);
    assert!(changed_inks(&h, &plain).contains(&chosen_ink(0.8)), "a new stroke's live overlay carries it too: {:?}", overlay_inks(&h));
    let state = h.state_mut();
    tool_event(state, ToolEvent::Up { x: 20.0, y: 60.0 }, m);
    h.run_steps(2);
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
}

/// The ink of the tint the tool options hold: their channels as bytes, at `share`.
fn chosen_ink(share: f32) -> Color32 {
    let byte = |v: f32| (v * 255.0).round() as u8;
    Color32::from_rgb(byte(CHOSEN[0]), byte(CHOSEN[1]), byte(CHOSEN[2])).gamma_multiply(share)
}

/// Walks a frame's shapes, flattening the groups.
fn flatten<'a>(shape: &'a Shape, out: &mut Vec<&'a Shape>) {
    if let Shape::Vec(v) = shape {
        v.iter().for_each(|s| flatten(s, out));
    } else {
        out.push(shape);
    }
}

/// The ink of every footprint quad the canvas painted this frame: a four-corner mesh whose four
/// corners carry one colour.
fn overlay_inks(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<Color32> {
    let mut inks = Vec::new();
    for clipped in &h.output().shapes {
        let mut flat = Vec::new();
        flatten(&clipped.shape, &mut flat);
        for shape in flat {
            if let Shape::Mesh(mesh) = shape
                && mesh.vertices.len() == 4
                && mesh.vertices.iter().all(|v| v.color == mesh.vertices[0].color)
            {
                inks.push(mesh.vertices[0].color);
            }
        }
    }
    inks
}

/// The inks `h` shows now that it did not show with `before`.
fn changed_inks(h: &Harness<'static, Box<PhotocraftApp>>, before: &[Color32]) -> Vec<Color32> {
    overlay_inks(h).into_iter().filter(|ink| !before.contains(ink)).collect()
}

#[test]
fn reset_tool_gives_the_overlay_its_starting_values_back() {
    let mut h = bar(Tool::Remove);
    open_gear(&mut h);
    type_into(&mut h, "Opacity", "30");
    let app = h.state();
    assert_eq!(app.ui.tool_options.remove_overlay_opacity, 30.0, "the popup's value is held");
    let start = ToolOptions::default();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "tool.reset", json!({})).unwrap();
    h.run_steps(3);
    let app = h.state();
    assert_eq!(app.ui.tool_options.remove_overlay_opacity, start.remove_overlay_opacity, "Reset Tool restores the opacity a fresh start has");
    assert_eq!(app.ui.tool_options.remove_overlay_color, start.remove_overlay_color, "Reset Tool restores the colour a fresh start has");
    assert!(OverlayTint::of(app, Tool::Remove).is_plain(), "and the overlay shows the tool's own again");
}

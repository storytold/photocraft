//! #13465: the Custom Shape tool's gear: Unconstrained, Defined Proportions, Defined Size, Fixed
//! Size (with W and H) and From Center, as the reference application measures them on an
//! 800 × 600 document. Defined Proportions and Defined Size take the picked preset's own path
//! bounds; the Rectangle tool's gear stays the Rectangle's own.

use egui::Modifiers;
use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{Tool, ToolOptions};

/// An 800 × 600 document with `tool` active, the built-in preset `preset` picked and the tool
/// options set by `set`.
fn app_with(tool: Tool, preset: &str, set: impl FnOnce(&mut ToolOptions)) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app.ui.presets_ui.custom_shape = preset.to_string();
    set(&mut app.ui.tool_options);
    app
}

/// The same with the Custom Shape tool active.
fn shape_app(preset: &str, set: impl FnOnce(&mut ToolOptions)) -> Box<PhotocraftApp> {
    app_with(Tool::CustomShape, preset, set)
}

/// A canvas drag from `from` to `to`, pressed without keys and moved and released with `held`.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], held: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, held);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, held);
}

/// The box `[x0, y0, x1, y1]` of the shape layer the app has now: its live box, else its path's
/// control bounds (a placed custom shape has no live box).
fn placed_box(app: &PhotocraftApp) -> [f64; 4] {
    let st = app.session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let photocraft_doc::LayerContent::Shape(sh) = &layer.content else { panic!("the drag leaves a shape layer") };
    if let Some(photocraft_doc::vector::LiveShape::Rect { rect, .. }) = &sh.live {
        return [rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]];
    }
    let (x0, y0, x1, y1) = sh.path.control_bounds().unwrap();
    [x0, y0, x1, y1]
}

/// The box `[x0, y0, x1, y1]` of the shape layer a `preset` drag from `from` to `to` places.
fn drawn(preset: &str, set: impl FnOnce(&mut ToolOptions), from: [f64; 2], to: [f64; 2], held: Modifiers) -> [f64; 4] {
    let mut app = shape_app(preset, set);
    let layers = app.session.active().unwrap().doc.layers.len();
    drag(&mut app, from, to, held);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1, "the drag places one shape layer");
    placed_box(&app)
}

/// The native size `[w, h]` of a built-in preset, from its path bounds: what the two
/// preset-driven options are measured against.
fn native(name: &str) -> (f64, f64) {
    let shape = photocraft_engine::presets::shapes::builtin().into_iter().flat_map(|g| g.items).find(|s| s.name == name).unwrap();
    let (x0, y0, x1, y1) = shape.path.control_bounds().unwrap();
    (x1 - x0, y1 - y0)
}

/// The gear on Defined Proportions.
fn proportions(o: &mut ToolOptions) {
    o.custom_constraint = "definedProportions".into();
}

/// The gear on Defined Size.
fn defined_size(o: &mut ToolOptions) {
    o.custom_constraint = "definedSize".into();
}

/// The gear on Fixed Size `w` × `h`.
fn fixed(w: f32, h: f32) -> impl FnOnce(&mut ToolOptions) {
    move |o| (o.custom_constraint, o.custom_fixed_width, o.custom_fixed_height) = ("fixedSize".into(), w, h)
}

#[test]
fn defined_proportions_locks_the_drag_to_the_preset_aspect() {
    // G1 and G1b: the largest box of the preset's own aspect inside the drag, from the press point.
    // Two presets, so the rule is the preset's aspect rather than one measured pair of numbers.
    assert_eq!(native("Arrow Right"), (100.0, 70.0));
    assert_eq!(native("Arrow Down"), (70.0, 100.0));
    // Height-limited drag: the 100 px height, the preset's width to match (100 × 100 / 70, whole px).
    assert_eq!(drawn("Arrow Right", proportions, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [400.0, 300.0, 543.0, 400.0]);
    // Width-limited drag (G1b): the 50 px width, the preset's height to match.
    assert_eq!(drawn("Arrow Right", proportions, [400.0, 300.0], [450.0, 500.0], Modifiers::NONE), [400.0, 300.0, 450.0, 335.0]);
    // The other preset in the same drag: it is tall, so the width follows its own aspect.
    assert_eq!(drawn("Arrow Down", proportions, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [400.0, 300.0, 470.0, 400.0]);
    // The other three corners of the document keep the press point as the box's corner.
    assert_eq!(drawn("Arrow Right", proportions, [400.0, 300.0], [200.0, 200.0], Modifiers::NONE), [257.0, 200.0, 400.0, 300.0]);
}

#[test]
fn defined_size_places_the_preset_size_at_the_release_point() {
    // G2: the native size, its top left at the release, whatever the drag covered.
    assert_eq!(drawn("Arrow Right", defined_size, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [600.0, 400.0, 700.0, 470.0]);
    assert_eq!(drawn("Arrow Down", defined_size, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [600.0, 400.0, 670.0, 500.0]);
    // A drag the other way still puts the size at the release point, not at the press.
    assert_eq!(drawn("Arrow Right", defined_size, [600.0, 400.0], [400.0, 300.0], Modifiers::NONE), [400.0, 300.0, 500.0, 370.0]);
    // A short drag places the same size: the drag sets only where the shape lands.
    assert_eq!(drawn("Arrow Right", defined_size, [400.0, 300.0], [410.0, 310.0], Modifiers::NONE), [410.0, 310.0, 510.0, 380.0]);
}

#[test]
fn fixed_size_uses_the_typed_size_at_the_release_point() {
    // G3: 120 × 60 at the release point, and a second size for the same rule.
    assert_eq!(drawn("Heart", fixed(120.0, 60.0), [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [600.0, 400.0, 720.0, 460.0]);
    assert_eq!(drawn("Heart", fixed(40.0, 90.0), [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [600.0, 400.0, 640.0, 490.0]);
    // The typed size ignores the preset: a third preset, the same box.
    assert_eq!(drawn("Arrow Down", fixed(120.0, 60.0), [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [600.0, 400.0, 720.0, 460.0]);
}

#[test]
fn a_fixed_size_field_left_empty_draws_the_dragged_box() {
    // The gear's own rule for an empty field, as the framework's Rectangle gear documents it: a 0
    // is no size, so nothing constrains the drag.
    assert_eq!(drawn("Heart", fixed(120.0, 0.0), [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [400.0, 300.0, 600.0, 400.0]);
    // The same for Defined Proportions: the preset's aspect, here the Heart's own 100 × 92 path.
    assert_eq!(native("Heart"), (100.0, 92.0));
    assert_eq!(drawn("Heart", proportions, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [400.0, 300.0, 509.0, 400.0]);
}

#[test]
fn from_center_mirrors_the_dragged_box_around_the_press_point() {
    // G4: Unconstrained with From Center on.
    let from_center = |o: &mut ToolOptions| o.custom_from_center = true;
    assert_eq!(drawn("Heart", from_center, [400.0, 300.0], [500.0, 350.0], Modifiers::NONE), [300.0, 250.0, 500.0, 350.0]);
    // With a constraint the constrained box is what grows about the press point.
    let proportions_from_center = |o: &mut ToolOptions| (o.custom_constraint, o.custom_from_center) = ("definedProportions".into(), true);
    assert_eq!(drawn("Arrow Right", proportions_from_center, [400.0, 300.0], [500.0, 400.0], Modifiers::NONE), [300.0, 230.0, 500.0, 370.0]);
}

#[test]
fn unconstrained_is_the_default_and_draws_the_dragged_box() {
    // The reference's default: Unconstrained, W and H empty, From Center off. The drag's own box.
    let d = ToolOptions::default();
    assert_eq!(d.custom_constraint, "unconstrained");
    assert_eq!((d.custom_fixed_width, d.custom_fixed_height), (0.0, 0.0), "W and H are empty to start");
    assert!(!d.custom_from_center, "From Center is off to start");
    assert_eq!(drawn("Heart", |_| {}, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [400.0, 300.0, 600.0, 400.0]);
}

#[test]
fn held_shift_and_alt_stay_as_they_are_with_the_default_gear() {
    // Not measured for this issue: with the gear left at its default the held keys do exactly what
    // they did before the gear existed (⇧ squares the box on the shorter side, ⌥ draws from the
    // centre). The square preset keeps the placed path on the same box as the drag. Guard test,
    // passes before and after.
    assert_eq!(native("Plus"), (100.0, 100.0));
    assert_eq!(drawn("Plus", |_| {}, [400.0, 300.0], [600.0, 400.0], Modifiers::SHIFT), [400.0, 300.0, 500.0, 400.0]);
    assert_eq!(drawn("Plus", |_| {}, [400.0, 300.0], [500.0, 350.0], Modifiers::ALT), [300.0, 250.0, 500.0, 350.0]);
    assert_eq!(drawn("Plus", |_| {}, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [400.0, 300.0, 600.0, 400.0]);
}

#[test]
fn the_gear_stays_set_for_the_next_drag() {
    // The option is a setting, not a one-shot: a second drag is constrained the same way.
    let mut app = shape_app("Arrow Right", defined_size);
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE);
    assert_eq!(placed_box(&app), [600.0, 400.0, 700.0, 470.0]);
    drag(&mut app, [100.0, 100.0], [200.0, 200.0], Modifiers::NONE);
    assert_eq!(placed_box(&app), [200.0, 200.0, 300.0, 270.0]);
}

#[test]
fn the_two_gears_are_the_two_tools_own() {
    // The Rectangle's gear settings stay the Rectangle's: none of the Custom Shape fields reach it.
    let mut app = shape_app("Arrow Right", |o| (o.custom_constraint, o.custom_from_center) = ("definedSize".into(), true));
    app.ui.tool = Tool::Rectangle;
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE);
    assert_eq!(placed_box(&app), [400.0, 300.0, 600.0, 400.0], "a Rectangle drag ignores the Custom Shape gear");
    // The other way: the Rectangle's Square does not square a Custom Shape drag, and the Ellipse
    // tool, which has no gear of its own yet (#13233), is not constrained by these fields either.
    let mut app = shape_app("Arrow Right", |_| {});
    app.ui.tool_options.rect_constraint = "square".into();
    app.ui.tool_options.rect_from_center = true;
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE);
    assert_eq!(placed_box(&app), [400.0, 300.0, 600.0, 400.0], "the Rectangle gear does not reach the Custom Shape tool");
    let mut app = app_with(Tool::EllipseShape, "Arrow Right", defined_size);
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE);
    assert_eq!(placed_box(&app), [400.0, 300.0, 600.0, 400.0], "the Ellipse tool draws the dragged box");
}

#[test]
fn path_mode_draws_the_same_constrained_outline() {
    // The gear shapes the outline that joins the Work Path, as it shapes the placed shape.
    let mut app = shape_app("Arrow Right", defined_size);
    let layers = app.session.active().unwrap().doc.layers.len();
    app.ui.tool_options.shape_mode = craft::draw::ui::VectorMode::Path;
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE);
    let work = app.session.active().unwrap().doc.work_path.clone().expect("the work path");
    let pts: Vec<_> = work.subpaths.iter().flat_map(|s| s.knots.iter()).map(|k| k.anchor).collect();
    let min = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
    let max = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let (xs, ys): (Vec<f64>, Vec<f64>) = (pts.iter().map(|p| p.x).collect(), pts.iter().map(|p| p.y).collect());
    assert_eq!([min(&xs), min(&ys), max(&xs), max(&ys)], [600.0, 400.0, 700.0, 470.0]);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "Path mode makes no layer");
}

#[test]
fn reset_tool_returns_the_gear_to_unconstrained() {
    // Done when: Reset Tool gives Unconstrained with From Center off, and the next drag is free.
    let mut app = shape_app("Arrow Right", fixed(120.0, 60.0));
    app.ui.tool_options.custom_from_center = true;
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, "tool.reset", json!({})).unwrap();
    let o = &app.ui.tool_options;
    assert_eq!(o.custom_constraint, "unconstrained");
    assert_eq!((o.custom_fixed_width, o.custom_fixed_height), (0.0, 0.0));
    assert!(!o.custom_from_center);
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE);
    assert_eq!(placed_box(&app), [400.0, 300.0, 600.0, 400.0]);
}

#[test]
fn the_settings_are_saved_with_the_tool_options() {
    // Saved with the tool options, and an options file written before this gear existed still loads.
    let mut o = ToolOptions::default();
    (o.custom_constraint, o.custom_fixed_width, o.custom_fixed_height, o.custom_from_center) = ("definedProportions".into(), 3.0, 2.0, true);
    let back: ToolOptions = serde_json::from_value(serde_json::to_value(&o).unwrap()).unwrap();
    assert_eq!(back, o);
    let mut old = serde_json::to_value(ToolOptions::default()).unwrap();
    let map = old.as_object_mut().unwrap();
    for k in ["custom_constraint", "custom_fixed_width", "custom_fixed_height", "custom_from_center"] {
        map.remove(k);
    }
    let back: ToolOptions = serde_json::from_value(old).unwrap();
    assert_eq!(back.custom_constraint, "unconstrained");
    assert_eq!((back.custom_fixed_width, back.custom_fixed_height), (0.0, 0.0));
    assert!(!back.custom_from_center);
}

/// The gear button's accessible name, the same words the Rectangle's gear uses.
const GEAR: &str = "Set additional shape and path options";

/// The options bar with the Custom Shape tool active.
fn bar() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.ui.tool = Tool::CustomShape;
    let mut h =
        Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 400.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    h.run_steps(4);
    h
}

/// The gear's number field labelled `label`.
fn field<'a>(h: &'a Harness<'static, Box<PhotocraftApp>>, label: &str) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("missing the {label:?} field"))
}

/// The gear's check box labelled `label`.
fn check<'a>(h: &'a Harness<'static, Box<PhotocraftApp>>, label: &str) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::CheckBox).find(|n| n.accesskit_node().label().as_deref() == Some(label)).unwrap_or_else(|| panic!("missing the {label:?} check"))
}

/// The bar's radios, top to bottom, by their labels.
fn radios_top_to_bottom(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<String> {
    let mut rows: Vec<(f32, String)> =
        h.query_all_by_role(Role::RadioButton).map(|n| (n.rect().top(), n.accesskit_node().label().unwrap_or_default())).collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|(_, l)| l).collect()
}

#[test]
fn the_gear_popup_lists_the_reference_controls_at_their_defaults() {
    // Done when: the popup, top to bottom, Unconstrained selected and From Center off. There are no
    // Symmetric, sides or star rows here.
    let mut h = bar();
    h.get_by_role_and_label(Role::Button, GEAR).click();
    h.run_steps(3);
    assert_eq!(radios_top_to_bottom(&h), ["Unconstrained", "Defined Proportions", "Defined Size", "Fixed Size"]);
    let picked: Vec<String> = h
        .query_all_by_role(Role::RadioButton)
        .filter(|n| n.accesskit_node().toggled() == Some(Toggled::True))
        .map(|n| n.accesskit_node().label().unwrap_or_default())
        .collect();
    assert_eq!(picked, ["Unconstrained"], "Unconstrained is the selected radio");
    assert_eq!(check(&h, "From Center").accesskit_node().toggled(), Some(Toggled::False), "From Center is off to start");
    assert_eq!(h.state().ui.tool_options.custom_constraint, "unconstrained");
    assert!(!h.state().ui.tool_options.custom_from_center, "From Center is off to start");
}

#[test]
fn the_gear_greys_w_and_h_until_fixed_size_is_chosen() {
    let mut h = bar();
    h.get_by_role_and_label(Role::Button, GEAR).click();
    h.run_steps(3);
    assert!(field(&h, "W:").accesskit_node().is_disabled() && field(&h, "H:").accesskit_node().is_disabled(), "greyed while Unconstrained is chosen");
    h.get_by_role_and_label(Role::RadioButton, "Fixed Size").click();
    h.run_steps(3);
    assert!(!field(&h, "W:").accesskit_node().is_disabled() && !field(&h, "H:").accesskit_node().is_disabled(), "both fields are live under Fixed Size");
    assert_eq!(h.state().ui.tool_options.custom_constraint, "fixedSize", "the pick is the stored option");
    // The preset-driven radios have no fields of their own: they grey them again.
    h.get_by_role_and_label(Role::RadioButton, "Defined Proportions").click();
    h.run_steps(3);
    assert!(field(&h, "W:").accesskit_node().is_disabled() && field(&h, "H:").accesskit_node().is_disabled(), "Defined Proportions keeps them greyed");
    assert_eq!(h.state().ui.tool_options.custom_constraint, "definedProportions");
    assert!(!check(&h, "From Center").accesskit_node().is_disabled(), "From Center is always live");
}

#[test]
fn the_from_center_check_stays_ticked_after_the_popover_closes() {
    let mut h = bar();
    h.get_by_role_and_label(Role::Button, GEAR).click();
    h.run_steps(3);
    check(&h, "From Center").click();
    h.run_steps(2);
    assert!(h.state().ui.tool_options.custom_from_center, "the tick is the tool option");
    // The gear's second press closes the popover...
    h.get_by_role_and_label(Role::Button, GEAR).click();
    h.run_steps(3);
    assert!(h.query_all_by_role(Role::CheckBox).all(|n| n.accesskit_node().label().as_deref() != Some("From Center")), "the popover closed");
    // ...and its third opens it to show the tick again: the setting is the tool's, not the popover's.
    h.get_by_role_and_label(Role::Button, GEAR).click();
    h.run_steps(3);
    assert_eq!(check(&h, "From Center").accesskit_node().toggled(), Some(Toggled::True));
}

#[test]
fn a_fixed_size_typed_past_the_range_takes_the_closest_value_and_alerts() {
    // Done when: below 1 px or above 300,000 px is clamped and the alert says so.
    for (typed, want) in [("0", 1.0), ("400000", 300_000.0), ("120", 120.0)] {
        let mut h = bar();
        h.get_by_role_and_label(Role::Button, GEAR).click();
        h.run_steps(3);
        h.get_by_role_and_label(Role::RadioButton, "Fixed Size").click();
        h.run_steps(3);
        field(&h, "W:").click();
        h.run_steps(1);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text(typed.into()));
        h.run_steps(1);
        h.key_press(egui::Key::Tab);
        h.run_steps(3);
        let o = &h.state().ui.tool_options;
        assert_eq!((o.custom_constraint.as_str(), o.custom_fixed_width), ("fixedSize", want), "{typed} px");
        assert_eq!(crate::dialogs::alerts::alerts(h.state()).len(), usize::from(typed != "120"), "{typed}: the alert only past the range");
    }
}

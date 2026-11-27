//! #12103: the Healing Brush's options bar gains Use Legacy right after Aligned and Diffusion
//! after Sample, with the reference's words, tooltips and defaults (off, 5). Diffusion holds a
//! whole number 1..7: a typed 0 or 20 becomes 1 or 7 and the house "Closest value inserted" alert
//! says so. Use Legacy on hides the Diffusion field and off shows it again with the value it kept.
//! Reset Tool restores off / 5. The Clone Stamp's bar gets neither cell, and only the Healing
//! Brush's stroke params carry the two keys.

use egui::Modifiers;
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The options bar with `tool` active, and the dialog host its alerts open on.
fn bar(tool: Tool) -> H {
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

/// A tool with a document and a clone source set, for the params a stroke would carry.
fn armed(tool: Tool) -> Box<PhotocraftApp> {
    let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    a.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    a.ui.tool = tool;
    crate::retouch_ui::set_source(&mut a, 8.0, 8.0);
    a
}

/// The field named `label` ("Diffusion"), with or without its caption's colon.
fn field<'a>(h: &'a H, label: &str) -> Option<egui_kittest::Node<'a>> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == label))
}

fn check_box<'a>(h: &'a H, label: &str) -> Option<egui_kittest::Node<'a>> {
    h.query_all_by_role(Role::CheckBox).find(|n| n.accesskit_node().label().as_deref() == Some(label))
}

fn combo<'a>(h: &'a H, label: &str) -> Option<egui_kittest::Node<'a>> {
    h.query_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == label))
}

fn type_into(h: &mut H, label: &str, text: &str) {
    field(h, label).unwrap_or_else(|| panic!("no {label} field to type into")).click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

/// The Healing Brush's two new options: Use Legacy and Diffusion.
fn options(app: &PhotocraftApp) -> (bool, f32) {
    let o = &app.ui.tool_options;
    (o.healing_legacy, o.healing_diffusion)
}

#[test]
fn use_legacy_follows_aligned_and_diffusion_follows_sample() {
    let h = bar(Tool::Healing);
    let aligned = h.get_by_label("Aligned");
    let legacy = check_box(&h, "Use Legacy").expect("the Use Legacy check box");
    let sample = combo(&h, "Sample").expect("the Sample dropdown");
    let diffusion = field(&h, "Diffusion").expect("the Diffusion field");
    let (a, l, s, d) = (aligned.rect(), legacy.rect(), sample.rect(), diffusion.rect());
    assert!(a.right() <= l.left() && l.right() <= s.left() && s.right() <= d.left(), "Aligned, Use Legacy, Sample, Diffusion: {a:?} {l:?} {s:?} {d:?}");
    // The defaults the ticket names: off, and 5 in the field.
    assert_eq!(options(h.state()), (false, 5.0), "off / 5 by default");
    assert_eq!(diffusion.accesskit_node().numeric_value(), Some(5.0), "the field reads 5");
    // The reference's tooltips.
    assert_eq!(legacy.accesskit_node().description().as_deref(), Some("Use the legacy Healing Brush algorithm"), "Use Legacy's tooltip");
    assert_eq!(diffusion.accesskit_node().description().as_deref(), Some("Adjust the degree of diffusion"), "Diffusion's tooltip");
}

#[test]
fn the_clone_stamp_gets_neither_cell() {
    let h = bar(Tool::CloneStamp);
    assert!(check_box(&h, "Use Legacy").is_none(), "no Use Legacy on the Clone Stamp");
    assert!(field(&h, "Diffusion").is_none(), "no Diffusion field on the Clone Stamp");
    // Its own two cells are still there.
    assert!(combo(&h, "Sample").is_some(), "the Clone Stamp keeps Sample");
}

#[test]
fn use_legacy_hides_the_diffusion_field_and_shows_it_again_with_its_value() {
    let mut h = bar(Tool::Healing);
    type_into(&mut h, "Diffusion", "3");
    assert_eq!(options(h.state()), (false, 3.0), "3 before the toggle");
    h.get_by_label("Use Legacy").click();
    h.run_steps(2);
    assert!(field(&h, "Diffusion").is_none(), "Use Legacy on hides the Diffusion field");
    h.get_by_label("Use Legacy").click();
    h.run_steps(2);
    let back = field(&h, "Diffusion").expect("Use Legacy off shows the Diffusion field again");
    assert_eq!(back.accesskit_node().numeric_value(), Some(3.0), "and shows the value it kept");
    assert_eq!(options(h.state()), (false, 3.0), "the value was kept while hidden");
}

#[test]
fn a_typed_diffusion_outside_one_to_seven_gets_the_closest_value_and_the_alert() {
    let mut h = bar(Tool::Healing);
    let format = crate::i18n::framework_i18n().format();
    let (low, high) = (format.number_ungrouped(1.0, 0), format.number_ungrouped(7.0, 0));
    let message = format!("An integer between {low} and {high} is required. Closest value inserted.");
    for (typed, expected) in [("0", 1.0), ("20", 7.0)] {
        type_into(&mut h, "Diffusion", typed);
        assert_eq!(options(h.state()), (false, expected), "{typed} inserts the closest value");
        let dialogs = h.state().framework.dialogs.open_dialogs();
        assert_eq!(dialogs.len(), 1, "{typed} opens one alert");
        assert_eq!(dialogs[0].fields.as_ref().unwrap()["message"], message, "{typed} words the alert as the house does");
        h.get_by_label("OK").click();
        h.run_steps(4);
        assert!(h.state().framework.dialogs.open_dialogs().is_empty(), "OK closes the alert");
        assert_eq!(options(h.state()), (false, expected), "OK keeps the clamped value");
    }
}

#[test]
fn reset_tool_restores_use_legacy_off_and_diffusion_five() {
    let mut h = bar(Tool::Healing);
    type_into(&mut h, "Diffusion", "3");
    h.get_by_label("Use Legacy").click();
    h.run_steps(2);
    assert_eq!(options(h.state()), (true, 3.0), "the two options are set");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "tool.reset", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(options(h.state()), (false, 5.0), "Reset Tool gives off / 5");
    assert_eq!(field(&h, "Diffusion").expect("the field shows again").accesskit_node().numeric_value(), Some(5.0), "and the field reads 5");
}

#[test]
fn only_the_healing_brush_s_params_carry_the_two_keys() {
    let mut a = armed(Tool::Healing);
    let p = crate::retouch_ui::clone_params(&a).expect("a source is set");
    assert_eq!(p["diffusion"], json!(5.0), "the field's default rides the params");
    assert!(p.get("legacy").is_none(), "Use Legacy off sends no key");
    a.ui.tool_options.healing_legacy = true;
    a.ui.tool_options.healing_diffusion = 3.0;
    let p = crate::retouch_ui::clone_params(&a).expect("a source is set");
    assert_eq!(p["legacy"], json!(true), "Use Legacy on sends it");
    assert_eq!(p["diffusion"], json!(3.0), "and the field's value with it");
    // The Clone Stamp's params stay as they were, whatever the Healing Brush's bar holds.
    let b = armed(Tool::CloneStamp);
    let q = crate::retouch_ui::clone_params(&b).expect("a source is set");
    assert!(q.get("legacy").is_none() && q.get("diffusion").is_none(), "the Clone Stamp sends neither: {q}");
}

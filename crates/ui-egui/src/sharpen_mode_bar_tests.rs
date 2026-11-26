//! #12943: the Sharpen tool's options bar has its own Mode dropdown - Normal, Darken, Lighten,
//! Hue, Saturation, Color, Luminosity, with a line between the groups - tooltip "Painting Mode",
//! default Normal, sent with the stroke and set back to Normal by Reset Tool. Blur and Smudge
//! share the bar's branch: Blur still has no Mode (#12642) and each tool keeps its own value.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_color::BlendMode;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

const NAMES: [&str; 7] = ["Normal", "Darken", "Lighten", "Hue", "Saturation", "Color", "Luminosity"];
/// The seven rows as the bar's own values, in the reference application's order.
const ROWS: [(BlendMode, &str); 7] = [
    (BlendMode::Normal, "Normal"),
    (BlendMode::Darken, "Darken"),
    (BlendMode::Lighten, "Lighten"),
    (BlendMode::Hue, "Hue"),
    (BlendMode::Saturation, "Saturation"),
    (BlendMode::Color, "Color"),
    (BlendMode::Luminosity, "Luminosity"),
];

fn new_app(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    crate::theme::install_fonts(app.ctx());
    crate::theme::apply(app.ctx(), &crate::theme::AppTheme::pro());
    app
}

/// The options bar of `tool`, plus the dialog host its alerts open on.
fn bar(tool: Tool) -> H {
    let app = new_app(tool);
    let mut h = Harness::builder().with_size(vec2(1800.0, 700.0)).with_context(app.ctx().clone()).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut Box<PhotocraftApp>| {
            crate::panels::options_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// The bar's Mode dropdown, if it has one.
fn mode(h: &H) -> Option<egui_kittest::Node<'_>> {
    h.query_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.trim_end_matches(':') == "Mode"))
}

/// The Strength field, to place the dropdown against.
fn strength(h: &H) -> egui_kittest::Node<'_> {
    h.query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.trim_end_matches(':') == "Strength"))
        .expect("the Strength field")
}

/// The open list's entries, top to bottom.
fn list_tops(h: &H, names: &[&str]) -> Vec<f32> {
    names.iter().map(|name| h.query_all_by_label(name).map(|n| n.rect().top()).fold(f32::MIN, f32::max)).collect()
}

#[test]
fn the_sharpen_bar_lists_the_seven_modes_in_order_and_starts_at_normal() {
    let mut h = bar(Tool::Sharpen);
    assert_eq!(h.state().ui.tool_options.sharpen_mode, BlendMode::Normal, "the default is Normal");
    let dropdown = mode(&h).expect("Sharpen has a Mode dropdown");
    assert_eq!(dropdown.accesskit_node().value().as_deref(), Some("Normal"), "the field shows Normal");
    assert!(dropdown.rect().right() <= strength(&h).rect().left(), "Mode comes before Strength, right after the brush controls");
    let at = dropdown.rect().center();
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(4);
    let tops = list_tops(&h, &NAMES);
    assert!(tops.iter().all(|t| *t > f32::MIN), "the list is open: {tops:?}");
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "the entries are in the reference order: {tops:?}");
    for other in ["Multiply", "Screen", "Overlay", "Difference", "Dissolve"] {
        assert!(h.query_by_label(other).is_none(), "{other} is not a Sharpen mode");
    }
    h.get_by_label("Luminosity").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.tool_options.sharpen_mode, BlendMode::Luminosity, "picking a row sets the tool's Mode");
}

#[test]
fn hovering_the_dropdown_shows_painting_mode() {
    let mut h = bar(Tool::Sharpen);
    let center = mode(&h).expect("Mode").rect().center();
    h.hover_at(center);
    // Let the real tooltip delay pass on the harness clock.
    h.run_steps(60);
    assert!(h.query_all_by_role(Role::Tooltip).any(|n| n.accesskit_node().label().as_deref() == Some("Painting Mode")), "the tooltip shows");
}

#[test]
fn blur_has_no_mode_and_smudge_keeps_its_own_dropdown() {
    // #12642 owns the Blur tool's Mode; #12959 gave Smudge its own.
    let blur = bar(Tool::Blur);
    assert!(mode(&blur).is_none(), "Blur has no Mode dropdown yet");
    let smudge = bar(Tool::Smudge);
    assert!(mode(&smudge).is_some(), "Smudge keeps its Mode dropdown");
    assert_eq!(smudge.state().ui.tool_options.smudge_mode, BlendMode::Normal);
}

#[test]
fn the_stroke_sends_the_tools_own_mode_and_the_engine_takes_every_row() {
    let mut app = new_app(Tool::Sharpen);
    let (cmd, p) = crate::tools::retouch_ui::dab_params(&app, Tool::Sharpen).unwrap();
    assert_eq!((cmd.as_ref(), &p["mode"]), ("paint.sharpen", &json!("Normal")), "Normal by default");
    for (mode, name) in ROWS {
        app.ui.tool_options.sharpen_mode = mode;
        let (_, p) = crate::tools::retouch_ui::dab_params(&app, Tool::Sharpen).unwrap();
        assert_eq!(p["mode"], json!(name), "the bar sends {name}");
        // The engine takes every name the bar sends.
        app.run("paint.sharpen", json!({"points": [[40, 40]], "size": 20, "mode": name})).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    // Another tool's Mode never rides along, and the Sharpen bar does not overwrite it.
    app.ui.tool_options.smudge_mode = BlendMode::Hue;
    let (_, p) = crate::tools::retouch_ui::dab_params(&app, Tool::Sharpen).unwrap();
    assert_eq!(p["mode"], json!(NAMES[6]), "Sharpen sends its own Mode");
    let (_, p) = crate::tools::retouch_ui::dab_params(&app, Tool::Smudge).unwrap();
    assert_eq!(p["mode"], json!("Hue"), "Smudge sends its own Mode");
    let (_, p) = crate::tools::retouch_ui::dab_params(&app, Tool::Blur).unwrap();
    assert!(p.get("mode").is_none(), "Blur sends no Mode");
}

#[test]
fn reset_tool_sets_normal_and_leaves_the_other_tools_alone() {
    let mut app = new_app(Tool::Sharpen);
    let ctx = app.ctx().clone();
    app.ui.tool_options.sharpen_mode = BlendMode::Lighten;
    app.ui.tool_options.smudge_mode = BlendMode::Color;
    crate::menus::invoke(&mut app, &ctx, crate::tools::tool_reset::RESET, json!({})).unwrap();
    assert_eq!(app.ui.tool_options.sharpen_mode, BlendMode::Normal, "Reset Tool sets Sharpen's Mode back to Normal");
    assert_eq!(app.ui.tool_options.smudge_mode, BlendMode::Color, "the Smudge tool's Mode is not Sharpen's to reset");
    assert!(crate::tools::tool_reset::option_keys(Tool::Sharpen).contains(&"sharpen_mode"), "a Sharpen tool preset stores the Mode");
    assert!(!crate::tools::tool_reset::option_keys(Tool::Blur).contains(&"sharpen_mode"), "it is not a Blur option");
}

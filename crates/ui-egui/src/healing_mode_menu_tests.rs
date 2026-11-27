//! #12101: the Healing Brush's Mode menus gain Replace and the reference's two separators, in the
//! measured order Normal, Replace, separator, Multiply, Screen, Darken, Lighten, separator, Color,
//! Luminosity. Picking Replace is a tool option only: it adds no History step, sends
//! `mode:"replace"` on the next stroke, and any other mode pick leaves it. The keyboard cycle
//! stays inside the seven blend modes.

use craft::ui::patterns::blend_keys::{BlendShortcut, BlendStep};
use photocraft_color::BlendMode;
use serde_json::json;

use super::PaintMode;
use super::fill::mode_entries;
use crate::blend_keys::{fired, test_fired};
use crate::brush_mode_menu::{Item, current, items, labels};
use crate::state::Tool;
use crate::{PhotocraftApp, brush_mode_menu};

/// The measured row order of the canvas Shift+right-click menu, separators in place.
const MENU_ROWS: [Item; 10] = [
    Item::Blend(BlendMode::Normal),
    Item::HealingReplace,
    Item::Separator,
    Item::Blend(BlendMode::Multiply),
    Item::Blend(BlendMode::Screen),
    Item::Blend(BlendMode::Darken),
    Item::Blend(BlendMode::Lighten),
    Item::Separator,
    Item::Blend(BlendMode::Color),
    Item::Blend(BlendMode::Luminosity),
];

/// The same list as the options bar dropdown's entry values (`None` is a rule).
const BAR_ROWS: [Option<PaintMode>; 10] = [
    Some(PaintMode::Blend(BlendMode::Normal)),
    Some(PaintMode::HealingReplace),
    None,
    Some(PaintMode::Blend(BlendMode::Multiply)),
    Some(PaintMode::Blend(BlendMode::Screen)),
    Some(PaintMode::Blend(BlendMode::Darken)),
    Some(PaintMode::Blend(BlendMode::Lighten)),
    None,
    Some(PaintMode::Blend(BlendMode::Color)),
    Some(PaintMode::Blend(BlendMode::Luminosity)),
];

#[test]
fn both_healing_mode_lists_show_replace_and_the_two_separators() {
    assert_eq!(items(Tool::Healing), MENU_ROWS, "the Shift+right-click menu rows");
    assert_eq!(
        labels(Tool::Healing),
        ["Normal", "Replace", "Multiply", "Screen", "Darken", "Lighten", "Color", "Luminosity"],
        "the labels, separators left out"
    );
    let entries = mode_entries(Tool::Healing, true, false);
    let values: Vec<Option<PaintMode>> = entries.iter().map(|e| e.value().cloned()).collect();
    assert_eq!(values, BAR_ROWS, "the options bar dropdown");
    // Another tool's list is untouched by this: the Clone Stamp keeps the layer modes.
    assert!(!mode_entries(Tool::CloneStamp, true, false).iter().any(|e| e.value() == Some(&PaintMode::HealingReplace)), "no Replace on the Clone Stamp");
    assert!(items(Tool::CloneStamp).iter().all(|i| *i != Item::HealingReplace), "no Replace row on the Clone Stamp menu");
}

/// A Healing Brush app with a document, a set clone source and the tool active.
fn app() -> Box<PhotocraftApp> {
    let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    a.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    a.ui.tool = Tool::Healing;
    crate::retouch_ui::set_source(&mut a, 8.0, 8.0);
    a
}

/// The mode the next stroke would carry in its params.
fn stroke_mode(a: &PhotocraftApp) -> String {
    crate::retouch_ui::clone_params(a).expect("a source is set")["mode"].as_str().unwrap_or_default().to_owned()
}

fn steps(a: &PhotocraftApp) -> usize {
    a.session.active().unwrap().history.past_len()
}

#[test]
fn the_canvas_menu_replace_row_switches_the_stroke_without_a_history_step() {
    let mut a = app();
    let before = steps(&a);
    brush_mode_menu::choose(&mut a, Item::HealingReplace);
    assert!(a.ui.tool_options.healing_replace, "the row sets the tool option");
    assert_eq!(stroke_mode(&a), "replace", "the next stroke sends mode:\"replace\"");
    assert_eq!(current(&a, Tool::Healing), Some(Item::HealingReplace), "the menu checks Replace");
    assert_eq!(steps(&a), before, "picking the Mode adds no History step");
    // An existing row leaves Replace and paints its blend mode.
    brush_mode_menu::choose(&mut a, Item::Blend(BlendMode::Multiply));
    assert!(!a.ui.tool_options.healing_replace, "Multiply leaves Replace");
    assert_eq!(a.session.tools.brush.mode, BlendMode::Multiply);
    assert_eq!(stroke_mode(&a), "Multiply");
    assert_eq!(steps(&a), before, "picking a Mode never adds a History step");
    // Replace is not offered to the other tools' choose route: their rows and options stay.
    let mut b = app();
    b.ui.tool = Tool::CloneStamp;
    brush_mode_menu::choose(&mut b, Item::HealingReplace);
    assert!(!b.ui.tool_options.healing_replace, "the Clone Stamp never takes Replace");
}

#[test]
fn the_options_bar_dropdown_replaces_and_picking_a_blend_mode_leaves_it() {
    use crate::app::framework::BuildApp as _;
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    // The options bar alone, like the other bar tests: open the Mode dropdown, click a row.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.ui.tool = Tool::Healing;
    crate::retouch_ui::set_source(&mut app, 8.0, 8.0);
    let mut h = Harness::builder().with_size(egui::vec2(1800.0, 300.0)).with_step_dt(1.0 / 60.0).build_app(|ui, app| crate::panels::options_bar(app, ui), app);
    h.run_steps(4);
    assert_eq!(h.get_by_role_and_label(Role::ComboBox, "Mode").accesskit_node().value().as_deref(), Some("Normal"), "starts at Normal");
    let before = steps(h.state());

    h.get_by_role_and_label(Role::ComboBox, "Mode").click();
    h.run_steps(2);
    h.get_by_label("Replace").click();
    h.run_steps(3);
    assert!(h.state().ui.tool_options.healing_replace, "the dropdown's Replace row sets the option");
    assert_eq!(h.get_by_role_and_label(Role::ComboBox, "Mode").accesskit_node().value().as_deref(), Some("Replace"), "the dropdown reads Replace");
    assert_eq!(crate::retouch_ui::clone_params(h.state()).expect("the source is set")["mode"], json!("replace"), "the next stroke would run in Replace");
    assert_eq!(steps(h.state()), before, "picking a Mode adds no History step");

    h.get_by_role_and_label(Role::ComboBox, "Mode").click();
    h.run_steps(2);
    h.get_by_label("Multiply").click();
    h.run_steps(3);
    assert!(!h.state().ui.tool_options.healing_replace, "Multiply leaves Replace");
    assert_eq!(h.state().session.tools.brush.mode, BlendMode::Multiply, "the brush takes the mode");
    assert_eq!(crate::retouch_ui::clone_params(h.state()).expect("the source is set")["mode"], json!("Multiply"));
    assert_eq!(steps(h.state()), before, "and no Mode pick ever adds a step");
}

#[test]
fn the_mode_keys_keep_the_seven_modes_and_clear_replace() {
    // Replace stays out of the letter keys and the ⇧+ / ⇧− cycle: the seven modes and their order.
    let mut a = app();
    a.ui.tool_options.healing_replace = true;
    assert!(fired(&mut a, &test_fired(BlendShortcut::Multiply.into())), "⇧⌥M is taken");
    assert_eq!(a.session.tools.brush.mode, BlendMode::Multiply);
    assert!(!a.ui.tool_options.healing_replace, "a letter pick leaves Replace");
    let mut b = app();
    assert!(fired(&mut b, &test_fired(BlendStep::Next.into())), "⇧+ is taken");
    assert_eq!(b.session.tools.brush.mode, BlendMode::Multiply, "the cycle still runs Normal to Multiply");
    assert!(!b.ui.tool_options.healing_replace, "the cycle never picks Replace");
}

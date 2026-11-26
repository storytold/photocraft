//! #12607: Clone Stamp clamps and explains out-of-range edits and supplies its options-bar tooltips.

use egui::accesskit::Role;
use egui::{Modifiers, PointerButton, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::app::framework::BuildApp as _;
use crate::state::Tool;

fn bar(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(1800.0, 300.0)).with_step_dt(1.0 / 60.0).build_app(
        |ui, app| {
            crate::panels::options_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

fn control<'a>(h: &'a Harness<'static, Box<PhotocraftApp>>, role: Role, label: &str) -> egui_kittest::Node<'a> {
    h.query_all_by_role(role)
        .find(|node| node.accesskit_node().label().is_some_and(|name| name.trim_end_matches(':') == label))
        .unwrap_or_else(|| panic!("could not find the {label} control"))
}

fn type_into(h: &mut Harness<'static, Box<PhotocraftApp>>, field: &str, text: &str) {
    let pos = control(h, Role::SpinButton, field).rect().center();
    h.hover_at(pos);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

#[test]
fn clone_stamp_commits_whole_percents_and_reports_clamps() {
    let mut h = bar(Tool::CloneStamp);
    let format = crate::i18n::framework_i18n().format();
    let fractional = format.number_ungrouped(33.7, 1);
    let (low, high) = (format.number_ungrouped(1.0, 0), format.number_ungrouped(100.0, 0));
    let range_alert = format!("An integer between {low}% and {high}% is required. Closest value inserted.");
    for field in ["Opacity", "Flow"] {
        for (typed, expected, clamps) in [(fractional.as_str(), 0.34, false), ("150", 1.0, true), ("0", 0.01, true), ("1", 0.01, false), ("100", 1.0, false)] {
            let pair = |app: &PhotocraftApp| {
                let b = &app.session.tools.brush;
                if field == "Opacity" { (b.opacity, b.flow) } else { (b.flow, b.opacity) }
            };
            let other = pair(h.state()).1;
            type_into(&mut h, field, typed);
            assert_eq!(pair(h.state()), (expected, other), "{field} {typed} edits only that field");
            let dialogs = h.state().framework.dialogs.open_dialogs();
            if clamps {
                assert_eq!(dialogs.len(), 1, "{field} {typed} opens one alert");
                assert_eq!(dialogs[0].fields.as_ref().unwrap()["message"], range_alert);
                h.get_by_label("OK").click();
                h.run_steps(4);
                assert_eq!(pair(h.state()), (expected, other), "OK keeps the clamped value");
            }
            assert!(h.state().framework.dialogs.open_dialogs().is_empty(), "{field} {typed} leaves no alert open");
        }
    }
}

#[test]
fn history_brush_keeps_fractional_edits_and_silent_clamping() {
    let mut h = bar(Tool::HistoryBrush);
    let fractional = crate::i18n::framework_i18n().format().number_ungrouped(33.7, 1);
    type_into(&mut h, "Opacity", &fractional);
    type_into(&mut h, "Flow", "0");
    let brush = &h.state().session.tools.brush;
    assert!((brush.opacity - 0.337).abs() < 1e-6, "History Brush keeps the fractional percent");
    assert_eq!(brush.flow, 0.01);
    assert!(h.state().framework.dialogs.open_dialogs().is_empty(), "History Brush clamps silently");
}

#[test]
fn clone_stamp_controls_show_their_tooltips_on_hover() {
    let mut h = bar(Tool::CloneStamp);
    for (label, tip) in [
        ("Mode", "Painting Mode"),
        ("Opacity", "Set opacity for stroke"),
        ("Flow", "Set flow rate for stroke"),
        ("Aligned", "Keep the same source offset between strokes"),
        ("Sample", "Choose which layers to sample"),
        ("Brush Preset picker", "Open the Brush Preset picker"),
        ("Toggle the Brush Settings panel", "Toggle the Brush Settings panel  (F5)"),
    ] {
        let pos = match label {
            "Mode" | "Sample" | "Brush Preset picker" => control(&h, Role::ComboBox, label).rect().center(),
            "Opacity" | "Flow" => control(&h, Role::SpinButton, label).rect().center(),
            _ => h.get_by_label(label).rect().center(),
        };
        h.hover_at(pos);
        // Let the real tooltip delay pass on the harness clock.
        h.run_steps(45);
        assert!(
            h.query_all_by_role(Role::Tooltip).any(|node| node.accesskit_node().label().as_deref() == Some(tip)),
            "{label} should show {tip:?} when hovered"
        );
    }
}

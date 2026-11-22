//! The options-bar field and the canvas popup must agree on the Background Eraser's compact rows.

use super::*;
use egui::{Event, Modifiers, PointerButton, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

fn field(tool: Tool, enabled: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.ui.tool = tool;
    crate::theme::install_fonts(app.ctx());
    crate::theme::apply(app.ctx(), &crate::theme::AppTheme::pro());
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(vec2(800.0, 600.0))
        .build_ui_state(move |ui, app: &mut Box<PhotocraftApp>| show_field(ui, app, enabled), app);
    h.run_steps(4);
    h
}

fn open_field(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    h.get_all_by_role(egui::accesskit::Role::ComboBox).next().expect("the closed brush field").click();
    h.run_steps(4);
}

#[test]
fn background_eraser_field_opens_compact_rows_without_the_preset_browser() {
    let mut h = field(Tool::BackgroundEraser, true);
    let history = h.state().session.active().unwrap().history.past_len();
    open_field(&mut h);
    assert!(
        h.get_by_role_and_label(egui::accesskit::Role::Slider, "Spacing").accesskit_node().numeric_value().is_some(),
        "the compact popup's spacing slider is visible"
    );
    assert!(h.query_by_label("Size:").is_some(), "the compact size pen control is visible");
    assert!(h.query_by_label("Tolerance:").is_some(), "the tolerance pen control is not replaced by presets");
    assert!(h.query_by_label(crate::bg_eraser_popup::PRESSURE_WARNING).is_some());
    assert!(h.query_by_label("Brush Preset Options").is_none(), "compact has no preset gear");
    assert!(h.query_by_label("New Brush Preset").is_none());
    assert_eq!(
        h.query_all_by_role(egui::accesskit::Role::TextInput).filter(|node| !node.accesskit_node().is_hidden()).count(),
        0,
        "compact has no visible preset search field; hidden numeric editors are not search"
    );
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history, "opening the picker does not edit the document");
}

#[test]
fn brush_field_keeps_the_full_preset_picker() {
    let mut h = field(Tool::Brush, true);
    open_field(&mut h);
    assert!(h.query_by_label("Brush Preset Options").is_some());
    assert!(h.query_by_label("New Brush Preset").is_some());
    assert!(h.query_by_label("Brush Settings…").is_some());
    assert!(h.query_by_label("Tolerance:").is_none());
    assert!(
        h.query_all_by_role(egui::accesskit::Role::TextInput).any(|node| !node.accesskit_node().is_hidden()),
        "the full picker's visible search remains available, not merely its hidden numeric editors"
    );
}

#[test]
fn compact_field_commits_its_tolerance_control_without_a_document_history_step() {
    let mut h = field(Tool::BackgroundEraser, true);
    let history = h.state().session.active().unwrap().history.past_len();
    open_field(&mut h);
    h.get_by_label("Tolerance:").click();
    h.run_steps(3);
    h.get_by_label("Stylus Wheel").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_options.bg_tolerance_control, crate::bg_eraser_popup::STYLUS_WHEEL);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
}

#[test]
fn disabled_brush_field_does_not_open_or_edit() {
    for tool in [Tool::BackgroundEraser, Tool::Brush] {
        let mut h = field(tool, false);
        let history = h.state().session.active().unwrap().history.past_len();
        let tolerance = h.state().ui.tool_options.bg_tolerance_control.clone();
        let brush = h.state().session.tools.brush.clone();
        let at = h.get_all_by_role(egui::accesskit::Role::ComboBox).next().expect("the disabled field").rect().center();
        h.event(Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.run_steps(1);
        }
        h.run_steps(3);
        assert!(h.query_by_label("Tolerance:").is_none());
        assert!(h.query_by_label("Brush Preset Options").is_none());
        assert_eq!(h.state().ui.tool_options.bg_tolerance_control, tolerance);
        assert_eq!(h.state().session.tools.brush, brush);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    }
}

#[test]
fn changing_tools_refreshes_the_open_fields_compact_rows() {
    let mut h = field(Tool::BackgroundEraser, true);
    let history = h.state().session.active().unwrap().history.past_len();
    open_field(&mut h);
    assert!(h.query_by_label("Tolerance:").is_some());
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(4);
    assert!(h.query_by_label("Tolerance:").is_none(), "the old tool's custom rows are not cached into Brush");
    assert!(h.query_by_label("Brush Preset Options").is_some());
    h.state_mut().ui.tool = Tool::BackgroundEraser;
    h.run_steps(4);
    assert!(h.query_by_label("Tolerance:").is_some(), "the memoized field observes the new tool's row inputs");
    assert!(h.query_by_label("Brush Preset Options").is_none());
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
}

//! #14649: the Opacity row that Edit › Fill and Edit › Stroke share takes whole percents from 1 to 100. A typed value
//! past them is clamped, once the field is left, with the range alert titled after the dialog; letters restore the
//! previous value without one. A typed fraction (the reference drops the dot) is left to craft-libs. A 0% remembered
//! before the row started at 1 reopens at 1%.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog};
use craft::ui::runtime::memory::{SaveKey, set_saved_value};
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;

type H = Harness<'static, Box<PhotocraftApp>>;

/// A document with a selection and the dialog `open` opens, hosted with its alerts.
fn dialog(open: fn(&mut PhotocraftApp)) -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    app.run("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
    crate::theme::install_fonts(app.ctx());
    crate::theme::apply(app.ctx(), &crate::theme::AppTheme::pro());
    open(&mut app);
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(vec2(1200.0, 900.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    h.run_steps(4);
    h
}

fn fill(app: &mut PhotocraftApp) {
    super::open(app);
}

fn stroke(app: &mut PhotocraftApp) {
    crate::dialogs::stroke_ui::open(app);
}

fn opacity<'a>(h: &'a H) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == "Opacity")).expect("the Opacity field")
}

fn value(h: &H) -> Option<f64> {
    opacity(h).accesskit_node().numeric_value()
}

/// Types `text` over Opacity and leaves the field with Tab.
fn type_and_tab(h: &mut H, text: &str) {
    let at = opacity(h).rect().center();
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        h.run_steps(1);
    }
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(4);
}

/// The message of the open alert, if any.
fn alert(h: &H) -> Option<String> {
    let dialogs = h.state().framework.dialogs.open_dialogs();
    dialogs.iter().find(|d| d.key.as_deref() == Some("alert")).and_then(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_string))
}

/// Answers the open alert, leaving the dialog `command` open.
fn dismiss(h: &mut H, command: &str) {
    let dialogs = h.state().framework.dialogs.open_dialogs();
    for id in dialogs.iter().filter(|d| d.key.as_deref() != Some(command)).map(|d| d.id) {
        h.state_mut().framework.dialogs.close_by_id(id);
    }
    h.run_steps(3);
}

/// The names of the open dialog windows, unnamed ones included.
fn titles(h: &H) -> Vec<Option<String>> {
    h.query_all_by_role(Role::Dialog).chain(h.query_all_by_role(Role::AlertDialog)).map(|n| n.accesskit_node().label()).collect()
}

fn check(open: fn(&mut PhotocraftApp), command: &str, title: &str) {
    let mut h = dialog(open);
    // Not `12.3`: the framework reads it with the system's decimal separator, so the result depends on the machine.
    for (typed, kept) in [("0", 1.0), ("150", 100.0), ("50", 50.0)] {
        type_and_tab(&mut h, typed);
        assert_eq!(value(&h), Some(kept), "{title}: typed {typed}");
        if kept == 50.0 {
            assert_eq!(alert(&h), None, "{title}: {typed} is in range");
            continue;
        }
        let message = alert(&h).unwrap_or_else(|| panic!("{title}: {typed} raises the range alert"));
        assert!(message.contains("integer between 1") && message.contains("100"), "{title}: {message}");
        let windows = titles(&h);
        assert!(windows.len() == 2 && windows.iter().all(|t| t.as_deref() == Some(title)), "{title}: the dialog and its alert: {windows:?}");
        dismiss(&mut h, command);
    }
    type_and_tab(&mut h, "42");
    type_and_tab(&mut h, "abc");
    assert_eq!(value(&h), Some(42.0), "{title}: letters restore the previous value");
    assert_eq!(alert(&h), None, "{title}: letters raise no alert");
}

#[test]
fn fill_opacity_takes_1_to_100_with_the_range_alert() {
    check(fill, super::COMMAND, "Fill");
}

#[test]
fn stroke_opacity_takes_1_to_100_with_the_range_alert() {
    check(stroke, crate::dialogs::stroke_ui::COMMAND, "Stroke");
}

/// A dialog's opener.
type Open = fn(&mut PhotocraftApp) -> DialogId;

/// The Opacity the dialog `open` (saving under `command`) opens at once `saved` is its remembered Opacity, or with no
/// remembered Opacity for `None`.
fn reopened(open: Open, command: &str, saved: Option<f64>) -> Option<f64> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    if let Some(opacity) = saved {
        let mut fields = DialogFields::new();
        fields.set("opacity", opacity);
        set_saved_value::<Option<DialogFields>>(app.ctx(), &SaveKey::named(crate::dialogs::params::memory_scope(command)), Some(fields));
    }
    let id = open(&mut app);
    app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("the dialog is open").fields().number("opacity")
}

/// A 0% remembered while the row started at 0 reopens at the new 1% minimum, not at the 100% default. A dialog that
/// never remembered an Opacity still opens at 100%, and a remembered Opacity in range is kept.
#[test]
fn a_remembered_0_percent_reopens_at_1_percent() {
    let dialogs: [(Open, &str); 2] = [(super::open, super::COMMAND), (crate::dialogs::stroke_ui::open, crate::dialogs::stroke_ui::COMMAND)];
    for (open, command) in dialogs {
        assert_eq!(reopened(open, command, Some(0.0)), Some(1.0), "{command}: a remembered 0");
        assert_eq!(reopened(open, command, None), Some(100.0), "{command}: nothing remembered");
        assert_eq!(reopened(open, command, Some(50.0)), Some(50.0), "{command}: a remembered 50");
    }
}

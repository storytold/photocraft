//! #12533: the Paint Bucket's Tolerance takes whole numbers 0..255 and Opacity whole percents
//! 1..100; a value typed past either range becomes the closest one with the reference's "Closest
//! value inserted" alert. A typed fraction rounds to the closest whole number: in Tolerance with
//! the same alert, in Opacity without one. The seven controls carry the
//! reference's tooltips, and the Source dropdown has no "Fill" label. The Magic Wand and the Magic
//! Eraser share Tolerance and keep their own behaviour.

use std::cell::Cell;

use craft::ui::runtime::env::Env;
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

const TOLERANCE_ALERT: &str = "An integer between 0 and 255 is required. Closest value inserted.";
const OPACITY_ALERT: &str = "An integer between 1% and 100% is required. Closest value inserted.";

thread_local! {
    /// The decimal mark the bar's fields read, from the env they are drawn in: the system region's
    /// unless the host sets one, so a typed fraction must use it to be read as one.
    static MARK: Cell<char> = const { Cell::new('.') };
}

/// The options bar with `tool` active, and the dialog host its alerts open on.
fn bar(tool: Tool) -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    app.ui.tool_options.tolerance = 32.0;
    app.ui.tool_options.fill_opacity = 37.0;
    crate::theme::install_fonts(app.ctx());
    crate::theme::apply(app.ctx(), &crate::theme::AppTheme::pro());
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut Box<PhotocraftApp>| {
            MARK.set(Env::of(ui).format().separators().0);
            crate::panels::options_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// `text` with `.` replaced by the fields' decimal mark.
fn localized(text: &str) -> String {
    text.replace('.', &MARK.get().to_string())
}

/// The number field named `label`, with or without its caption's colon.
fn field<'a>(h: &'a H, label: &str) -> Option<egui_kittest::Node<'a>> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == label))
}

/// Types `text` (with `.` as the decimal mark) into the field named `label` and commits it with Enter.
fn type_into(h: &mut H, label: &str, text: &str) {
    let text = localized(text);
    field(h, label).unwrap_or_else(|| panic!("the {label} field")).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

/// The messages of the open framework alerts.
fn alerts(h: &H) -> Vec<String> {
    h.state().framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_string)).collect()
}

/// The description (tooltip) of the control labelled `label`: not its caption's text node.
fn tip(h: &H, label: &str) -> Option<String> {
    let named = move |n: &egui_kittest::kittest::AccessKitNode<'_>| n.role() != Role::Label && n.label().is_some_and(|l| l.trim_end_matches(':') == label);
    let node = h.query_all(egui_kittest::kittest::by().predicate(named)).next()?;
    node.accesskit_node().description().map(|d| d.to_string())
}

#[test]
fn tolerance_past_0_255_takes_the_closest_value_and_alerts() {
    for (typed, want) in [("300", 255.0), ("256", 255.0), ("-5", 0.0)] {
        let mut h = bar(Tool::PaintBucket);
        type_into(&mut h, "Tolerance", typed);
        assert_eq!(h.state().ui.tool_options.tolerance, want, "{typed}");
        assert_eq!(alerts(&h), vec![TOLERANCE_ALERT.to_string()], "{typed}: one alert");
    }
}

#[test]
fn tolerance_is_a_whole_number() {
    for (typed, want) in [("12.7", 13.0), ("12.4", 12.0), ("255", 255.0), ("0", 0.0), ("abc", 32.0)] {
        let mut h = bar(Tool::PaintBucket);
        type_into(&mut h, "Tolerance", typed);
        assert_eq!(h.state().ui.tool_options.tolerance, want, "{typed}");
        if !typed.contains('.') {
            assert!(alerts(&h).is_empty(), "{typed}: no alert, got {:?}", alerts(&h));
        }
    }
}

#[test]
fn a_fractional_tolerance_takes_the_closest_value_and_alerts() {
    for (typed, want) in [("12.7", 13.0), ("12.4", 12.0), ("0.4", 0.0), ("254.6", 255.0)] {
        let mut h = bar(Tool::PaintBucket);
        type_into(&mut h, "Tolerance", typed);
        assert_eq!(h.state().ui.tool_options.tolerance, want, "{typed}");
        assert_eq!(alerts(&h), vec![TOLERANCE_ALERT.to_string()], "{typed}: one alert");
    }
}

#[test]
fn a_fractional_opacity_rounds_without_an_alert() {
    for (typed, want) in [("50.5", 51.0), ("50.4", 50.0), ("1.4", 1.0), ("99.6", 100.0)] {
        let mut h = bar(Tool::PaintBucket);
        type_into(&mut h, "Opacity", typed);
        assert_eq!(h.state().ui.tool_options.fill_opacity, want, "{typed}");
        assert!(alerts(&h).is_empty(), "{typed}: no alert, got {:?}", alerts(&h));
    }
}

#[test]
fn opacity_past_1_100_takes_the_closest_value_and_alerts() {
    for (typed, want) in [("150", 100.0), ("0", 1.0), ("-3", 1.0)] {
        let mut h = bar(Tool::PaintBucket);
        type_into(&mut h, "Opacity", typed);
        assert_eq!(h.state().ui.tool_options.fill_opacity, want, "{typed}");
        assert_eq!(alerts(&h), vec![OPACITY_ALERT.to_string()], "{typed}: one alert");
    }
}

#[test]
fn opacity_in_range_rounds_to_a_whole_percent_without_an_alert() {
    for (typed, want) in [("50.5", 51.0), ("50.4", 50.0), ("100", 100.0), ("1", 1.0), ("abc", 37.0)] {
        let mut h = bar(Tool::PaintBucket);
        type_into(&mut h, "Opacity", typed);
        assert_eq!(h.state().ui.tool_options.fill_opacity, want, "{typed}");
        assert!(alerts(&h).is_empty(), "{typed}: no alert, got {:?}", alerts(&h));
    }
}

#[test]
fn the_seven_controls_have_the_reference_tooltips() {
    let h = bar(Tool::PaintBucket);
    for (control, want) in [
        ("Source", "Set source for fill area"),
        ("Mode", "Fill Mode"),
        ("Opacity", "Set opacity for fill"),
        ("Tolerance", "Range of colors to fill"),
        ("Anti-alias", "Smoothes edge transition"),
        ("Contiguous", "Fill only contiguous pixels"),
        ("All Layers", "Fill composite image"),
    ] {
        assert_eq!(tip(&h, control).as_deref(), Some(want), "{control}");
    }
}

#[test]
fn the_source_dropdown_has_no_fill_label() {
    let h = bar(Tool::PaintBucket);
    assert!(h.query_by_label("Fill").is_none() && h.query_by_label("Fill:").is_none(), "a Fill label is shown");
    assert!(h.query_all_by_role(Role::ComboBox).any(|n| n.accesskit_node().label().as_deref() == Some("Source")), "the Source dropdown is shown");
}

/// The Magic Wand and the Magic Eraser share Tolerance and keep today's behaviour: a value past
/// 0..255 is clamped without an alert.
#[test]
fn magic_wand_and_magic_eraser_tolerance_are_unchanged() {
    for tool in [Tool::MagicWand, Tool::MagicEraser] {
        for (typed, want) in [("300", 255.0), ("-5", 0.0), ("128", 128.0)] {
            let mut h = bar(tool);
            type_into(&mut h, "Tolerance", typed);
            assert_eq!(h.state().ui.tool_options.tolerance, want, "{tool:?} {typed}");
            assert!(alerts(&h).is_empty(), "{tool:?} {typed}: no alert, got {:?}", alerts(&h));
        }
    }
}

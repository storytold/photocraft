//! #14639: the Exposure dialog shows values at one and above with a plus sign (`+1`,
//! `+1.23`, `+9.99`), as the reference application does. Only these three rows opt in.

use crate::widgets::{format_field_text, parse_num};

/// The Exposure, Offset and Gamma rows prefix values at one and above with `+`; anything
/// below one (and zero and negatives) reads as today.
#[test]
fn exposure_fields_show_plus_at_one_and_above() {
    // Exposure (two decimals): the dialog opens at 0, Gamma at `+1`.
    assert_eq!(format_field_text(1.0, Some(2), false, false, false, true), "+1");
    assert_eq!(format_field_text(1.23456, Some(2), false, false, false, true), "+1.23");
    assert_eq!(format_field_text(20.0, Some(2), false, false, false, true), "+20");
    assert_eq!(format_field_text(0.12, Some(2), false, false, false, true), "0.12");
    assert_eq!(format_field_text(-1.0, Some(2), false, false, false, true), "-1");
    assert_eq!(format_field_text(0.0, Some(2), false, false, false, true), "0");
    // Offset (four decimals) stays plain below one.
    assert_eq!(format_field_text(0.5, Some(4), false, false, false, true), "0.5");
    // Gamma Correction (small range: two decimals picked by the range) defaults to `+1`.
    assert_eq!(format_field_text(1.0, None, true, false, false, true), "+1");
    assert_eq!(format_field_text(9.99, None, true, false, false, true), "+9.99");
}

/// Without the opt-in the same values read as today, so every other dialog is unchanged.
#[test]
fn other_rows_keep_todays_display() {
    assert_eq!(format_field_text(1.0, Some(2), false, false, false, false), "1");
    assert_eq!(format_field_text(20.0, Some(2), false, false, false, false), "20");
    assert_eq!(format_field_text(9.99, None, true, false, false, false), "9.99");
}

/// Typing with or without `+` reads the same number, so stored values are unchanged.
#[test]
fn typed_plus_sign_parses_like_a_plain_number() {
    assert_eq!(parse_num("+1.23"), Some(1.23));
    assert_eq!(parse_num("+20"), Some(20.0));
    assert_eq!(parse_num("1.23"), Some(1.23));
}

/// #14639: after an arrow step the field selects the text it shows, plus sign included, so a
/// typed number replaces the whole value rather than leaving its last digit behind.
#[test]
fn stepping_a_plus_prefixed_field_selects_the_whole_shown_value() {
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut h = Harness::builder().with_size(egui::vec2(300.0, 400.0)).build_ui_state(
        |ui, values: &mut serde_json::Value| {
            let cx = crate::adjust_editors::EditorCx {
                mem: egui::Id::unique("plus-selection"),
                hist: None,
                gray: false,
                swatches: [[0.0; 3], [1.0; 3]],
                gradients: Vec::new(),
                dialog: true,
                dynamic_color_sliders: true,
                isolated_curve: None,
            };
            crate::adjust_editors::editor(ui, "exposure", values, &cx);
        },
        serde_json::json!({}),
    );
    crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    let gamma = |h: &Harness<'static, serde_json::Value>| h.query_all_by_role(Role::SpinButton).nth(2).unwrap().value().unwrap_or_default();
    assert_eq!(gamma(&h), "+1", "the dialog opens Gamma at one, shown with its plus");
    h.query_all_by_role(Role::SpinButton).nth(2).unwrap().click();
    h.run_steps(2);
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(gamma(&h), "+1.01", "an arrow step keeps the plus");
    // The step selected the whole shown text: typing replaces it, rather than leaving `1` behind
    // (`+1.01` with four characters selected would read `21`).
    h.event(egui::Event::Text("2".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state()["gamma"].as_f64(), Some(2.0), "a typed 2 replaces +1.01: {}", h.state());
    assert_eq!(gamma(&h), "+2");
}

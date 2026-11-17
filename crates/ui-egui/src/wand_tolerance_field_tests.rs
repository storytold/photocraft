//! #11786: the Magic Wand's Tolerance is a whole number 0-255, and the wand floors what it is given
//! (`wand_region`: `(tolerance + 1e-3).floor()`). A field that keeps a fraction shows a number the tool does not use.
//! Typing `12.5` must leave 13 behind, so the field and the wand agree; 300 and -5 clamp. The decimal comma, typed
//! expressions and the Command+arrow steps are the framework number field's own behaviour (`commit_decimals(0)`).

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The Magic Wand's options bar, with a document open, after a few settled frames.
fn wand_bar() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.ui.tool = Tool::MagicWand;
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1000.0, 60.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::studio());
    h.run_steps(4);
    h
}

/// The number the Tolerance field shows, as a person reads it off the screen.
fn shown_tolerance(h: &Harness<'_, PhotocraftApp>) -> f32 {
    let node = h.get_by_role(Role::SpinButton);
    node.accesskit_node().value().unwrap_or_default().parse().unwrap_or(f32::NAN)
}

/// Types `text` over the field's number and presses Enter, as a person would, and returns the option the edit left
/// behind.
fn type_tolerance(h: &mut Harness<'_, PhotocraftApp>, text: &str) -> f32 {
    h.get_by_role(Role::SpinButton).click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    h.state().ui.tool_options.tolerance
}

/// A typed decimal rounds to the nearest whole number: the wand's option is never a fraction for it to floor
/// differently from what the field shows.
#[test]
fn typed_decimals_round_to_the_nearest_whole_number() {
    for (text, want) in [("12.5", 13.0), ("12.4", 12.0), ("12.6", 13.0)] {
        let mut h = wand_bar();
        assert_eq!(type_tolerance(&mut h, text), want, "typing {text}");
        assert_eq!(shown_tolerance(&h), want, "the field shows the number it kept ({text})");
    }
}

/// A typed number outside the range clamps, as in any value field.
#[test]
fn typed_values_outside_the_range_clamp() {
    for (text, want) in [("300", 255.0), ("-5", 0.0)] {
        let mut h = wand_bar();
        assert_eq!(type_tolerance(&mut h, text), want, "typing {text}");
        assert_eq!(shown_tolerance(&h), want, "the field shows the clamped number ({text})");
    }
}

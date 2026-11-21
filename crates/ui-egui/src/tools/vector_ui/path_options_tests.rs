//! Path Options (#9713): the Pen gear's Thickness and Color are one display setting shared by every
//! path tool, as in Photoshop, not a tool option; its Rubber Band is the Pen's own tool option
//! (#12900). The Color swatch opens the Color Picker.

use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::test_input::harness;
use crate::PhotocraftApp;
use crate::color_picker_ui::{PickTarget, target, testing, top};
use crate::state::Tool;

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

/// The gear's settings as the user leaves them: red, 2.5 px, Rubber Band on.
fn edited(app: &mut PhotocraftApp) {
    let mut options = super::path_options(app);
    (options.color, options.thickness, options.rubber_band) = (Some([255, 42, 42]), Some(2.5), true);
    super::store_path_options(app, &options);
}

/// One setting for every path tool: Reset Tool and Reset All Tools leave Thickness and Color as
/// they are, and Reset Tool on the Pen turns its Rubber Band off.
#[test]
fn thickness_and_color_are_one_display_setting_not_a_tool_option() {
    let mut app = app();
    let ctx = egui::Context::default();
    edited(&mut app);
    let tool = &app.ui.tool_options.path_options;
    assert_eq!((tool.color, tool.thickness), (None, None), "the tool options hold no colour or thickness");
    for t in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen, Tool::Rectangle, Tool::PathSelection] {
        app.ui.tool = t;
        crate::menus::invoke(&mut app, &ctx, "tool.reset", json!({})).unwrap();
        let shown = super::path_options(&app);
        assert_eq!((shown.color, shown.thickness), (Some([255, 42, 42]), Some(2.5)), "Reset Tool on {t:?}");
    }
    assert!(!super::path_options(&app).rubber_band, "Reset Tool on the Pen turns its Rubber Band off");
    crate::menus::invoke(&mut app, &ctx, crate::tool_reset::RESET_ALL, json!({"confirmed": true})).unwrap();
    let shown = super::path_options(&app);
    assert_eq!((shown.color, shown.thickness), (Some([255, 42, 42]), Some(2.5)), "Reset All Tools");
}

/// A thickness outside the gear's range (a hand-edited setting) is not drawn: the theme's line is.
#[test]
fn a_thickness_out_of_range_draws_the_themes_line() {
    let mut app = app();
    for bad in [f32::NAN, f32::INFINITY, 0.1, 99.0] {
        app.ui.view.path_options.thickness = Some(bad);
        assert_eq!(super::path_options(&app).thickness, None, "{bad}");
    }
}

/// The Pen gear's Color swatch opens the Color Picker; OK sets the colour every path tool draws in.
#[test]
fn the_color_swatch_opens_the_picker_and_ok_sets_the_path_color() {
    let mut h = harness(Tool::Pen);
    // The gear is named by the framework's words; the popover by the options bar.
    h.get_by_label("Path Options").click();
    h.run_steps(4);
    assert!(h.query_by_label("Thickness").is_some() && h.query_by_label("Rubber Band").is_some(), "the gear opens the popover");
    h.get_all_by_role_and_label(egui::accesskit::Role::ColorWell, "Color").next().expect("the Color swatch").click();
    h.run_steps(4);
    let picker = top(h.state()).expect("the swatch opens the Color Picker");
    assert!(matches!(target(h.state(), picker), Some(PickTarget::PathColor)));
    testing::set_color(h.state_mut(), picker, "#123456");
    crate::dialogs::testing::confirm(h.state_mut(), picker).unwrap();
    assert_eq!(h.state().ui.view.path_options.color, Some([0x12, 0x34, 0x56]));
    assert_eq!(super::path_options(h.state()).color, Some([0x12, 0x34, 0x56]), "what the paths are drawn with");
}

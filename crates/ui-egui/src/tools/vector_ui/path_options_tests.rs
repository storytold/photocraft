//! Path Options (#9713): the options-bar gear of the Pen and shape tools sets the colour and
//! thickness paths are drawn in. It is one display setting shared by every path tool, as in
//! Photoshop, not a tool option; the Pen's gear also holds its own Rubber Band check item.

use egui::Color32;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::add_anchor_options_bar_tests::harness;
use super::{PathLook, options_bar};
use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};

const ACCENT: Color32 = Color32::from_rgb(10, 20, 30);
const PEN_GEAR: &str = "Set additional pen and path options";
const GEAR: &str = "Path Options";

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

#[test]
fn default_path_options_draw_the_usual_look() {
    assert_eq!(PathLook::of(&app(), ACCENT), PathLook::plain(ACCENT), "1 px accent outline");
}

#[test]
fn path_options_set_the_colour_and_thickness() {
    let mut app = app();
    app.ui.view.path_options.color = "red".into();
    app.ui.view.path_options.thickness = 2.5;
    assert_eq!(PathLook::of(&app, ACCENT), PathLook { color: Color32::from_rgb(0xff, 0x2a, 0x2a), width: 2.5 });
}

#[test]
fn hostile_path_options_fall_back_to_valid_ones() {
    let mut app = app();
    for (thickness, width) in [(f32::NAN, 1.0), (f32::INFINITY, 1.0), (-4.0, 0.5), (99.0, 3.0)] {
        app.ui.view.path_options.thickness = thickness;
        assert_eq!(PathLook::of(&app, ACCENT).width, width, "thickness {thickness}");
    }
    app.ui.view.path_options.color = "no such colour".into();
    assert_eq!(PathLook::of(&app, ACCENT).color, ACCENT);
}

/// One setting for every path tool: not among any tool's own options, so switching tools, Reset
/// Tool and Reset All Tools leave it as it is.
#[test]
fn path_options_are_one_display_setting_not_a_tool_option() {
    let saved = serde_json::to_value(ToolOptions::default()).unwrap();
    for key in ["path_color", "path_thickness", "anchor_size", "path_mark_start"] {
        assert!(saved.get(key).is_none(), "{key} is not saved with the tool options");
    }
    for tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen, Tool::Line, Tool::Rectangle, Tool::CustomShape] {
        let keys = crate::tool_reset::option_keys(tool);
        assert!(!keys.iter().any(|k| k.starts_with("path_")), "{tool:?}: {keys:?}");
    }
    // Tool options saved by an earlier build of this change still load.
    let mut old = saved.clone();
    old["path_color"] = json!("red");
    old["anchor_size"] = json!(7);
    assert_eq!(serde_json::from_value::<ToolOptions>(old).unwrap(), ToolOptions::default());

    let mut app = app();
    let ctx = egui::Context::default();
    app.ui.view.path_options.color = "green".into();
    app.ui.view.path_options.thickness = 2.0;
    let look = PathLook::of(&app, ACCENT);
    for tool in [Tool::Pen, Tool::Rectangle, Tool::FreeformPen] {
        app.ui.tool = tool;
        crate::menus::invoke(&mut app, &ctx, "tool.reset", json!({})).unwrap();
        assert_eq!(PathLook::of(&app, ACCENT), look, "{tool:?}");
    }
    crate::tool_reset::confirm(&mut app).unwrap();
    assert_eq!(PathLook::of(&app, ACCENT), look, "Reset All Tools");
}

#[test]
fn pen_and_shape_options_bars_have_the_path_options_gear() {
    for tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen, Tool::Rectangle] {
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 60.0)).build_ui_state(
            move |ui, app: &mut PhotocraftApp| {
                ui.horizontal(|ui| options_bar(app, ui, tool));
            },
            app(),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(2);
        // The Pen's gear also holds Rubber Band and carries Photoshop's name for it (#12900).
        let gear = if tool == Tool::Pen { PEN_GEAR } else { GEAR };
        assert!(h.query_by_label(gear).is_some(), "{tool:?}");
    }
}

/// The popover holds Thickness and Color; the Pen's adds Rubber Band under them. No Anchor Size
/// and no Highlight Start Point: Photoshop's Path Options has neither.
#[test]
fn the_popover_holds_thickness_color_and_the_pens_rubber_band() {
    for (tool, gear) in [(Tool::Pen, PEN_GEAR), (Tool::Rectangle, GEAR), (Tool::FreeformPen, GEAR)] {
        let mut h = harness(tool);
        h.get_by_label(gear).click();
        h.run_steps(4);
        // Each row is a caption and its field, both named after the caption.
        let row = |name: &str| h.get_all_by_label(name).map(|n| n.rect()).max_by(|a, b| a.left().total_cmp(&b.left())).unwrap();
        let (thickness, color) = (row("Thickness:"), row("Color:"));
        assert!(thickness.top() < color.top(), "{tool:?}: Thickness, then Color");
        for gone in ["Anchor Size:", "Highlight Start Point"] {
            assert!(h.query_by_label(gone).is_none(), "{tool:?}: {gone}");
        }
        match h.query_by_label("Rubber Band") {
            Some(rb) => {
                assert_eq!(tool, Tool::Pen, "only the Pen's gear has Rubber Band");
                assert!(rb.rect().top() > color.bottom(), "Rubber Band sits under Thickness and Color");
                assert!(!h.state().ui.tool_options.pen_rubber_band, "off by default");
            }
            None => assert_ne!(tool, Tool::Pen, "the Pen's gear has Rubber Band"),
        }
    }
}

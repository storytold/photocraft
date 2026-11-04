//! Path Options (#9713): the options-bar gear of the Pen and shape tools sets how paths, anchors
//! and handles are drawn, and can ring where each open path begins.

use egui::{Color32, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::vector::{Path, Subpath};

use super::overlay::draw_start_marks;
use super::{PathLook, options_bar};
use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};

const ACCENT: Color32 = Color32::from_rgb(10, 20, 30);

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

#[test]
fn default_path_options_draw_the_usual_look() {
    assert_eq!(PathLook::of(&app(), ACCENT), PathLook::plain(ACCENT), "1 px accent outline, 6 px anchors, no start ring");
}

#[test]
fn path_options_set_the_colour_thickness_and_anchor_size() {
    let mut app = app();
    let o = &mut app.ui.tool_options;
    o.path_color = "red".into();
    o.path_thickness = 2.5;
    o.anchor_size = 7;
    o.path_mark_start = true;
    let look = PathLook::of(&app, ACCENT);
    assert_eq!(look.color, Color32::from_rgb(0xff, 0x2a, 0x2a));
    assert_eq!(look.width, 2.5);
    assert_eq!(look.anchor, 10.0, "size 7 is four steps of 1 px over the 6 px default");
    assert!(look.mark_start);
    app.ui.tool_options.anchor_size = 1;
    assert_eq!(PathLook::of(&app, ACCENT).anchor, 4.0);
}

#[test]
fn hostile_path_options_fall_back_to_valid_ones() {
    let mut app = app();
    for (thickness, width) in [(f32::NAN, 1.0), (f32::INFINITY, 1.0), (-4.0, 0.5), (99.0, 3.0)] {
        app.ui.tool_options.path_thickness = thickness;
        assert_eq!(PathLook::of(&app, ACCENT).width, width, "thickness {thickness}");
    }
    app.ui.tool_options.anchor_size = 0;
    assert_eq!(PathLook::of(&app, ACCENT).anchor, 4.0);
    app.ui.tool_options.anchor_size = u32::MAX;
    assert_eq!(PathLook::of(&app, ACCENT).anchor, 10.0);
    app.ui.tool_options.path_color = "no such colour".into();
    assert_eq!(PathLook::of(&app, ACCENT).color, ACCENT);
}

#[test]
fn settings_saved_before_path_options_load_with_the_defaults() {
    let mut saved = serde_json::to_value(ToolOptions::default()).unwrap();
    let o = saved.as_object_mut().unwrap();
    for key in ["path_color", "path_thickness", "anchor_size", "path_mark_start"] {
        assert!(o.remove(key).is_some(), "{key} is saved");
    }
    let loaded: ToolOptions = serde_json::from_value(saved).unwrap();
    assert_eq!(loaded, ToolOptions::default());
}

/// The centres of the rings `draw_start_marks` paints for `path` (document = screen px here).
fn start_rings(path: &Path, look: &PathLook) -> Vec<egui::Pos2> {
    let ctx = egui::Context::default();
    let out = ctx.run_ui(egui::RawInput::default(), |ui| draw_start_marks(ui.painter(), path, &|q| pos2(q[0] as f32, q[1] as f32), look));
    let rings = out
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Circle(c) => Some(c.center),
            _ => None,
        })
        .collect();
    out.drop_without_applying_deltas();
    rings
}

#[test]
fn highlight_start_point_rings_the_first_anchor_of_open_subpaths_only() {
    let path = Path::new(vec![
        Subpath::polyline(&[(10.0, 10.0), (50.0, 10.0), (50.0, 40.0)]),
        Subpath::polygon(&[(60.0, 60.0), (90.0, 60.0), (90.0, 90.0)]),
        Subpath::polyline(&[(5.0, 80.0), (30.0, 95.0)]),
        // A lone anchor has no end to tell its start from.
        Subpath::polyline(&[(70.0, 5.0)]),
    ]);
    let on = PathLook { mark_start: true, ..PathLook::plain(ACCENT) };
    assert_eq!(
        start_rings(&path, &on),
        [pos2(10.0, 10.0), pos2(5.0, 80.0)],
        "the first anchors of the two open subpaths, not the closed one or the lone anchor"
    );
    assert!(start_rings(&path, &PathLook::plain(ACCENT)).is_empty(), "off by default");
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
        assert!(h.query_by_label("Path Options").is_some(), "{tool:?}");
    }
}

//! #13341: a right-click on the canvas with the Rectangle tool opens the shape and path menu, as
//! measured in the reference application: in Shape mode over a pixel layer and on empty canvas
//! over a rectangle shape layer, and in Path mode over a pixel layer with a Work Path. A right-click
//! on a shape, on a corner radius widget or in Pixels mode opens nothing, as before.
//!
//! The checks shared with the Triangle tool (#13367), which measured the same three menus, live
//! here and take the tool; `triangle_menu_tests` runs them for the Triangle.

use craft::draw::geometry::ShapeGeometry;
use craft::draw::ui::VectorMode;
use craft::keymap::Mods;
use egui::Modifiers;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::line_menu_tests::{greyed, menu, shown};
use super::tests::{any_dialog, app, harness, open_here, pick, right_click, screen_of};
use super::{available, open_at};
use crate::PhotocraftApp;
use crate::state::Tool;

/// The measured menu over a pixel layer with no path, in the issue's notation.
const PIXEL_MENU: &str = "Define Custom Shape… ― Blending Options… ― Make Selection… · New Guides From Shape · Fill Path… · Stroke Path… ― \
    Rasterize Layer · Rasterize Layer Style · Rasterize Vector Mask ― Free Transform ― Unite Shapes · Subtract Front Shape · \
    Unite Shapes at Overlap · Subtract Shapes at Overlap ― Copy Fill · Copy Complete Stroke ― Paste Fill · Paste Complete Stroke ― \
    Isolate Layers";

/// The measured menu over a shape layer, and in Path mode over a Work Path: the transform row
/// reads Free Transform Path.
const PATH_MENU: &str = "Define Custom Shape… ― Blending Options… ― Make Selection… · New Guides From Shape · Fill Path… · Stroke Path… ― \
    Rasterize Layer · Rasterize Layer Style · Rasterize Vector Mask ― Free Transform Path ― Unite Shapes · Subtract Front Shape · \
    Unite Shapes at Overlap · Subtract Shapes at Overlap ― Copy Fill · Copy Complete Stroke ― Paste Fill · Paste Complete Stroke ― \
    Isolate Layers";

/// The four Unite rows, greyed in every measured menu.
const UNITE: [&str; 4] = ["Unite Shapes", "Subtract Front Shape", "Unite Shapes at Overlap", "Subtract Shapes at Overlap"];

/// A point of the 200 × 200 document off every shape the fixtures draw, clear of the shape's
/// bounding-box handles.
pub(super) const EMPTY: [f32; 2] = [10.0, 10.0];

/// A point inside every fixture's shape.
pub(super) const INSIDE: [f32; 2] = [80.0, 90.0];

/// A 200 × 200 document.
fn document() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
}

/// `app` with `tool` picked in `mode`.
pub(super) fn with_tool(mut app: Box<PhotocraftApp>, tool: Tool, mode: VectorMode) -> Box<PhotocraftApp> {
    app.ui.tool = tool;
    app.ui.tool_options.shape_mode = mode;
    app
}

/// The geometry of the shape `tool` draws from (40, 40) to (120, 120).
fn geometry(tool: Tool) -> Value {
    json!({"kind": if tool == Tool::Triangle { "triangle" } else { "rect" }, "rect": [40.0, 40.0, 80.0, 80.0]})
}

/// A pixel layer filled red above the Background, active, with `tool` in Shape mode and no path.
pub(super) fn pixel_layer(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = document();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    with_tool(app, tool, VectorMode::Shape)
}

/// `tool`'s shape from (40, 40) to (120, 120) with a 2 px stroke, filled when `filled`, as a drag
/// in Shape mode makes it: its shape layer is active and selected.
fn shape_layer_filled(tool: Tool, filled: bool) -> Box<PhotocraftApp> {
    let mut app = with_tool(document(), tool, VectorMode::Shape);
    app.ui.tool_options.shape_fill = filled;
    app.ui.tool_options.stroke_width = 2.0;
    let rect = craft::behavior::kurbo::Rect::new(40.0, 40.0, 120.0, 120.0);
    let shape = if tool == Tool::Triangle { ShapeGeometry::Triangle { rect, corner_radius: 0.0 } } else { ShapeGeometry::Rect { rect, radii: [0.0; 4] } };
    crate::vector_ui::create_geometry(&mut app, &shape, crate::vector_ui::kind_of(tool), None).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().content.kind_name(), "Shape", "{tool:?} made a shape layer");
    app
}

/// [`shape_layer_filled`], filled.
pub(super) fn shape_layer(tool: Tool) -> Box<PhotocraftApp> {
    shape_layer_filled(tool, true)
}

/// [`pixel_layer`], then `tool` drew the same shape in Path mode: the Work Path holds it, and the
/// pixel layer stays active.
pub(super) fn work_path(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = with_tool(pixel_layer(tool), tool, VectorMode::Path);
    let outline = photocraft_engine::vector_cmds::shape_path(&geometry(tool)).unwrap();
    app.run("path.set", json!({"name": "work", "path": photocraft_engine::vector_cmds::path_json(&outline)})).unwrap();
    app.ui.selected_path = Some("work".into());
    let st = app.session.active().unwrap();
    assert!(st.doc.work_path.is_some(), "{tool:?} drew the Work Path");
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().content.kind_name(), "Pixel", "the pixel layer stays active");
    assert_eq!(crate::vector_ui::active_path_name(&app).as_deref(), Some("work"));
    app
}

/// Right-click document point `doc`: what `ui.inspect` reports.
fn inspect_at(h: &mut Harness<'static, Box<PhotocraftApp>>, doc: [f32; 2]) -> Value {
    let p = screen_of(h.state(), doc);
    right_click(h, p, Modifiers::NONE);
    crate::control::inspect(h.state(), &h.ctx)
}

/// Right-click document point `doc` with `tool`: the open menu's entries.
pub(super) fn entries_at(h: &mut Harness<'static, Box<PhotocraftApp>>, tool: Tool, doc: [f32; 2]) -> Vec<Value> {
    let inspected = inspect_at(h, doc);
    assert_eq!(inspected["canvasToolMenu"]["tool"], json!(format!("{tool:?}")), "a menu opened at {doc:?}");
    inspected["canvasToolMenu"]["entries"].as_array().cloned().unwrap_or_default()
}

/// A right-click at document point `doc` opens no menu, neither the tool's nor the layer menu.
pub(super) fn opens_nothing(h: &mut Harness<'static, Box<PhotocraftApp>>, doc: [f32; 2], case: &str) {
    let inspected = inspect_at(h, doc);
    assert_eq!(inspected["canvasToolMenu"], Value::Null, "{case}");
    assert_eq!(inspected["layerMenu"], Value::Null, "{case}");
}

/// Click the open menu's row `label`, which closes the menu.
fn pick_row(h: &mut Harness<'static, Box<PhotocraftApp>>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(2);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "{label} closes the menu");
}

/// Copy Fill and Paste Fill over a pixel layer follow the rule every shape tool's menu shares,
/// read from the Line tool's menu: greyed on `main`, live once #13705 lands for all of them. The
/// other rows are pinned to the measurement as they are.
fn pixel_fill_rows_greyed() -> bool {
    let mut app = pixel_layer(Tool::Line);
    assert!(open_here(&mut app, Tool::Line, [10.0, 10.0]));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    let copy = available(&app, &menu, "path.style.copyFill");
    assert_eq!(copy, available(&app, &menu, "path.style.pasteFill"), "the two fill rows share one rule");
    !copy
}

/// The labels of `spec` that are in `greyed`, in menu order, plus Copy Fill and Paste Fill when
/// `pixel` and [`pixel_fill_rows_greyed`].
fn greyed_rows(spec: &str, greyed: &[&str], pixel: bool) -> Vec<String> {
    let fill = pixel && pixel_fill_rows_greyed();
    menu(spec)
        .into_iter()
        .flatten()
        .filter(|label| greyed.contains(&label.as_str()) || (fill && matches!(label.as_str(), "Copy Fill" | "Paste Fill")))
        .collect()
}

/// Shape mode, a pixel layer active, no path: the measured rows, and Free Transform runs the menu
/// bar's on the layer.
pub(super) fn check_pixel_layer_menu(tool: Tool) {
    let mut h = harness(pixel_layer(tool));
    let entries = entries_at(&mut h, tool, INSIDE);
    assert_eq!(shown(&entries), menu(PIXEL_MENU), "{tool:?}");
    let measured = [
        &["Define Custom Shape…", "Make Selection…", "Fill Path…", "Stroke Path…"][..],
        &["Rasterize Layer", "Rasterize Layer Style", "Rasterize Vector Mask"],
        &UNITE,
        &["Copy Complete Stroke", "Paste Complete Stroke"],
    ]
    .concat();
    assert_eq!(greyed(&entries), greyed_rows(PIXEL_MENU, &measured, true), "{tool:?}");
    pick_row(&mut h, "Free Transform");
    assert!(h.state().ui.transform.is_some(), "{tool:?}: Free Transform opens the box on the layer");
}

/// Shape mode, the tool's shape layer active, right-click at `at`: the measured rows. Paste Fill
/// is live once a fill is copied, as in the measured session; Copy Fill copies the layer's.
pub(super) fn check_shape_layer_menu(tool: Tool, at: [f32; 2]) {
    let mut h = harness(shape_layer(tool));
    let entries = entries_at(&mut h, tool, at);
    assert_eq!(shown(&entries), menu(PATH_MENU), "{tool:?}");
    let before = h.state().session.journal.len();
    pick_row(&mut h, "Copy Fill");
    let ran: Vec<_> = h.state().session.journal[before..].iter().map(|(id, _)| id.clone()).collect();
    assert_eq!(ran, ["path.style.copyFill"], "{tool:?}");
    let st = h.state().session.active().unwrap();
    let fill = match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        photocraft_doc::LayerContent::Shape(sh) => sh.fill.clone(),
        _ => None,
    };
    assert!(fill.is_some() && h.state().session.path_fill_clipboard == fill, "{tool:?}: the layer's fill is copied");
    let entries = entries_at(&mut h, tool, at);
    let measured = [&["Fill Path…", "Stroke Path…", "Rasterize Layer Style", "Rasterize Vector Mask"][..], &UNITE, &["Paste Complete Stroke"]].concat();
    assert_eq!(greyed(&entries), greyed_rows(PATH_MENU, &measured, false), "{tool:?}");
}

/// Path mode, a Work Path holding the tool's shape, the pixel layer active, right-click at `at`:
/// the measured rows, with Free Transform Path.
pub(super) fn check_work_path_menu(tool: Tool, at: [f32; 2]) {
    let mut h = harness(work_path(tool));
    let entries = entries_at(&mut h, tool, at);
    assert_eq!(shown(&entries), menu(PATH_MENU), "{tool:?}");
    let measured =
        [&["Rasterize Layer", "Rasterize Layer Style", "Rasterize Vector Mask"][..], &UNITE, &["Copy Complete Stroke", "Paste Complete Stroke"]].concat();
    assert_eq!(greyed(&entries), greyed_rows(PATH_MENU, &measured, true), "{tool:?}");
}

/// What choosing a row did: the dialog it opened, the commands it recorded, whether a fill was
/// copied, and how many layers are isolated.
type Outcome = (bool, Vec<String>, bool, usize);

/// A document builder for a tool, and the menu rows to choose on it.
type Case = (fn(Tool) -> Box<PhotocraftApp>, &'static [&'static str]);

/// Choose `command` from `menu_tool`'s menu on the document `make(tool)` builds.
fn outcome(make: fn(Tool) -> Box<PhotocraftApp>, menu_tool: Tool, tool: Tool, command: &str) -> Outcome {
    let mut app = make(tool);
    app.ui.tool = menu_tool;
    assert!(open_here(&mut app, menu_tool, [2.0, 2.0]), "{menu_tool:?}");
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(available(&app, &menu, command), "{menu_tool:?}: {command} is enabled");
    let before = app.session.journal.len();
    pick(&mut app, command);
    assert!(!app.ui.status_error, "{menu_tool:?}: {command}: {}", app.ui.status);
    let ran = app.session.journal[before..].iter().map(|(id, _)| id.clone()).collect();
    let isolated = app.session.active().map_or(0, |st| st.isolated_layers.len());
    (any_dialog(&app), ran, app.session.path_fill_clipboard.is_some(), isolated)
}

/// Each enabled path row opens the dialog or runs the command the Pen's menu does on the same
/// document, over the shape layer and over the Work Path; the layer rows run the menu bar's.
pub(super) fn check_rows_run_as_elsewhere(tool: Tool) {
    let cases: [Case; 2] = [
        (shape_layer, &["edit.defineCustomShape", "path.toSelection", "path.transform", "path.style.copyFill", "select.isolateLayers"]),
        (work_path, &["edit.defineCustomShape", "path.toSelection", "path.fill", "path.stroke", "path.transform", "select.isolateLayers"]),
    ];
    for (make, commands) in cases {
        for command in commands {
            let pen = outcome(make, Tool::Pen, tool, command);
            assert!(pen.0 || !pen.1.is_empty() || pen.2 || pen.3 > 0, "{command} does something from the Pen's menu");
            assert_eq!(outcome(make, tool, tool, command), pen, "{tool:?}: {command}");
        }
    }
    let mut app = shape_layer(tool);
    assert!(open_here(&mut app, tool, [2.0, 2.0]));
    pick(&mut app, "layer.rasterize.layer");
    assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("layer.rasterize.layer"), "{tool:?}");
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().content.kind_name(), "Pixel", "{tool:?}: rasterized");
}

/// Pixels mode, Path mode with no Work Path or with a shape layer active, and a type layer: no
/// menu, as before.
pub(super) fn check_unmeasured_cases_open_nothing(tool: Tool) {
    for make in [pixel_layer, shape_layer, work_path] {
        let mut h = harness(with_tool(make(tool), tool, VectorMode::Pixels));
        opens_nothing(&mut h, EMPTY, &format!("{tool:?}: Pixels mode"));
    }
    let mut h = harness(with_tool(pixel_layer(tool), tool, VectorMode::Path));
    opens_nothing(&mut h, EMPTY, &format!("{tool:?}: Path mode, no path"));
    let mut h = harness(with_tool(shape_layer(tool), tool, VectorMode::Path));
    opens_nothing(&mut h, EMPTY, &format!("{tool:?}: Path mode, a shape layer active"));
    let mut app = app();
    app.run("type.create", json!({"x": 4, "y": 20, "text": "A", "size": 12})).unwrap();
    let mut app = with_tool(app, tool, VectorMode::Shape);
    assert!(!open_here(&mut app, tool, [2.0, 2.0]), "{tool:?}: a type layer");
    assert!(app.ui.canvas_tool_menu.is_none());
}

#[test]
fn on_a_pixel_layer_the_menu_lists_the_reference_rows() {
    check_pixel_layer_menu(Tool::Rectangle);
}

#[test]
fn on_empty_canvas_over_a_rectangle_layer_the_menu_lists_the_reference_rows() {
    check_shape_layer_menu(Tool::Rectangle, EMPTY);
}

#[test]
fn in_path_mode_over_a_work_path_the_menu_lists_the_reference_rows() {
    // Inside the rectangle too: in Path mode it is part of the Work Path, not a shape layer.
    check_work_path_menu(Tool::Rectangle, EMPTY);
    check_work_path_menu(Tool::Rectangle, INSIDE);
}

#[test]
fn the_rows_run_the_pen_menu_and_menu_bar_commands() {
    check_rows_run_as_elsewhere(Tool::Rectangle);
}

#[test]
fn pixels_mode_and_unmeasured_cases_open_nothing() {
    check_unmeasured_cases_open_nothing(Tool::Rectangle);
}

#[test]
fn inside_the_rectangle_opens_nothing() {
    let mut h = harness(shape_layer(Tool::Rectangle));
    opens_nothing(&mut h, INSIDE, "inside the rectangle");
    // On empty canvas the same document opens the menu.
    entries_at(&mut h, Tool::Rectangle, EMPTY);
}

#[test]
fn on_a_corner_radius_widget_opens_nothing() {
    // No fill, so a point inside the top-left corner, where its widget sits, is off the shape as
    // Path Selection picks it: only the widget keeps the right-click.
    let mut h = harness(shape_layer_filled(Tool::Rectangle, false));
    let widget = [50.0, 50.0];
    assert!(!crate::shape_controls::corner_targets(h.state()).is_empty(), "the rectangle shows its corner widgets");
    let st = h.state().session.active().unwrap();
    assert!(crate::vector_ui::pick_shape_at(&st.doc, widget, crate::path_components::press_tolerance(h.state())).is_none());
    opens_nothing(&mut h, [widget[0] as f32, widget[1] as f32], "on the widget");
    // Also through the control channel's path, which calls `open_at` directly.
    let at = screen_of(h.state(), [widget[0] as f32, widget[1] as f32]);
    assert!(!open_at(h.state_mut(), Tool::Rectangle, Mods::NONE, at, widget, None));
    entries_at(&mut h, Tool::Rectangle, EMPTY);
}

//! #16182: Stroke Path paints with the brush of the tool chosen in its dialog (its size, hardness,
//! tip, dynamics), not with the live brush, which is the last brush tool's. A tool not used yet
//! paints with the brush it starts with. The live brush is left as it was.

use craft::draw::ui::paths::{PathsEvent, PathsFooterButton};
use craft::keymap::Mods;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use serde_json::json;

use super::paths_panel::apply;
use crate::PhotocraftApp;
use crate::canvas_tool_menu::pen_dialogs;
use crate::canvas_tool_menu::tests::{confirm_hosted, hosted};
use crate::direct_select::PathRef;
use crate::state::Tool;

/// A 200×200 document with a new empty layer and a straight open Work Path across it at y = 100.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[40, 100], [160, 100]]}]}})).unwrap();
    app
}

/// Picks `tool`, as a frame after the click does, and sets its brush size when given.
fn pick(app: &mut PhotocraftApp, tool: Tool, size: Option<f64>) {
    app.ui.tool = tool;
    crate::paint_mouse::sync_tool_brush(app);
    if let Some(size) = size {
        app.run("tools.setBrush", json!({"brush": {"size": size}})).unwrap();
    }
}

/// Stroke Path… on the Work Path with the dialog's Tool set to `tool`, then OK.
fn stroke_path(app: &mut PhotocraftApp, tool: &str) {
    let ctx = app.ctx().clone();
    pen_dialogs::open(app, "path.stroke", "work");
    assert!(app.framework.dialogs.is_open::<HostedParamDialog>(), "Stroke Path… opens its dialog");
    hosted(app).set_field("tool", tool.into());
    confirm_hosted(app, &ctx);
}

/// How many rows of the active layer the stroke painted.
fn painted_rows(app: &PhotocraftApp) -> usize {
    let st = app.session.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    (0..200).filter(|&y| (0..200).any(|x| surf.rgba(x, y)[3] > 0.0)).count()
}

fn undo(app: &mut PhotocraftApp) {
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(painted_rows(app), 0, "Undo took the stroke away");
}

/// The issue's steps: a 3 px Brush, then an 80 px Eraser, then the Pen; Stroke Path with Tool: Brush
/// paints a 3 px line, as it does after the Brush was picked again. The Eraser keeps its 80 px.
#[test]
fn stroke_path_with_the_brush_paints_with_the_brushs_own_size() {
    let mut app = app();
    pick(&mut app, Tool::Brush, Some(3.0));
    pick(&mut app, Tool::Eraser, Some(80.0));
    pick(&mut app, Tool::Pen, None);
    stroke_path(&mut app, "brush");
    let first = painted_rows(&app);
    assert!((1..=6).contains(&first), "a 3 px Brush line, not the Eraser's 80 px: {first} rows");
    assert_eq!(app.session.tools.brush.size, 80.0, "the live brush is the Eraser's again");
    undo(&mut app);
    pick(&mut app, Tool::Brush, None);
    pick(&mut app, Tool::Pen, None);
    stroke_path(&mut app, "brush");
    assert_eq!(painted_rows(&app), first, "the same line with the Brush picked last");
}

/// The same with the Pencil, which also keeps its own brush; a Pencil not used yet strokes with
/// the 1 px hard round it starts with (#12200), not with the Brush's 40 px.
#[test]
fn stroke_path_with_the_pencil_paints_with_the_pencils_own_brush() {
    let mut app = app();
    pick(&mut app, Tool::Brush, Some(40.0));
    pick(&mut app, Tool::Pen, None);
    stroke_path(&mut app, "pencil");
    let fresh = painted_rows(&app);
    assert!((1..=2).contains(&fresh), "the Pencil's 1 px start, not the Brush's 40 px: {fresh} rows");
    undo(&mut app);
    pick(&mut app, Tool::Pencil, Some(5.0));
    pick(&mut app, Tool::Brush, None);
    pick(&mut app, Tool::Pen, None);
    stroke_path(&mut app, "pencil");
    let rows = painted_rows(&app);
    assert!((4..=9).contains(&rows), "the Pencil's 5 px, not the Brush's 40 px: {rows} rows");
    assert_eq!(app.session.tools.brush.size, 40.0, "the live brush is the Brush's again");
}

/// The Paths panel's Stroke Path button strokes with the Brush's own brush too.
#[test]
fn paths_panel_stroke_button_paints_with_the_brushs_own_size() {
    let mut app = app();
    pick(&mut app, Tool::Brush, Some(3.0));
    pick(&mut app, Tool::Eraser, Some(80.0));
    pick(&mut app, Tool::Pen, None);
    apply(&mut app, PathsEvent::footer(PathsFooterButton::Stroke, vec![PathRef::Work], Mods::NONE));
    let rows = painted_rows(&app);
    assert!((1..=6).contains(&rows), "a 3 px Brush line, not the Eraser's 80 px: {rows} rows");
    assert_eq!(app.session.tools.brush.size, 80.0, "the live brush is the Eraser's again");
}

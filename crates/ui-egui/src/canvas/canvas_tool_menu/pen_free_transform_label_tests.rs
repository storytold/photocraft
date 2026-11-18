//! #12913: the Pen's canvas menu names its transform row after what it would act on: Free
//! Transform (greyed) with no path, Free Transform Points while drawing, Free Transform Path on a
//! finished path. A path that exists but isn't active keeps Free Transform Path.

use egui::Modifiers;
use egui_kittest::kittest::{NodeT, Queryable};

use super::tests::{app, choose_open, confirm_hosted, harness, pen_clicks, right_click};
use super::{PEN_MENU, rows};
use crate::PhotocraftApp;
use crate::state::Tool;

const LABELS: [&str; 3] = ["Free Transform", "Free Transform Points", "Free Transform Path"];

/// Right-click the middle of the canvas with `tool`: the one transform row label the path menu
/// shows, and whether that row is enabled. `ui.inspect` lists the row the same way.
fn transform_row(mut app: PhotocraftApp, tool: Tool) -> (&'static str, bool) {
    app.ui.tool = tool;
    let mut h = harness(app);
    let center = h.state().last_canvas_rect.center();
    right_click(&mut h, center, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.tool == tool && rows(m) == PEN_MENU), "{tool:?}: the path menu is open");
    let shown: Vec<_> = LABELS.into_iter().filter(|label| h.query_by_label(label).is_some()).collect();
    assert_eq!(shown.len(), 1, "{tool:?}: one transform row, got {shown:?}");
    let enabled = !h.get_by_label(shown[0]).accesskit_node().is_disabled();
    let inspected = crate::control::inspect(h.state(), &h.ctx);
    let entries = inspected["canvasToolMenu"]["entries"].as_array().cloned().unwrap_or_default();
    let entry = entries.iter().find(|e| e["id"] == "path.transform");
    assert_eq!(entry.map(|e| (&e["label"], &e["enabled"])), Some((&serde_json::json!(shown[0]), &serde_json::json!(enabled))), "{tool:?}: ui.inspect");
    (shown[0], enabled)
}

#[test]
fn while_drawing_the_pen_row_reads_free_transform_points() {
    let mut app = app();
    pen_clicks(&mut app, &[(2.0, 2.0), (20.0, 2.0), (20.0, 20.0)]);
    assert!(app.ui.pen.as_ref().is_some_and(|p| p.draft.knots.len() >= 2), "the path is still being drawn");
    assert_eq!(transform_row(app, Tool::Pen), ("Free Transform Points", true));
}

/// Make Selection hides the Work Path (#6079): it stays in the document but isn't active, so the
/// row keeps today's label, greyed.
#[test]
fn an_inactive_path_keeps_free_transform_path() {
    let mut app = app();
    pen_clicks(&mut app, &[(2.0, 2.0), (28.0, 2.0), (28.0, 28.0), (2.0, 2.0)]);
    choose_open(&mut app, "path.toSelection");
    let ctx = app.ctx().clone();
    confirm_hosted(&mut app, &ctx);
    assert!(app.session.active().unwrap().doc.work_path.is_some());
    assert!(!crate::vector_ui::work_path_shown(&app), "the Work Path is not active");
    assert_eq!(transform_row(app, Tool::Pen), ("Free Transform Path", false));
}

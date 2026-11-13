//! #12710: with the Add Anchor Point tool a right-click on the canvas opens the Pen's path menu
//! with the entry for the point under the pointer on top, as for Delete Anchor Point (#12771).
//! A right-click on an anchor selects it, and with anchors selected the Fill, Stroke and transform
//! rows name what they act on: Fill / Stroke Subpath… (one subpath), Subpaths… (several), Free
//! Transform Points. The shared steps live in `delete_anchor_menu_tests`.

use egui::Modifiers;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::delete_anchor_menu_tests::{assert_top_entry, empty_canvas, right_clicked, saved, setup, steps};
use super::tests::{harness, right_click, screen_of};
use super::{PEN_MENU, rows};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

const TOOL: Tool = Tool::AddAnchorPoint;

/// The Add Anchor Point tool with Ctrl held (its temporary Direct Selection) clicks `anchors`: the
/// first with Ctrl, the rest with Ctrl+Shift.
fn with_selected(anchors: &[[f64; 2]]) -> PhotocraftApp {
    let mut app = setup(TOOL);
    for (i, [x, y]) in anchors.iter().copied().enumerate() {
        let mods = if i == 0 { Modifiers::COMMAND } else { Modifiers::COMMAND | Modifiers::SHIFT };
        tool_event(&mut app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
        tool_event(&mut app, ToolEvent::Up { x, y }, mods);
    }
    assert_eq!(app.ui.direct_selection.anchors.len(), anchors.len(), "the anchors are selected");
    app
}

fn right_click_at(app: PhotocraftApp, at: [f32; 2]) -> Harness<'static, PhotocraftApp> {
    let mut h = harness(app);
    let p = screen_of(h.state(), at);
    right_click(&mut h, p, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.is_some(), "the menu opened at {at:?}");
    h
}

/// The Fill, Stroke and transform rows of the open menu read `labels`, and none of the others.
fn assert_rows(h: &Harness<'static, PhotocraftApp>, labels: [&str; 3]) {
    const ALL: [&str; 11] = [
        "Fill Path…",
        "Fill Subpath…",
        "Fill Subpaths…",
        "Stroke Path…",
        "Stroke Subpath…",
        "Stroke Subpaths…",
        "Free Transform Path",
        "Free Transform Points",
        "Free Transform",
        "Fill Subpath(s)…",
        "Stroke Subpath(s)…",
    ];
    for label in ALL {
        assert_eq!(h.query_by_label(label).is_some(), labels.contains(&label), "{label}: expected only {labels:?}");
    }
}

/// Right-click the square's right edge at (300,400): Add Anchor Point heads the menu, then the
/// Pen's rows in their order; choosing it puts an anchor there, one step named Add Anchor Point.
#[test]
fn right_click_on_a_segment_offers_add_anchor_point_and_it_inserts_there() {
    let mut h = right_clicked(TOOL, [300.0, 400.0]);
    assert_top_entry(&h, TOOL, "Add Anchor Point");
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| rows(m) == PEN_MENU), "then PEN_MENU's rows");
    let before = steps(h.state());
    h.get_by_label("Add Anchor Point").click();
    h.run_steps(2);
    let path = saved(h.state());
    assert_eq!(path.subpaths[0].knots.len(), 5);
    assert!(path.subpaths[0].closed);
    let added = &path.subpaths[0].knots[2].anchor;
    assert!((added.x - 300.0).abs() < 1.0 && (added.y - 400.0).abs() < 3.0, "the anchor lands at (300,400): {added:?}");
    assert_eq!(steps(h.state()), before + 1);
    assert_eq!(h.state().session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
}

/// Right-click the corner (100,300): Delete Anchor Point heads the menu and the corner is selected;
/// choosing it removes the corner, the square stays closed, one step named Delete Anchor Point.
#[test]
fn right_click_on_an_anchor_selects_it_and_offers_delete_anchor_point() {
    let mut h = right_clicked(TOOL, [100.0, 300.0]);
    assert_top_entry(&h, TOOL, "Delete Anchor Point");
    assert_eq!(h.state().ui.direct_selection.anchors, vec![[0, 0]], "the right-clicked anchor is selected");
    let before = steps(h.state());
    h.get_by_label("Delete Anchor Point").click();
    h.run_steps(2);
    let path = saved(h.state());
    assert!(path.subpaths[0].closed, "the square stays closed");
    assert_eq!(path.subpaths[0].knots.len(), 3);
    assert!(path.subpaths[0].knots.iter().all(|k| (k.anchor.x, k.anchor.y) != (100.0, 300.0)), "(100,300) is gone");
    assert_eq!(steps(h.state()), before + 1);
    assert_eq!(h.state().session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
}

#[test]
fn right_click_on_empty_canvas_opens_the_path_menu_without_a_top_entry() {
    empty_canvas(TOOL);
}

/// Opening the menu on an anchor only selects it: no history step and no path change.
#[test]
fn right_click_on_an_anchor_changes_no_path() {
    let app = setup(TOOL);
    let (path, n) = (saved(&app), steps(&app));
    let h = right_click_at(app, [100.0, 300.0]);
    assert_eq!(steps(h.state()), n);
    assert_eq!(saved(h.state()), path);
}

/// Nothing selected, path targeted: the table's labels.
#[test]
fn with_no_anchor_selected_the_rows_name_the_path() {
    let h = right_clicked(TOOL, [500.0, 100.0]);
    assert_rows(&h, ["Fill Path…", "Stroke Path…", "Free Transform Path"]);
}

/// Anchors of one subpath selected: the singular labels and Free Transform Points.
#[test]
fn with_anchors_of_one_subpath_selected_the_rows_name_the_subpath() {
    let h = right_click_at(with_selected(&[[100.0, 300.0], [300.0, 300.0]]), [500.0, 100.0]);
    assert_rows(&h, ["Fill Subpath…", "Stroke Subpath…", "Free Transform Points"]);
}

/// Anchors in two subpaths selected: the plural labels.
#[test]
fn with_anchors_of_two_subpaths_selected_the_rows_name_the_subpaths() {
    let h = right_click_at(with_selected(&[[100.0, 300.0], [400.0, 300.0]]), [500.0, 100.0]);
    assert_rows(&h, ["Fill Subpaths…", "Stroke Subpaths…", "Free Transform Points"]);
}

/// The anchor a right-click selects counts too: the menu on (100,300) reads Subpath…
#[test]
fn the_right_clicked_anchor_counts_as_selected() {
    let h = right_clicked(TOOL, [100.0, 300.0]);
    assert_rows(&h, ["Fill Subpath…", "Stroke Subpath…", "Free Transform Points"]);
}

/// A right-click on an anchor already among several selected ones keeps them all.
#[test]
fn right_click_on_a_selected_anchor_keeps_the_selection() {
    let h = right_click_at(with_selected(&[[100.0, 300.0], [400.0, 300.0]]), [400.0, 300.0]);
    assert_eq!(h.state().ui.direct_selection.anchors, vec![[0, 0], [1, 0]]);
    assert_rows(&h, ["Fill Subpaths…", "Stroke Subpaths…", "Free Transform Points"]);
}

/// No path targeted: Fill Path…, Stroke Path… and Free Transform, and no top entry.
#[test]
fn with_no_path_targeted_the_transform_row_reads_free_transform() {
    let mut app = setup(TOOL);
    app.ui.selected_path = None;
    let h = right_click_at(app, [300.0, 400.0]);
    assert!(h.query_by_label("Add Anchor Point").is_none() && h.query_by_label("Delete Anchor Point").is_none(), "no top entry");
    assert_rows(&h, ["Fill Path…", "Stroke Path…", "Free Transform"]);
}

/// The labels belong to this tool: Delete Anchor Point's menu keeps the table's, also on an anchor.
#[test]
fn other_anchor_tools_keep_the_path_labels() {
    for tool in [Tool::DeleteAnchorPoint, Tool::ConvertPoint] {
        let h = right_clicked(tool, [100.0, 300.0]);
        assert_rows(&h, ["Fill Path…", "Stroke Path…", "Free Transform Path"]);
    }
}

/// A selection the control channel set on another document revision does not rename the rows.
#[test]
fn a_stale_selection_does_not_rename_the_rows() {
    let mut app = with_selected(&[[100.0, 300.0]]);
    app.run("path.addAnchor", json!({"name": "Path 1", "subpath": 1, "knot": 0, "t": 0.5})).unwrap();
    let h = right_click_at(app, [500.0, 100.0]);
    assert_rows(&h, ["Fill Path…", "Stroke Path…", "Free Transform Path"]);
}

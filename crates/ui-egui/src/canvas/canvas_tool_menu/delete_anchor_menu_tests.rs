//! #12771: with the Delete Anchor Point tool a right-click on the canvas opens the Pen's path menu
//! with one entry for what is under the pointer on top: Delete Anchor Point on an anchor, Add
//! Anchor Point on a segment, nothing on empty canvas. Opening it records no history step and
//! leaves the anchor selection alone. The helpers are shared with the Convert Point tests (#12744).

use egui::Modifiers;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::vector::Path;
use serde_json::json;

use super::tests::{harness, right_click, screen_of};
use super::{PEN_MENU, rows};
use crate::PhotocraftApp;
use crate::state::Tool;

/// 800 x 600 with a saved, targeted **Path 1**: a closed square (100,300)(300,300)(300,500)
/// (100,500), an open line (400,300)->(700,300) and an open S-curve (400,450)->(700,450).
pub(super) fn setup(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    let path = json!({"subpaths": [
        {"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]},
        {"closed": false, "knots": [[400, 300], [700, 300]]},
        {"closed": false, "knots": [
            {"anchor": [400, 450], "in": [400, 450], "out": [500, 350]},
            {"anchor": [700, 450], "in": [600, 550], "out": [700, 450]},
        ]},
    ]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    app.ui.tool = tool;
    app
}

/// A harness over [`setup`] with the right-click at document point `at` done.
pub(super) fn right_clicked(tool: Tool, at: [f32; 2]) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = harness(setup(tool));
    let p = screen_of(h.state(), at);
    right_click(&mut h, p, Modifiers::NONE);
    h
}

pub(super) fn saved(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.paths.iter().find(|p| p.name == "Path 1").unwrap().path.clone()
}

pub(super) fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

/// The path menu is open and its rows are the Pen's, in the Pen's order.
fn assert_path_menu(h: &Harness<'static, Box<PhotocraftApp>>, tool: Tool) {
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.tool == tool && rows(m) == PEN_MENU), "{tool:?}: the path menu is open");
}

/// The first entry of the open menu is `label`, above Create Vector Mask, and is the only one of
/// the two Add / Delete Anchor Point entries shown.
pub(super) fn assert_top_entry(h: &Harness<'static, Box<PhotocraftApp>>, tool: Tool, label: &str) {
    assert_path_menu(h, tool);
    let other = if label == "Delete Anchor Point" { "Add Anchor Point" } else { "Delete Anchor Point" };
    assert!(h.query_by_label(other).is_none(), "{other} is not offered");
    assert!(h.get_by_label(label).rect().top() < h.get_by_label("Create Vector Mask").rect().top(), "{label} heads the menu");
}

/// The context-menu contract after a menu edit: undo restores the whole original path and redo
/// replays the edit (the path as it was right after the menu choice, `edited`).
fn assert_undo_redo(h: &mut Harness<'static, Box<PhotocraftApp>>, original: &Path, edited: &Path) {
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(&saved(h.state()), original, "undo restores the whole path");
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(&saved(h.state()), edited, "redo replays the menu edit");
}

/// Right-click the corner (300,300), choose Delete Anchor Point: the square stays closed with
/// three anchors and the history gains one step by that name.
pub(super) fn delete_the_corner(tool: Tool) {
    let mut h = right_clicked(tool, [300.0, 300.0]);
    assert_top_entry(&h, tool, "Delete Anchor Point");
    let before = steps(h.state());
    let original = saved(h.state());
    h.get_by_label("Delete Anchor Point").click();
    h.run_steps(2);
    let path = saved(h.state());
    assert!(path.subpaths[0].closed, "the square stays closed");
    assert_eq!(path.subpaths[0].knots.len(), 3);
    assert!(path.subpaths[0].knots.iter().all(|k| (k.anchor.x, k.anchor.y) != (300.0, 300.0)), "(300,300) is gone");
    assert_eq!(steps(h.state()), before + 1);
    assert_eq!(h.state().session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
    assert_undo_redo(&mut h, &original, &path);
}

/// Right-click the top edge (200,300): Add Anchor Point heads the menu and adds one there.
pub(super) fn add_on_the_edge(tool: Tool) {
    let mut h = right_clicked(tool, [200.0, 300.0]);
    assert_top_entry(&h, tool, "Add Anchor Point");
    let before = steps(h.state());
    let original = saved(h.state());
    h.get_by_label("Add Anchor Point").click();
    h.run_steps(2);
    let path = saved(h.state());
    assert_eq!(path.subpaths[0].knots.len(), 5);
    let added = &path.subpaths[0].knots[1].anchor;
    assert!((added.x - 200.0).abs() < 3.0 && (added.y - 300.0).abs() < 3.0, "the anchor lands where it was clicked: {added:?}");
    assert_eq!(steps(h.state()), before + 1);
    assert_eq!(h.state().session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
    assert_undo_redo(&mut h, &original, &path);
}

#[test]
fn right_click_on_an_anchor_offers_delete_anchor_point_on_top_and_it_deletes() {
    delete_the_corner(Tool::DeleteAnchorPoint);
}

#[test]
fn right_click_on_a_segment_offers_add_anchor_point_on_top() {
    add_on_the_edge(Tool::DeleteAnchorPoint);
}

/// The entry is about the path as it was when the menu opened: an edit meanwhile (a control call
/// inserting an anchor before the corner) shifts the knots, so choosing it deletes nothing.
#[test]
fn a_point_entry_expires_once_the_path_is_edited() {
    let mut h = right_clicked(Tool::DeleteAnchorPoint, [300.0, 300.0]);
    assert_top_entry(&h, Tool::DeleteAnchorPoint, "Delete Anchor Point");
    h.state_mut().run("path.addAnchor", json!({"name": "Path 1", "subpath": 0, "knot": 0, "t": 0.5})).unwrap();
    let after_add = saved(h.state());
    assert_eq!(after_add.subpaths[0].knots.len(), 5, "the anchor went in");
    let n = steps(h.state());
    let ctx = h.ctx.clone();
    super::choose(h.state_mut(), &ctx, "path.deleteAnchor");
    assert_eq!(saved(h.state()), after_add, "the stale entry deleted nothing");
    assert_eq!(steps(h.state()), n, "and added no history step");
}

/// The expired entry is drawn disabled, not just rejected on click (#12771, CodeRabbit).
#[test]
fn an_expired_point_entry_is_drawn_disabled() {
    let mut h = right_clicked(Tool::DeleteAnchorPoint, [300.0, 300.0]);
    assert!(!h.get_by_label("Delete Anchor Point").accesskit_node().is_disabled(), "enabled while the path is as it was");
    h.state_mut().run("path.addAnchor", json!({"name": "Path 1", "subpath": 0, "knot": 0, "t": 0.5})).unwrap();
    h.run();
    assert!(h.get_by_label("Delete Anchor Point").accesskit_node().is_disabled(), "disabled once the path is edited");
}

/// Retained point menus must disable their ordinary rows too, not merely reject the pick.
#[test]
fn expired_point_menus_disable_ordinary_rows_after_edits_undo_and_redo() {
    for tool in [Tool::DeleteAnchorPoint, Tool::ConvertPoint] {
        let mut h = right_clicked(tool, [300.0, 300.0]);
        for (command, params) in
            [("path.addAnchor", json!({"pathIndex": 0, "subpath": 0, "knot": 0, "t": 0.5})), ("edit.undo", json!({})), ("edit.redo", json!({}))]
        {
            assert!(!h.get_by_label("Make Selection…").accesskit_node().is_disabled());
            h.state_mut().run(command, params).unwrap();
            let doc = h.state().session.active().unwrap().doc.clone();
            let n = steps(h.state());
            h.run();
            assert!(h.state().ui.canvas_tool_menu.is_some(), "the expired point menu stays visible");
            for label in ["Delete Anchor Point", "Delete Path", "Define Custom Shape…", "Make Selection…", "Fill Path…", "Stroke Path…"] {
                assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{tool:?}: {label} must be disabled after {command}");
            }
            let ctx = h.ctx.clone();
            super::choose(h.state_mut(), &ctx, "path.toSelection");
            assert!(h.state().framework.dialogs.is_empty(), "a direct stale pick cannot open a dialog");
            assert_eq!(h.state().session.active().unwrap().doc.as_ref(), doc.as_ref());
            assert_eq!(steps(h.state()), n);
            h.run_steps(2);
            let p = screen_of(h.state(), [300.0, 300.0]);
            right_click(&mut h, p, Modifiers::NONE);
        }
        // A newly opened menu uses the current source and its ordinary actions still work.
        assert!(!h.get_by_label("Make Selection…").accesskit_node().is_disabled());
        h.get_by_label("Make Selection…").click();
        h.run_steps(2);
        assert!(
            h.state()
                .framework
                .dialogs
                .get::<craft::ui::patterns::form_dialogs::HostedParamDialog>()
                .is_some_and(|d| craft::ui::dialog::StackDialog::key(d) == Some("path.toSelection"))
        );
    }
}

#[test]
#[ignore = "offscreen visual evidence; requires a wgpu adapter"]
fn expired_point_menu_visual() {
    let output = std::env::var_os("PHOTOCRAFT_PEN_VISUAL_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| "log/path-menu-visual".into());
    std::fs::create_dir_all(&output).unwrap();
    let mut h = right_clicked(Tool::DeleteAnchorPoint, [300.0, 300.0]);
    h.render().unwrap().save(output.join("point-menu-current.png")).unwrap();
    h.state_mut().run("path.addAnchor", json!({"pathIndex": 0, "subpath": 0, "knot": 0, "t": 0.5})).unwrap();
    h.run();
    h.render().unwrap().save(output.join("point-menu-expired.png")).unwrap();
    assert!(h.get_by_label("Make Selection…").accesskit_node().is_disabled());
}

/// A shape with only its own position lock is ignored by the anchor tools' click (#12730, #12825),
/// so the menu offers no point entry on its corner either: the rest of the menu is as before.
#[test]
fn a_shape_with_only_its_position_lock_gets_no_point_entry() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    let shape = app.run("shape.create", json!({"kind": "rect", "rect": [450, 100, 200, 150], "fill": "#3070c0"})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.select", json!({"layer": shape})).unwrap();
    app.run("layer.lockLayers", json!({"position": true})).unwrap();
    app.ui.tool = Tool::DeleteAnchorPoint;
    let mut h = harness(app);
    let p = screen_of(h.state(), [650.0, 250.0]);
    right_click(&mut h, p, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.is_some(), "the menu opens");
    assert!(h.query_by_label("Delete Anchor Point").is_none() && h.query_by_label("Add Anchor Point").is_none(), "no point entry on the locked shape");
}

use super::*;
use crate::canvas::document_display::active_view;
use crate::control::{ControlRequest, Reply, handle};
use photocraft_engine::warp_cmds::{Split, split_warp};

const REMOVE: &str = "edit.transform.removeWarpSplit";

fn invoke(h: &mut Harness, command: &str, params: serde_json::Value) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, command, params).unwrap();
}

fn pick(h: &mut Harness, column: usize, row: usize) {
    let anchor = session_warp(h.state()).mesh.point(3 * column, 3 * row);
    let p = active_view(h.state()).unwrap().to_screen(anchor);
    h.hover_at(p);
    h.run_steps(1);
    h.drag_at(p);
    h.run_steps(1);
    h.drop_at(p);
    h.run_steps(2);
}

fn warp_harness() -> Harness {
    let mut app = app_with_square(640, photocraft_geom::Rect::new(80, 80, 560, 560));
    let ctx = app.ctx().clone();
    begin_warp(&mut app, &ctx).unwrap();
    harness(app, egui::vec2(1400.0, 900.0))
}

fn grid(h: &mut Harness, size: usize) {
    invoke(h, "edit.transform.warpGrid", json!({"size": size}));
    h.run_steps(3);
}

#[test]
fn removing_a_warp_split_uses_the_picked_line() {
    let mut h = warp_harness();
    grid(&mut h, 3);
    let before = session_warp(h.state()).mesh.clone();
    pick(&mut h, 1, 0);
    invoke(&mut h, REMOVE, json!({}));
    let after = &session_warp(h.state()).mesh;
    assert!(!after.us.contains(&before.us[1]), "the picked vertical line remains: {:?}", after.us);
    assert!(after.us.contains(&before.us[2]));
    assert_eq!(after.vs, before.vs);
}

#[test]
fn picked_horizontal_split_is_one_undoable_warp_edit() {
    let mut h = warp_harness();
    grid(&mut h, 3);
    let mut mesh = warp_mesh(h.state());
    for point in [0, 1, mesh.width()] {
        mesh.points[point].x -= 16.0;
    }
    pull_mesh(h.state_mut(), 1, mesh);
    h.run_steps(3);

    let before = session_warp(h.state()).warp.clone();
    let history = h.state().session.active().unwrap().history.past_len();
    pick(&mut h, 0, 1);
    invoke(&mut h, REMOVE, json!({}));
    let after = session_warp(h.state()).warp.clone();
    let (before_mesh, after_mesh) = (before.mesh.as_ref().unwrap(), after.mesh.as_ref().unwrap());
    assert_eq!(after_mesh.us, before_mesh.us);
    assert!(!after_mesh.vs.contains(&before_mesh.vs[1]));
    assert!(after_mesh.vs.contains(&before_mesh.vs[2]));
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history, "the removal is still a preview");

    invoke(&mut h, "edit.undo", json!({}));
    assert_eq!(session_warp(h.state()).warp, before);
    invoke(&mut h, "edit.redo", json!({}));
    assert_eq!(session_warp(h.state()).warp, after);
    commit(h.state_mut());
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1, "the whole warp commits in one step");
}

#[test]
fn picking_the_only_added_split_restores_the_default_grid() {
    for (command, column, row) in [("edit.transform.splitWarpHorizontally", 0, 1), ("edit.transform.splitWarpVertically", 1, 0)] {
        let mut h = warp_harness();
        invoke(&mut h, command, json!({"at": [248, 248]}));
        h.run_steps(3);
        pick(&mut h, column, row);
        invoke(&mut h, REMOVE, json!({}));
        let warp = session_warp(h.state());
        assert_eq!((warp.mesh.columns(), warp.mesh.rows()), (1, 1), "{command} put back the guide-only grid");
        assert!(warp.warp.is_identity());
    }
}

fn assert_inner_anchors_restore_default(command: &str, anchors: [(usize, usize); 2]) {
    for (column, row) in anchors {
        let mut h = warp_harness();
        invoke(&mut h, command, json!({"at": [320, 320]}));
        h.run_steps(3);

        pick(&mut h, column, row);
        invoke(&mut h, REMOVE, json!({}));
        let warp = session_warp(h.state());
        assert_eq!((warp.mesh.columns(), warp.mesh.rows()), (1, 1), "{command} at ({column}, {row}) restores the Default grid");
        assert!(warp.warp.is_identity());
    }
}

#[test]
fn inner_anchors_remove_the_only_horizontal_split() {
    assert_inner_anchors_restore_default("edit.transform.splitWarpHorizontally", [(1, 1), (2, 1)]);
}

#[test]
fn inner_anchors_remove_the_only_vertical_split() {
    assert_inner_anchors_restore_default("edit.transform.splitWarpVertically", [(1, 1), (1, 2)]);
}

#[test]
fn an_explicit_control_channel_point_takes_precedence_over_the_pick() {
    let mut h = warp_harness();
    grid(&mut h, 3);
    pick(&mut h, 1, 0);

    let before = warp_mesh(h.state());
    let target = before.point(0, 6);
    let (req, _) = ControlRequest::new("ui.menu.invoke", json!({"command": REMOVE, "params": {"at": [target.x, target.y]}}));
    let ctx = h.ctx.clone();
    assert!(matches!(handle(h.state_mut(), &ctx, &req), Reply::Done(v) if v["ok"] == true));
    let after = &session_warp(h.state()).mesh;
    assert_eq!(after.us, before.us);
    assert!(!after.vs.contains(&before.vs[2]));
    assert!(after.vs.contains(&before.vs[1]));
}

fn assert_uses_center(h: &mut Harness) {
    let expected = split_warp(&session_warp(h.state()).warp, None, Split::Remove).unwrap();
    invoke(h, REMOVE, json!({}));
    assert_eq!(session_warp(h.state()).warp, expected);
}

#[test]
fn a_crossing_or_an_unpicked_grid_uses_the_center() {
    let mut h = warp_harness();
    grid(&mut h, 3);
    pick(&mut h, 1, 1);
    assert_uses_center(&mut h);

    invoke(&mut h, "edit.undo", json!({}));
    h.run_steps(2);
    assert_uses_center(&mut h);
}

#[test]
fn a_crossing_of_two_added_splits_uses_the_center() {
    let mut h = warp_harness();
    invoke(&mut h, "edit.transform.splitWarpHorizontally", json!({"at": [320, 320]}));
    invoke(&mut h, "edit.transform.splitWarpVertically", json!({"at": [344, 320]}));
    h.run_steps(3);

    pick(&mut h, 2, 1);
    assert_uses_center(&mut h);
}

#[test]
fn a_pick_on_the_old_grid_is_cleared_when_the_grid_changes() {
    let mut h = warp_harness();
    grid(&mut h, 4);
    pick(&mut h, 1, 0);

    // The picked index also names a vertical anchor on the new grid. It must not be reused.
    grid(&mut h, 3);
    assert_uses_center(&mut h);
}

#[test]
fn paging_to_a_warp_point_can_select_its_split() {
    let mut h = warp_harness();
    grid(&mut h, 3);
    pick(&mut h, 0, 0);

    let before = warp_mesh(h.state());
    for _ in 0..=before.handles(0).len() {
        h.key_press(egui::Key::PageDown);
        h.run_steps(1);
    }
    invoke(&mut h, REMOVE, json!({}));
    let after = &session_warp(h.state()).mesh;
    assert!(!after.us.contains(&before.us[1]), "the line reached with Page Down was removed");
    assert_eq!(after.vs, before.vs);
}

#[test]
fn the_picked_line_keeps_its_identity_on_a_folded_mesh() {
    let mut h = warp_harness();
    let bounds = session_warp(h.state()).warp.bounds;
    let mesh = BezierMesh::fit(
        &|s, t| {
            // The first interior anchor projects onto a point on the far patch.
            let x = if s <= 0.28 {
                0.75 * s / 0.28
            } else if s <= 0.7 {
                0.75 - 0.35 * (s - 0.28) / (0.7 - 0.28)
            } else {
                0.4 + 0.6 * (s - 0.7) / (1.0 - 0.7)
            };
            [bounds[0] + x * (bounds[2] - bounds[0]), bounds[1] + t * (bounds[3] - bounds[1])]
        },
        vec![0.0, 0.28, 0.7, 1.0],
        vec![0.0, 1.0],
    );
    let anchor = mesh.points[3];
    let warp = Warp::custom(mesh, bounds);
    let hit = split_warp(&warp, Some(anchor), Split::Remove).unwrap().mesh.unwrap();
    assert!(hit.us.contains(&0.28), "the document point alone found the other line");
    set_warp(h.state_mut(), warp);
    h.run_steps(3);
    pick(&mut h, 1, 0);
    invoke(&mut h, REMOVE, json!({}));
    assert!(!session_warp(h.state()).mesh.us.contains(&0.28));
    assert!(session_warp(h.state()).mesh.us.contains(&0.7));
}

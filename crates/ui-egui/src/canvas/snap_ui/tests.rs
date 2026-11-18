use super::*;
use crate::canvas::{ToolEvent, tool_event, tool_event_response};
use craft::behavior::geometry::SnapLine;
use serde_json::json;

fn app_with_box() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.views[0].zoom = 1.0;
    app.ui.views[0].clear_pending();
    // Two boxes: a target at (300..350, 200..260) and the moving one at (40..90, 40..80).
    app.run("layer.new.layer", json!({"name": "target"})).unwrap();
    app.run("select.rect", json!({"x": 300, "y": 200, "width": 50, "height": 60})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("layer.new.layer", json!({"name": "mover"})).unwrap();
    app.run("select.rect", json!({"x": 40, "y": 40, "width": 50, "height": 40})).unwrap();
    app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app
}

fn mover_bounds(app: &PhotocraftApp) -> photocraft_geom::Rect {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds()
}

/// The target of kind `kind` at `at` on `along` that a position within `tol` of `at + 0.5` snaps to.
fn near(app: &PhotocraftApp, exclude: &[LayerId], along: Axis, at: f64) -> Option<(SnapKind, f64)> {
    targets(app, 0, exclude).nearest(along, at + 0.5, 1.0).map(|t| (t.kind, t.position))
}

fn has_line(lines: &[SnapLine], position: f64) -> bool {
    lines.iter().any(|l| l.along == Axis::Y && l.position == position)
}

/// Drags the Move tool from the mover's body to `to` and returns the snap lines of the last move.
fn move_drag(app: &mut PhotocraftApp, to: [f64; 2], m: egui::Modifiers) -> Vec<SnapLine> {
    tool_event(app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, m);
    let lines = tool_event_response(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m).snapped;
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
    lines
}

/// #7538: the moving rect is the floating piece's outline at its offset, not the marquee round it.
#[test]
fn moving_selected_pixels_snaps_their_bounds_not_the_marquee() {
    for alt in [false, true] {
        let mut app = app_with_box();
        app.ui.tool = Tool::Move;
        app.run("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 90})).unwrap();
        let m = egui::Modifiers { alt, ..Default::default() };
        // 263 px right puts the piece's left edge 3 px from the target's (the marquee's 20 px off).
        let lines = move_drag(&mut app, [323.0, 100.0], m);
        assert!(has_line(&lines, 300.0), "alt {alt}: a line at the target's edge: {lines:?}");
        app.run("select.drop", json!({})).unwrap();
        let sel = app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds().x0);
        assert_eq!(sel, Some(300), "alt {alt}: snapped to the target's edge");
    }
}

/// Other layers' edges are targets and smart kinds; the moving set is not.
#[test]
fn move_tool_snaps_to_other_layer_edges_with_smart_guides() {
    let mut app = app_with_box();
    app.ui.tool = Tool::Move;
    app.ui.view.show.smart_guides = true;
    app.ui.tool_options.move_auto_select = false;
    let mover = app.session.active().unwrap().active_layer.unwrap();
    assert!(near(&app, &[mover], Axis::Y, 40.0).is_none(), "the moving layer's own edge is no target");
    assert_eq!(near(&app, &[], Axis::Y, 40.0), Some((SnapKind::LayerEdge, 40.0)));
    let lines = move_drag(&mut app, [323.0, 100.0], egui::Modifiers::NONE);
    assert!(lines.iter().any(|l| l.position == 300.0 && l.kind.is_smart()), "{lines:?}");
    assert_eq!(mover_bounds(&app).x0, 300);
}

/// The auto-selected layer is the moving set; an empty layer gives no box and no snap.
#[test]
fn move_tool_box_and_snapping_follow_the_auto_selected_layer() {
    for before in ["target", "empty"] {
        let mut app = app_with_box();
        let mover = app.session.active().unwrap().active_layer.unwrap();
        if before == "empty" {
            app.run("layer.new.layer", json!({"name": "empty"})).unwrap();
        } else {
            let st = app.session.active().unwrap();
            let target = st.doc.walk().into_iter().find(|(_, _, l)| l.name == "target").unwrap().2.id;
            app.run("layer.select", json!({"layer": target.0})).unwrap();
        }
        app.ui.tool = Tool::Move;
        app.ui.tool_options.move_auto_select = true;
        let lines = move_drag(&mut app, [323.0, 100.0], egui::Modifiers::NONE);
        assert_eq!(app.session.active().unwrap().active_layer, Some(mover), "{before}: Auto-Select picked the mover");
        assert!(has_line(&lines, 300.0), "{before}: {lines:?}");
        assert_eq!(mover_bounds(&app).x0, 300, "{before}: snapped to the target's edge");
    }
}

/// A ruler-dragged guide snaps to layer edges: layers are targets for guides.
#[test]
fn guides_snap_to_layer_edges() {
    let app = app_with_box();
    assert_eq!(near(&app, &[], Axis::Y, 300.0), Some((SnapKind::LayerEdge, 300.0)));
    assert_eq!(near(&app, &[], Axis::X, 200.0), Some((SnapKind::LayerEdge, 200.0)));
}

/// The target cache follows the Snap To options and the document, not only edits.
#[test]
fn guide_targets_follow_snap_to_options() {
    let mut app = app_with_box();
    let first = targets(&app, 0, &[]);
    assert!(Arc::ptr_eq(&first, &targets(&app, 0, &[])), "nothing changed: the same targets");
    assert!(near(&app, &[], Axis::Y, 300.0).is_some());
    app.ui.view.snap_to.layers = false;
    let second = targets(&app, 0, &[]);
    assert!(!Arc::ptr_eq(&first, &second), "a Snap To option rebuilds");
    assert!(near(&app, &[], Axis::Y, 300.0).is_none(), "layers off: no layer targets");
    app.run("layer.new.layer", json!({})).unwrap();
    assert!(!Arc::ptr_eq(&second, &targets(&app, 0, &[])), "a document revision rebuilds");
}

/// The percentage grid snaps at separate intervals per axis.
#[test]
fn percentage_grid_snaps_at_separate_axis_intervals() {
    let mut app = app_with_box();
    app.ui.view.snap_to.grid = true;
    app.ui.extras.grid = true;
    let (_, step) = grid(&app, app.session.active().unwrap());
    let kind_at = |along: Axis, v: f64| targets(&app, 0, &[]).nearest(along, v + 0.25, 0.5).map(|t| (t.kind, t.position));
    assert_eq!(kind_at(Axis::Y, step.x), Some((SnapKind::Grid, step.x)));
    assert_eq!(kind_at(Axis::X, step.y), Some((SnapKind::Grid, step.y)));
}

/// Grid targets come from Guides, Grid & Slices, and the snap carries the options.
#[test]
fn grid_snapping_uses_preferences() {
    let mut app = app_with_box();
    assert!(!snap(&app, 0).options().grid);
    app.ui.view.snap_to.grid = true;
    app.ui.extras.grid = true;
    assert!(snap(&app, 0).options().grid);
    let before = grid(&app, app.session.active().unwrap()).1;
    app.run("prefs.set", json!({"path": "guidesGridAndSlices.subdivisions", "value": 8})).unwrap();
    let after = grid(&app, app.session.active().unwrap()).1;
    assert!(after.x < before.x, "more subdivisions, a finer grid: {before:?} {after:?}");
}

/// The targets of a document: edges, centre, guides, grid, layers minus the moving set, an artboard,
/// the selection, and pixels past the canvas.
#[test]
fn snap_targets_follow_the_document() {
    let mut app = app_with_box();
    let mover = app.session.active().unwrap().active_layer.unwrap();
    // Document edges and centre.
    assert_eq!(near(&app, &[], Axis::Y, 0.0), Some((SnapKind::Bounds, 0.0)));
    assert_eq!(near(&app, &[], Axis::Y, 200.0), Some((SnapKind::Center, 200.0)));
    // Layers, minus the moving set.
    assert_eq!(near(&app, &[], Axis::Y, 350.0), Some((SnapKind::LayerEdge, 350.0)));
    assert!(near(&app, &[mover], Axis::Y, 90.0).is_none());
    // Guides, only while Snap To › Guides is on.
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 321})).unwrap();
    assert_eq!(near(&app, &[], Axis::Y, 321.0), Some((SnapKind::Guide, 321.0)));
    app.ui.view.snap_to.guides = false;
    assert!(near(&app, &[], Axis::Y, 321.0).is_none());
    // The selection's edges: not a target while the Move tool would carry its pixels (#6290).
    app.ui.tool = Tool::RectMarquee;
    app.run("select.rect", json!({"x": 120, "y": 100, "width": 60, "height": 50})).unwrap();
    assert_eq!(near(&app, &[], Axis::Y, 120.0), Some((SnapKind::Selection, 120.0)));
    app.run("select.deselect", json!({})).unwrap();
    // An artboard offers its own edges and centre.
    app.session
        .edit("artboard", |doc, _| {
            let mut board = photocraft_doc::Layer::group("Artboard 1", vec![]);
            if let photocraft_doc::LayerContent::Group(g) = &mut board.content {
                g.artboard = Some(photocraft_doc::Artboard {
                    rect: photocraft_geom::Rect::new(20, 110, 120, 170),
                    background: Default::default(),
                    preset: String::new(),
                });
            }
            doc.insert_above(None, board);
            Ok(())
        })
        .unwrap();
    assert_eq!(near(&app, &[], Axis::Y, 120.0), Some((SnapKind::LayerEdge, 120.0)));
    // Its centre line is off the document's (y 150), which would win a tie.
    assert_eq!(near(&app, &[], Axis::X, 140.0), Some((SnapKind::LayerCenter, 140.0)));
    // A layer whose pixels reach past the canvas offers its whole rect (#1919).
    app.run("layer.new.layer", json!({"name": "off"})).unwrap();
    let off = app.session.active().unwrap().active_layer.unwrap();
    app.session
        .edit("paint past the edge", |doc, _| {
            doc.layer_mut(off).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(-40, -20, 160, 80), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    for (along, at) in [(Axis::Y, -40.0), (Axis::X, -20.0), (Axis::Y, 160.0), (Axis::X, 80.0)] {
        assert_eq!(near(&app, &[], along, at), Some((SnapKind::LayerEdge, at)), "{along:?} {at}");
    }
}

/// A group of two layers (40..90 x 40..80 and 100..140 x 100..130) selected, as one layer.
fn app_with_group() -> PhotocraftApp {
    let mut app = app_with_box();
    app.run("layer.new.layer", json!({"name": "second"})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 40, "height": 30})).unwrap();
    app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("layer.select", json!({"name": "mover"})).unwrap();
    app.run("layer.select", json!({"name": "second", "mode": "add"})).unwrap();
    app.run("layer.groupLayers", json!({"name": "G"})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = false;
    app
}

fn child_bounds(app: &PhotocraftApp, name: &str) -> photocraft_geom::Rect {
    let st = app.session.active().unwrap();
    let (_, _, l) = st.doc.walk().into_iter().find(|(_, _, l)| l.name == name).unwrap();
    l.surface().unwrap().content_bounds()
}

/// A plain Move drag from `from` to `to`, released there.
fn group_drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    crate::canvas::tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    crate::canvas::tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

/// #13418: a group is framed by the layers inside it, whole even with a selection: it moves them
/// whole, so the frame is not cut to the selection as a pixel layer's is.
#[test]
fn a_group_is_framed_by_the_layers_inside_it() {
    for selected in [false, true] {
        let mut app = app_with_group();
        if selected {
            app.run("select.rect", json!({"x": 50, "y": 50, "width": 20, "height": 20})).unwrap();
        }
        let st = app.session.active().unwrap();
        let group = st.active_layer.unwrap();
        assert!(st.doc.layer(group).unwrap().is_group());
        assert_eq!(tree_rect(&st.doc, group), Some([40.0, 40.0, 140.0, 130.0]), "selection {selected}");
    }
}

/// #13418: the group's edge snaps to another layer's edge as a single layer's does.
#[test]
fn a_dragged_group_snaps_to_other_layers() {
    let mut app = app_with_group();
    // 72 px down puts the group's bottom edge (130 -> 202) 2 px past the target's top (200); no
    // other edge or centre is within reach.
    group_drag(&mut app, [60.0, 60.0], [80.0, 132.0]);
    let (m, s) = (child_bounds(&app, "mover"), child_bounds(&app, "second"));
    assert_eq!((m.x0, m.y0, s.y0), (60, 110, 170), "snapped 2 px up, the layers kept their spacing");
}

/// #13418: a moving group's layers are not snap targets, or it would snap to where it started.
#[test]
fn a_dragged_group_does_not_snap_to_its_own_layers() {
    let mut app = app_with_group();
    // +46, +3: the group's left edge (86) is 4 px from where the mover's right edge was (90) and
    // its top (43) 3 px from where the mover's top was (40); nothing else is in reach.
    group_drag(&mut app, [60.0, 60.0], [106.0, 63.0]);
    let b = child_bounds(&app, "mover");
    assert_eq!((b.x0, b.y0), (86, 43), "moved by the raw offset");
}

/// Selected pixels float off the active layer alone: a layer linked to it stays put, so the
/// pixels still snap to it.
#[test]
fn moving_selected_pixels_snaps_to_a_linked_layer() {
    let mut app = app_with_box();
    app.run("layer.new.layer", json!({"name": "linked"})).unwrap();
    app.run("select.rect", json!({"x": 150, "y": 120, "width": 40, "height": 20})).unwrap();
    app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("layer.select", json!({"name": "mover"})).unwrap();
    app.run("layer.select", json!({"name": "linked", "mode": "add"})).unwrap();
    app.run("layer.linkLayers", json!({"linked": true})).unwrap();
    app.run("layer.select", json!({"name": "mover"})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = false;
    app.run("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 90})).unwrap();
    // 37 px down puts the pixels' bottom edge (80 -> 117) 3 px above the linked layer's top (120);
    // no other edge or centre is within reach.
    group_drag(&mut app, [60.0, 60.0], [60.0, 97.0]);
    app.run("select.drop", json!({})).unwrap();
    let sel = app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds().y0);
    assert_eq!(sel, Some(80), "snapped to the linked layer's top");
    assert_eq!(child_bounds(&app, "linked").y0, 120, "the linked layer stayed put");
}

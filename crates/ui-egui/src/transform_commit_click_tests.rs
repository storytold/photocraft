//! Free Transform commit rules for clicks (#11100): a click outside the box on the image keeps the
//! box open, a click on the pasteboard commits.

use egui::Modifiers;
use photocraft_doc::LayerId;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};

/// Moves the open box 10 px right, reference point and corners together.
fn shift_right(app: &mut PhotocraftApp) {
    app.ui.transform.as_mut().expect("a box is open").edit_box(|b| *b = b.moved((10.0, 0.0).into()));
}

fn quad(app: &PhotocraftApp) -> [[f64; 2]; 4] {
    app.ui.transform.as_ref().expect("a box is open").session.quad().map(|p| [p.x, p.y])
}

/// A 200 × 200 document with a red block (x 20..60, y 20..60) on layer "Paint", Free Transform
/// open on it, and the box moved 10 px to the right as in the measurement.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    let paint = LayerId(app.run("layer.new.layer", json!({"name": "Paint"})).unwrap()["layer"].as_u64().unwrap());
    app.run("layer.select", json!({"layer": paint.0})).unwrap();
    app.run("select.rect", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let app_ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &app_ctx, "edit.freeTransform", json!({})).unwrap();
    shift_right(&mut app);
    app
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn click(app: &mut PhotocraftApp, p: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: p[0], y: p[1] }, Modifiers::NONE);
}

#[test]
fn a_click_outside_the_box_on_the_image_keeps_it_open() {
    let mut app = app();
    let (before, q) = (steps(&app), quad(&app));
    click(&mut app, [150.0, 150.0]);
    assert!(app.ui.transform.is_some(), "the box stays open");
    assert_eq!(quad(&app), q, "and unchanged");
    assert_eq!(steps(&app), before);
}

#[test]
fn a_click_on_the_pasteboard_commits_one_free_transform_step() {
    for p in [[-30.0, 100.0], [100.0, -30.0], [230.0, 100.0], [100.0, 230.0]] {
        let mut app = app();
        // This oracle addresses the canvas at these same four document points.
        // The floating task bar can cover them; use the real Window command to
        // expose the pasteboard without changing the click or bypassing its route.
        let app_ctx = app.ctx().clone();
        crate::menus::invoke(&mut app, &app_ctx, "window.panel.contextualTaskBar", json!({"on": false})).unwrap();
        assert!(!app.ui.panels.context_task_bar);
        crate::canvas::testing::shown(&mut app);
        let view = crate::canvas::testing::view(&app).expect("the canvas shows the original click point");
        let point = crate::canvas::testing::to_screen(&app, p);
        assert!(view.rect.contains(point), "the unchanged pasteboard point reaches the real canvas: {p:?}");
        assert!(crate::transform_task_bar::shown_rect(&app, &app_ctx).is_none(), "the bar cannot own this canvas click");
        let before = steps(&app);
        click(&mut app, p);
        assert!(app.ui.transform.is_none(), "{p:?}");
        assert_eq!(steps(&app), before + 1, "{p:?}");
        assert_eq!(app.session.active().unwrap().history.entries().last().map(ToString::to_string).as_deref(), Some("Free Transform"));
    }
}

/// #11100, measured in the reference application: a click outside the box on another layer's
/// pixels commits one Free Transform step, and the transformed layer stays the only selected one
/// (the click doesn't pick the layer under it).
#[test]
fn a_click_on_another_layers_pixels_commits_and_keeps_the_selection() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    // "Other" has a blue block at x 120..160, y 120..160; "Paint" a red one at x 20..60, y 20..60.
    let mut other = None;
    for (name, at, color) in [("Other", 120, "#0000ff"), ("Paint", 20, "#ff0000")] {
        let id = app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap();
        if name == "Other" {
            other = Some(LayerId(id));
        }
        app.run("layer.select", json!({"layer": id})).unwrap();
        app.run("select.rect", json!({"x": at, "y": at, "width": 40, "height": 40})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
    }
    let paint = app.session.active().unwrap().active_layer.unwrap();
    let app_ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &app_ctx, "edit.freeTransform", json!({})).unwrap();
    app.ui.transform.as_mut().unwrap().edit_box(|b| *b = b.scaled_about_pivot(2.0, 2.0));
    let before = steps(&app);
    click(&mut app, [140.0, 140.0]);
    assert!(app.ui.transform.is_none(), "committed");
    assert_eq!(steps(&app), before + 1);
    let bounds = app.session.active().unwrap().doc.layer(paint).unwrap().surface().unwrap().content_bounds();
    assert_eq!(bounds, photocraft_geom::Rect::new(0, 0, 80, 80), "the resized pixels are committed");
    assert_eq!(app.session.active().unwrap().history.entries().last().map(ToString::to_string).as_deref(), Some("Free Transform"));
    assert_eq!(app.session.active().unwrap().selected_layers(), vec![paint], "the selection doesn't change");
    app.ui.tool = crate::state::Tool::Move;
    app.ui.tool_options.move_auto_select = true;
    app.ui.tool_options.move_target = "layer".into();
    click(&mut app, [140.0, 140.0]);
    assert_eq!(app.session.active().unwrap().selected_layers(), vec![other.unwrap()], "the second click picks the other layer");
}

/// A layer linked to the transformed one after the box opened moves with the commit
/// (`edit.transform` takes the link group along), so a click on its pixels is not a click on
/// another layer: the box stays open.
#[test]
fn a_click_on_a_layer_linked_after_the_box_opened_keeps_it_open() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    let mut ids = Vec::new();
    for (name, at, color) in [("Other", 120, "#0000ff"), ("Paint", 20, "#ff0000")] {
        let id = app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap();
        app.run("layer.select", json!({"layer": id})).unwrap();
        app.run("select.rect", json!({"x": at, "y": at, "width": 40, "height": 40})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        ids.push(LayerId(id));
    }
    let app_ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &app_ctx, "edit.freeTransform", json!({})).unwrap();
    shift_right(&mut app);
    app.session
        .edit("Link Layers", |doc, _| {
            for id in &ids {
                doc.layer_mut(*id).unwrap().link_group = Some(7);
            }
            Ok(())
        })
        .unwrap();
    click(&mut app, [140.0, 140.0]);
    assert!(app.ui.transform.is_some(), "the linked layer moves with the box: the click doesn't commit");
}

/// Only the frontmost layer under the click counts: where a grouped layer (not measured, so the
/// box stays open there) covers a top-level layer's pixels, the click doesn't commit through the
/// covered layer.
#[test]
fn a_click_on_a_grouped_layer_covering_another_layer_keeps_the_box_open() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    // "Paint" has a red block at x 20..60, y 20..60; "Other" a blue one at x 120..160, y 120..160,
    // covered by a green one on "Cover", which then goes into a group.
    let mut ids = Vec::new();
    for (name, at, color) in [("Paint", 20, "#ff0000"), ("Other", 120, "#0000ff"), ("Cover", 120, "#00ff00")] {
        let id = app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap();
        app.run("layer.select", json!({"layer": id})).unwrap();
        app.run("select.rect", json!({"x": at, "y": at, "width": 40, "height": 40})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        ids.push(id);
    }
    app.run("layer.groupLayers", json!({})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert!(doc.layers.iter().all(|l| l.id.0 != ids[2]), "Cover is grouped");
    assert!(doc.layers.iter().any(|l| l.id.0 == ids[1]), "Other is top-level");
    app.run("layer.select", json!({"layer": ids[0]})).unwrap();
    let app_ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &app_ctx, "edit.freeTransform", json!({})).unwrap();
    shift_right(&mut app);
    let before = steps(&app);
    click(&mut app, [140.0, 140.0]);
    assert!(app.ui.transform.is_some(), "the box stays open");
    assert_eq!(steps(&app), before);
}

/// An 800 × 600 document with "A" (red, x 120..220, y 150..250) and "B" (blue, x 300..400) on one
/// artboard, the loose layer "C" (magenta, 100 × 80 at `loose`) on no board, and an empty second
/// board 100 px right of the first, hidden when `hidden`. Free Transform is open on A, its box
/// moved 10 px right. Returns A and the two boards' rects.
fn on_artboards(loose: [i32; 2], hidden: bool) -> (Box<PhotocraftApp>, LayerId, [photocraft_geom::Rect; 2]) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "transparent"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    let mut ids = Vec::new();
    for (name, at, size, color) in [("C", loose, [100, 80], "#ff00ff"), ("B", [300, 150], [100, 100], "#0000ff"), ("A", [120, 150], [100, 100], "#ff0000")] {
        let id = app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap();
        app.run("layer.select", json!({"layer": id})).unwrap();
        app.run("select.rect", json!({"x": at[0], "y": at[1], "width": size[0], "height": size[1]})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        ids.push(id);
    }
    app.run("layer.select", json!({"layer": ids[1], "mode": "add"})).unwrap();
    app.run("layer.new.artboardFromLayers", json!({"name": "Board"})).unwrap();
    let first = board_rects(&app)[0];
    let second = [first.x1 + 100, first.y0, 200, first.height() as i32];
    let board = app.run("layer.new.artboard", json!({"rect": second, "name": "Second"})).unwrap()["layer"].as_u64().unwrap();
    if hidden {
        app.run("layer.hideLayers", json!({"layer": board})).unwrap();
    }
    app.run("layer.select", json!({"layer": ids[2]})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert!(doc.layers.iter().all(|l| l.id.0 != ids[2]), "A is on a board");
    assert!(doc.layers.iter().any(|l| l.id.0 == ids[0]), "C is on no board");
    let rects = board_rects(&app);
    assert_eq!(rects.len(), 2);
    let app_ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &app_ctx, "edit.freeTransform", json!({})).unwrap();
    shift_right(&mut app);
    (app, LayerId(ids[2]), [rects[0], rects[1]])
}

/// The artboards' rects, in document pixels, first board first.
fn board_rects(app: &PhotocraftApp) -> Vec<photocraft_geom::Rect> {
    let mut rects: Vec<_> = app.session.active().unwrap().doc.layers.iter().filter_map(|l| l.artboard()).map(|a| a.rect).collect();
    rects.sort_by_key(|r| r.x0);
    rects
}

fn inside_canvas(app: &PhotocraftApp, p: [f64; 2]) {
    let size = app.session.active().unwrap().doc.size;
    assert!(p[0] >= 0.0 && p[1] >= 0.0 && p[0] < f64::from(size.width) && p[1] < f64::from(size.height), "{p:?} is inside the canvas size {size:?}");
}

fn commits(app: &mut PhotocraftApp, a: LayerId, p: [f64; 2]) {
    inside_canvas(app, p);
    let before = steps(app);
    click(app, p);
    assert!(app.ui.transform.is_none(), "committed at {p:?}");
    assert_eq!(steps(app), before + 1);
    assert_eq!(app.session.active().unwrap().history.entries().last().map(ToString::to_string).as_deref(), Some("Free Transform"));
    assert_eq!(app.session.active().unwrap().selected_layers(), vec![a], "the selection doesn't change");
}

fn stays_open(app: &mut PhotocraftApp, p: [f64; 2]) {
    inside_canvas(app, p);
    let (before, q) = (steps(app), quad(app));
    click(app, p);
    assert!(app.ui.transform.is_some(), "the box stays open at {p:?}");
    assert_eq!(quad(app), q);
    assert_eq!(steps(app), before);
}

/// Measured in the reference application (#17019): in an artboard document the pasteboard is what
/// lies outside the bounding box of the visible boards, also within the canvas size, so a click
/// there commits one Free Transform step and the transformed layer stays selected. That holds on a
/// loose layer's pixels out there too.
#[test]
fn a_click_outside_the_artboards_commits() {
    let (mut app, a, [_, second]) = on_artboards([100, 480], false);
    commits(&mut app, a, [f64::from(second.x1) + 20.0, f64::from(second.y0) + 10.0]);
    let (mut app, a, _) = on_artboards([100, 480], false);
    commits(&mut app, a, [150.0, 520.0]);
}

/// Measured in the reference application (#17019): a hidden board doesn't count, a click on its
/// area commits.
#[test]
fn a_click_on_a_hidden_artboard_commits() {
    let (mut app, a, [_, second]) = on_artboards([100, 480], true);
    commits(&mut app, a, [f64::from(second.x0) + 50.0, f64::from(second.y0) + 50.0]);
}

/// Measured in the reference application (#17019): inside the boards' bounding box the box stays
/// open: on another layer's pixels on the board, on an empty spot of it, in the gap between two
/// boards, and on a loose layer's pixels in that gap.
#[test]
fn a_click_within_the_artboards_keeps_the_box_open() {
    let (mut app, _, [first, _]) = on_artboards([100, 480], false);
    let gap = [f64::from(first.x1) + 50.0, f64::from(first.y0) + 20.0];
    // B's pixels, then the board between A's box (x 130..230) and B (x 300..400), then the gap.
    for p in [[350.0, 200.0], [260.0, 200.0], gap] {
        assert!(app.ui.transform.as_ref().is_some(), "the box is open");
        stays_open(&mut app, p);
    }
    // C fills the 100 px gap between the boards.
    let (mut app, _, _) = on_artboards([first.x1, first.y0], false);
    let p = [f64::from(first.x1) + 50.0, f64::from(first.y0) + 40.0];
    let doc = &app.session.active().unwrap().doc;
    let hit = photocraft_engine::pick_cmds::layers_at(doc, p[0] as i32, p[1] as i32);
    assert_eq!(hit.first().and_then(|id| doc.layer(*id)).map(|l| l.name.as_str()), Some("C"), "the click lands on C");
    stays_open(&mut app, p);
}

//! The Free Transform reference-point pointer (#11683). Away from the box's handles with Alt held
//! (inside, outside and in the rotation zone), and over a shown reference point, a press places the
//! point; the pointer there is the reference application's arrowhead with a small target circle,
//! which the canvas draws while the system pointer is hidden (`CursorIcon::None`), as it does for
//! the distort and skew pointers. Over a corner or edge handle the scale pointer stays, and without
//! Alt nothing changes.

use egui::{CursorIcon, Vec2};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::transform_tool::{Hit, box_hit, cursor, handle_tolerance, pointer, shows_reference_pointer};

/// A 256 px document with a red square at 32..160 and Free Transform open on its layer.
fn transforming(reference_point: bool) -> PhotocraftApp {
    let mut app = square_app(reference_point);
    crate::transform_tool::begin(&mut app, &egui::Context::default()).unwrap();
    app
}

/// [`transforming`] with the box still closed: for a harness that opens it through the menu.
fn square_app(reference_point: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 256, "height": 256})).unwrap();
    app.sync_views();
    app.run("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(32, 32, 160, 160), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.ui.tool_options.transform_reference_point = reference_point;
    app
}

/// The open box's corners, clockwise from the top-left.
fn quad(app: &PhotocraftApp) -> [[f64; 2]; 4] {
    app.ui.transform.as_ref().unwrap().quad
}

/// The middle of edge `i`, which joins corners `i` and `i + 1`.
fn edge_mid(q: [[f64; 2]; 4], i: usize) -> [f64; 2] {
    [(q[i][0] + q[(i + 1) % 4][0]) / 2.0, (q[i][1] + q[(i + 1) % 4][1]) / 2.0]
}

/// A point just outside corner 0, in the rotation zone, and one far outside the box.
fn outside_points(app: &PhotocraftApp) -> ([f64; 2], [f64; 2]) {
    let tol = handle_tolerance(app);
    let q = quad(app);
    ([q[0][0] - tol * 1.5, q[0][1] - tol * 1.5], [200.0, 200.0])
}

fn view(app: &PhotocraftApp) -> ViewXform {
    ViewXform::active(app).unwrap()
}

/// Alt inside the box, away from the handles: a press there puts the reference point down, so the
/// canvas draws its pointer with the system one hidden (#11683).
#[test]
fn alt_inside_the_box_shows_the_reference_pointer() {
    let app = transforming(false);
    let (xf, q) = (view(&app), quad(&app));
    let tol = handle_tolerance(&app);
    let p = [q[0][0] + 2.0 * tol, q[0][1] + 5.0 * tol];
    assert_eq!(box_hit(&app, app.ui.transform.as_ref().unwrap(), p, tol), Hit::Inside, "the point is the box's inside, off its handles");
    assert_eq!(cursor(&app, &xf, p, egui::Modifiers::ALT), Some(CursorIcon::None), "the canvas draws the reference-point pointer there");
    assert!(shows_reference_pointer(&app, p, egui::Modifiers::ALT), "and says so to the canvas");
    assert!(!shows_reference_pointer(&app, p, egui::Modifiers::NONE));
    assert_eq!(cursor(&app, &xf, p, egui::Modifiers::NONE), Some(CursorIcon::Move), "without Alt a press moves the content");
}

/// Alt outside the box, both in the rotation zone just beyond a corner and further away: the same
/// pointer, since a press places the point in both.
#[test]
fn alt_outside_the_box_shows_the_reference_pointer() {
    let app = transforming(false);
    let (xf, (zone, far)) = (view(&app), outside_points(&app));
    let tol = handle_tolerance(&app);
    let t = app.ui.transform.as_ref().unwrap();
    assert_eq!(box_hit(&app, t, zone, tol), Hit::Outside, "just beyond the corner, where a drag rotates");
    assert_eq!(box_hit(&app, t, far, tol), Hit::Outside, "and further out");
    assert!(crate::transform_tool::rotate_cursor_dir(&app, zone).is_some(), "the zone is the rotate one");
    assert!(crate::transform_tool::rotate_cursor_dir(&app, far).is_none(), "far out it is not");
    for p in [zone, far] {
        assert_eq!(cursor(&app, &xf, p, egui::Modifiers::ALT), Some(CursorIcon::None), "{p:?}: the canvas draws the reference-point pointer there");
        assert!(shows_reference_pointer(&app, p, egui::Modifiers::ALT), "{p:?}");
    }
    assert_eq!(cursor(&app, &xf, far, egui::Modifiers::NONE), Some(CursorIcon::Alias), "without Alt nothing changes");
}

/// The shown reference point is a handle of its own: a press there drags the point, so hovering it
/// shows the reference-point pointer, with or without Alt (#11683).
#[test]
fn hovering_the_shown_reference_point_shows_it() {
    let app = transforming(true);
    let (xf, pivot) = (view(&app), app.ui.transform.as_ref().unwrap().pivot);
    let tol = handle_tolerance(&app);
    assert_eq!(box_hit(&app, app.ui.transform.as_ref().unwrap(), pivot, tol), Hit::Pivot);
    for mods in [egui::Modifiers::NONE, egui::Modifiers::ALT] {
        assert_eq!(cursor(&app, &xf, pivot, mods), Some(CursorIcon::None), "alt {}: the canvas draws the reference-point pointer", mods.alt);
        assert!(shows_reference_pointer(&app, pivot, mods), "alt {}", mods.alt);
    }
}

/// Over a corner or an edge handle Alt does not take the point: the scale pointer stays (#11683).
#[test]
fn alt_over_a_handle_keeps_the_scale_pointer() {
    use CursorIcon::{ResizeHorizontal as H, ResizeNeSw as NeSw, ResizeNwSe as NwSe, ResizeVertical as V};
    let app = transforming(false);
    let (xf, q) = (view(&app), quad(&app));
    let alt = egui::Modifiers::ALT;
    let corners: Vec<_> = (0..4).map(|i| cursor(&app, &xf, q[i], alt)).collect();
    assert_eq!(corners, vec![Some(NwSe), Some(NeSw), Some(NwSe), Some(NeSw)], "the corner's own scale pointer");
    let edges: Vec<_> = (0..4).map(|i| cursor(&app, &xf, edge_mid(q, i), alt)).collect();
    assert_eq!(edges, vec![Some(V), Some(H), Some(V), Some(H)], "the edge's own scale pointer");
}

/// With Toggle Reference Point on, Alt over a handle still scales: the point is only placed away
/// from the handles.
#[test]
fn alt_over_a_handle_with_a_shown_reference_point_still_scales() {
    let app = transforming(true);
    let (xf, q) = (view(&app), quad(&app));
    for i in 0..4 {
        assert_ne!(cursor(&app, &xf, q[i], egui::Modifiers::ALT), Some(CursorIcon::None), "corner {i} stays a handle");
        assert_ne!(cursor(&app, &xf, edge_mid(q, i), egui::Modifiers::ALT), Some(CursorIcon::None), "edge {i} stays a handle");
    }
}

/// A drag in progress keeps the pointer its press started with: the reference application's
/// drag-time pointers were not measured (#11683).
#[test]
fn a_drag_in_progress_keeps_its_pointer() {
    let mut app = transforming(true);
    let (xf, pivot) = (view(&app), app.ui.transform.as_ref().unwrap().pivot);

    // Alt held, dragging the shown reference point: today's crosshair.
    pointer(&mut app, ToolEvent::Down { x: pivot[0], y: pivot[1], pressure: 1.0 }, egui::Modifiers::ALT);
    pointer(&mut app, ToolEvent::Move { x: pivot[0] + 6.0, y: pivot[1], pressure: 1.0 }, egui::Modifiers::ALT);
    assert_eq!(cursor(&app, &xf, pivot, egui::Modifiers::ALT), Some(CursorIcon::Crosshair), "the drag keeps its pointer");

    // A move drag started without Alt, Alt pressed during it: still the move pointer.
    let mut app = transforming(false);
    let (xf, q) = (view(&app), quad(&app));
    let tol = handle_tolerance(&app);
    let p = [q[0][0] + 2.0 * tol, q[0][1] + 5.0 * tol];
    pointer(&mut app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, egui::Modifiers::NONE);
    pointer(&mut app, ToolEvent::Move { x: p[0] + 6.0, y: p[1], pressure: 1.0 }, egui::Modifiers::NONE);
    assert_eq!(cursor(&app, &xf, p, egui::Modifiers::ALT), Some(CursorIcon::Move), "Alt mid-drag does not take the pointer");
}

/// Content-Aware Scale keeps today's pointers (#11683): its box only scales and moves, and its
/// pointer was not measured for the reference point.
#[test]
fn content_aware_scale_keeps_its_pointer() {
    for reference_point in [false, true] {
        let mut app = transforming(reference_point);
        let (xf, q) = (view(&app), quad(&app));
        let tol = handle_tolerance(&app);
        app.ui.transform.as_mut().unwrap().content_aware = Some(Default::default());
        let inside = [q[0][0] + 2.0 * tol, q[0][1] + 5.0 * tol];
        let far = outside_points(&app).1;
        let pivot = app.ui.transform.as_ref().unwrap().pivot;
        let points: Vec<[f64; 2]> = if reference_point { vec![inside, far, pivot] } else { vec![inside, far] };
        for p in points {
            assert_eq!(cursor(&app, &xf, p, egui::Modifiers::ALT), Some(CursorIcon::Crosshair), "Content-Aware Scale keeps its crosshair at {p:?}");
            assert!(!shows_reference_pointer(&app, p, egui::Modifiers::ALT), "{p:?}");
        }
    }
}

/// The pointer the canvas paints at `at`: an arrowhead with its tip there and a small target circle
/// beside it, with the system pointer hidden, and never the curved turn arrow (#11683).
fn assert_paints_the_reference_pointer(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, at: egui::Pos2, what: &str) {
    h.hover_at(at);
    h.run_steps(3);
    assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::None, "{what}: the system pointer is hidden");
    let near = |c: egui::Pos2| (c - at).length() < 32.0;
    // The target circle, just below right of the pointer.
    let target = h
        .output()
        .shapes
        .iter()
        .filter(|s| matches!(&s.shape, egui::Shape::Circle(c) if near(c.center) && (c.center - at).x > 5.0 && (c.center - at).y > 5.0 && c.radius > 2.0 && c.radius < 6.0))
        .count();
    assert!(target >= 1, "{what}: a small target circle is painted beside the pointer");
    // The arrowhead, its tip on the pointer: the closed four-point outline it is painted from.
    let arrow = h
        .output()
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, egui::Shape::Path(p) if p.closed && p.points.len() == 4 && p.points.iter().any(|q| (*q - at).length() < 0.01)));
    assert!(arrow, "{what}: an arrowhead is painted with its tip on the pointer");
    let turn =
        h.output().shapes.iter().filter(|s| matches!(&s.shape, egui::Shape::Path(p) if p.points.len() == 17 && p.points.iter().all(|q| near(*q)))).count();
    assert_eq!(turn, 0, "{what}: the rotate arrow does not take the pointer's place");
}

/// The canvas paints the pointer where `cursor` hides the system one, inside the box and in its
/// rotation zone, where the preview's overlay would otherwise draw the curved turn arrow (#11683).
#[test]
fn the_canvas_paints_the_reference_pointer_at_the_pointer() {
    let app = square_app(false);
    let mut h = egui_kittest::Harness::builder().with_size(Vec2::new(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    h.run_steps(3);
    h.state_mut().ui.shell.sticky_alt = true;
    let (xf, tol, q) = (ViewXform::active(h.state()).unwrap(), handle_tolerance(h.state()), quad(h.state()));
    let inside = xf.to_screen((q[0][0] + 2.0 * tol) as f32, (q[0][1] + 5.0 * tol) as f32);
    // Just beyond a corner: the rotate zone, whose arrow both the preview's overlay and the canvas's
    // cursor branch would draw for a plain hover.
    let zone = xf.to_screen((q[0][0] - tol * 1.5) as f32, (q[0][1] - tol * 1.5) as f32);
    assert_paints_the_reference_pointer(&mut h, zone, "the rotation zone, with the preview's overlay");
    // No preview: the canvas's cursor branch is what paints the pointer (#4139).
    h.state_mut().transform_preview = None;
    assert!(h.state().transform_preview.is_none());
    for (at, what) in [(inside, "inside the box"), (zone, "the rotation zone, with no preview")] {
        assert_paints_the_reference_pointer(&mut h, at, what);
    }
}

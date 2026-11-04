use super::*;
use crate::state::{ContentAwareOptions, TransformMode, TransformSession};
use crate::transform_tool::{Hit, cursor, hit, rotate_cursor_dir};
use egui::{CursorIcon, Modifiers, pos2};

const QUAD: [[f64; 2]; 4] = [[100.0, 100.0], [500.0, 100.0], [500.0, 300.0], [100.0, 300.0]];

fn session(quad: [[f64; 2]; 4]) -> TransformSession {
    TransformSession {
        session: 1,
        layer: 1,
        rect: [100.0, 100.0, 500.0, 300.0],
        quad,
        pivot: quad.iter().fold([0.0; 2], |c, p| [c[0] + p[0] / 4.0, c[1] + p[1] / 4.0]),
        interpolation: "bicubic".into(),
        warp: None,
        selection: false,
        target: None,
        path: None,
        made: None,
        mode: TransformMode::Free,
        content_aware: None,
        skew_v: 0.0,
        layers: Vec::new(),
    }
}

fn app(quad: [[f64; 2]; 4], zoom: f32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", serde_json::json!({"width": 32, "height": 32})).unwrap();
    app.sync_views();
    app.ui.views[0].zoom = zoom;
    app.ui.transform = Some(session(quad));
    app
}

fn view(zoom: f32) -> ViewXform {
    ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0)), zoom, center: [300.0, 200.0], flip: false, rotation: 0.0, aspect: 1.0 }
}

fn near(actual: Option<[f64; 2]>, expected: [f64; 2]) {
    let actual = actual.expect("rotate direction");
    assert!((actual[0] - expected[0]).hypot(actual[1] - expected[1]) < 1e-6, "{actual:?} != {expected:?}");
}

#[test]
fn whole_edge_and_corner_priority_table_at_100_and_200_percent() {
    for zoom in [1.0, 2.0] {
        let q = QUAD.map(|p| p.map(|v| v / zoom));
        let t = session(q);
        // Coordinates are screen pixels, so the same table must hold at both document zooms.
        for (p, expected) in [
            ([100.0, 100.0], Hit::Corner(0)),
            ([105.0, 105.0], Hit::Corner(0)),
            ([95.0, 95.0], Hit::Corner(0)),
            ([500.0, 300.0], Hit::Corner(2)),
            ([180.0, 100.0], Hit::Edge(0)),
            ([180.0, 89.0], Hit::Edge(0)),
            ([500.0, 150.0], Hit::Edge(1)),
            ([511.0, 150.0], Hit::Edge(1)),
            ([420.0, 311.0], Hit::Edge(2)),
            ([89.0, 250.0], Hit::Edge(3)),
            ([180.0, 101.0], Hit::Inside),
            ([499.0, 150.0], Hit::Inside),
            ([180.0, 180.0], Hit::Inside),
            ([180.0, 88.0], Hit::Outside),
            ([620.0, 200.0], Hit::Outside),
        ] {
            assert_eq!(hit(&t, p.map(|v| v / zoom), 12.0 / zoom), expected, "zoom {zoom}, screen {p:?}");
        }
        let tiny = session([[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]);
        assert_eq!(TransformSession::box_hit(&tiny.quad, [5.0, -1.0], 12.0), Hit::Corner(0), "corners beat overlapping edge bands");
    }
}

#[test]
fn turned_box_zones_and_resize_cursors_follow_the_quad() {
    let turn = |p: [f64; 2]| {
        let (x, y) = (p[0] - 300.0, p[1] - 200.0);
        let k = std::f64::consts::FRAC_1_SQRT_2;
        [300.0 + (x - y) * k, 200.0 + (x + y) * k]
    };
    for zoom in [1.0_f32, 2.0] {
        let scale = |p: [f64; 2]| turn(p).map(|v| v / f64::from(zoom));
        let q = QUAD.map(scale);
        let app = app(q, zoom);
        let xf = view(zoom);
        let t = app.ui.transform.as_ref().unwrap();
        for (p, expected, icon) in [
            ([500.0, 100.0], Hit::Corner(1), CursorIcon::ResizeHorizontal),
            ([180.0, 95.0], Hit::Edge(0), CursorIcon::ResizeNeSw),
            ([505.0, 150.0], Hit::Edge(1), CursorIcon::ResizeNwSe),
            ([180.0, 101.0], Hit::Inside, CursorIcon::Default),
            ([180.0, -20.0], Hit::Outside, CursorIcon::None),
        ] {
            assert_eq!(hit(t, scale(p), 12.0 / f64::from(zoom)), expected);
            assert_eq!(cursor(&app, &xf, scale(p), Modifiers::NONE), Some(icon), "zoom {zoom}, {p:?}");
        }
        assert_eq!(hit(t, scale([100.0, 100.0]), 12.0 / f64::from(zoom)), Hit::Corner(0));
    }
}

#[test]
fn distorted_and_flipped_quads_use_their_edges_not_an_axis_aligned_box() {
    let q = [[100.0, 100.0], [500.0, 140.0], [460.0, 300.0], [80.0, 260.0]];
    for q in [q, [q[1], q[0], q[3], q[2]]] {
        let t = session(q);
        assert_eq!(hit(&t, [220.0, 112.0], 12.0), Hit::Edge(0));
        assert_eq!(hit(&t, [220.0, 107.0], 12.0), Hit::Edge(0));
        assert_eq!(hit(&t, [220.0, 113.0], 12.0), Hit::Inside);
        assert_eq!(hit(&t, [220.0, 90.0], 12.0), Hit::Outside);
        near(TransformSession::box_turn_direction(&q, [220.0, 90.0]), [-0.1 / 1.01_f64.sqrt(), 1.0 / 1.01_f64.sqrt()]);
    }
}

#[test]
fn concave_quad_edge_direction_uses_winding_in_both_orientations() {
    let q = [[0.0, 0.0], [100.0, 0.0], [10.0, 10.0], [0.0, 100.0]];
    for q in [q, [q[1], q[0], q[3], q[2]]] {
        near(TransformSession::box_turn_direction(&q, [56.0, 14.0]), [-1.0 / 82.0_f64.sqrt(), -9.0 / 82.0_f64.sqrt()]);
    }
}

#[test]
fn outside_rotation_has_eight_edge_and_corner_orientations() {
    let app = app(QUAD, 1.0);
    let h = std::f64::consts::FRAC_1_SQRT_2;
    for (p, direction) in [
        ([300.0, -20.0], [0.0, 1.0]),
        ([620.0, -20.0], [-h, h]),
        ([620.0, 200.0], [-1.0, 0.0]),
        ([620.0, 420.0], [-h, -h]),
        ([300.0, 420.0], [0.0, -1.0]),
        ([-20.0, 420.0], [h, -h]),
        ([-20.0, 200.0], [1.0, 0.0]),
        ([-20.0, -20.0], [h, h]),
    ] {
        near(rotate_cursor_dir(&app, p), direction);
        assert_eq!(cursor(&app, &view(1.0), p, Modifiers::NONE), Some(CursorIcon::None));
        assert_eq!(cursor(&app, &view(1.0), p, Modifiers::COMMAND), Some(CursorIcon::None));
    }
}

#[test]
fn unopened_move_controls_share_the_open_sessions_zones_and_cursors() {
    let app = app(QUAD, 1.0);
    let r = Rect::from_min_max(pos2(100.0, 100.0), pos2(500.0, 300.0));
    for p in [pos2(100.0, 100.0), pos2(180.0, 95.0), pos2(505.0, 150.0), pos2(180.0, 101.0), pos2(620.0, 200.0), pos2(-20.0, -20.0)] {
        let d = [f64::from(p.x), f64::from(p.y)];
        assert_eq!(transform_controls_cursor(r, p), cursor(&app, &view(1.0), d, Modifiers::NONE), "{p:?}");
        assert_eq!(transform_controls_hit(r, p), hit(app.ui.transform.as_ref().unwrap(), d, 12.0) != Hit::Inside);
        match (transform_controls_turn(r, p), rotate_cursor_dir(&app, d)) {
            (None, None) => {}
            (Some(a), Some(b)) => near(Some([f64::from(a.x) / 16.0, f64::from(a.y) / 16.0]), b),
            pair => panic!("unopened/open rotation disagreement: {pair:?}"),
        }
    }
    assert_eq!(transform_controls_cursor(r, r.center()), Some(CursorIcon::Default));
    assert!(!transform_controls_hit(r, r.center()), "inside remains an ordinary Move drag");
}

#[test]
fn modes_that_do_not_rotate_and_invalid_points_do_not_hide_the_pointer() {
    let mut app = app(QUAD, 1.0);
    for mode in [TransformMode::Distort, TransformMode::Skew] {
        app.ui.transform.as_mut().unwrap().mode = mode;
        assert_eq!(rotate_cursor_dir(&app, [620.0, 200.0]), None);
        assert_eq!(cursor(&app, &view(1.0), [620.0, 200.0], Modifiers::NONE), Some(CursorIcon::Default));
    }
    app.ui.transform.as_mut().unwrap().mode = TransformMode::Free;
    app.ui.transform.as_mut().unwrap().content_aware = Some(ContentAwareOptions::default());
    assert_eq!(rotate_cursor_dir(&app, [620.0, 200.0]), None);
    assert_eq!(cursor(&app, &view(1.0), [620.0, 200.0], Modifiers::NONE), Some(CursorIcon::Default));
    app.ui.transform.as_mut().unwrap().content_aware = None;
    let t = app.ui.transform.as_mut().unwrap();
    t.warp = Some(photocraft_geom::warp::Warp::none(t.rect));
    assert_eq!(rotate_cursor_dir(&app, [620.0, 200.0]), None);
    assert_eq!(cursor(&app, &view(1.0), [620.0, 200.0], Modifiers::NONE), Some(CursorIcon::Crosshair), "Warp retains its own cursor");
    app.ui.transform.as_mut().unwrap().warp = None;
    assert_eq!(rotate_cursor_dir(&app, [f64::NAN, 200.0]), None);
    assert_eq!(TransformSession::box_turn_direction(&[[0.0, 0.0]; 4], [20.0, 20.0]), None);
}

#[test]
fn rotate_drag_keeps_the_arrow_when_the_pointer_enters_the_box() {
    let mut app = app(QUAD, 1.0);
    app.ui.transform = None;
    app.session.execute("shape.create", serde_json::json!({"kind": "rect", "rect": [4, 4, 16, 16], "fill": "#ff0000"})).unwrap();
    let ctx = egui::Context::default();
    crate::transform_tool::begin(&mut app, &ctx).unwrap();
    crate::transform_tool::pointer(&mut app, crate::canvas::ToolEvent::Down { x: 80.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
    crate::transform_tool::pointer(&mut app, crate::canvas::ToolEvent::Move { x: 10.0, y: 10.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(rotate_cursor_dir(&app, [10.0, 10.0]).is_some(), "the held rotation gesture owns the arrow even inside");
    assert_eq!(cursor(&app, &view(1.0), [10.0, 10.0], Modifiers::NONE), Some(CursorIcon::None));
}

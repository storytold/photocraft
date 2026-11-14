//! #11801: the Perspective Crop quad has four edge midpoint handles. Dragging one moves that edge,
//! its corners sliding along the adjacent sides by the same fraction of each side; ⇧ scales the
//! quad about the opposite edge's centre, ⌥ moves the opposite edge the other way. A drag outside
//! the quad turns it about its centre. Corner and inside drags keep today's behaviour.

use egui::Modifiers;

use super::*;

const SHIFT: Modifiers = Modifiers::SHIFT;
const ALT: Modifiers = Modifiers::ALT;

/// An 800 × 500 document with the Perspective Crop tool and `quad` pending.
fn setup(quad: [[f64; 2]; 4]) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 500})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PerspectiveCrop;
    app.ui.perspective_crop = Some(quad);
    app
}

/// A press at `from`, a move half way and the release at `to`, all with `mods`.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    assert!(pointer(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods));
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    assert!(pointer(app, ToolEvent::Move { x: mid[0], y: mid[1], pressure: 1.0 }, mods));
    assert!(pointer(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods));
    assert!(pointer(app, ToolEvent::Up { x: to[0], y: to[1] }, mods));
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> [[f64; 2]; 4] {
    [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

fn near(a: [f64; 2], b: [f64; 2], tol: f64) -> bool {
    (a[0] - b[0]).abs() <= tol && (a[1] - b[1]).abs() <= tol
}

fn quad(app: &PhotocraftApp) -> [[f64; 2]; 4] {
    app.ui.perspective_crop.expect("a pending quad")
}

#[test]
fn an_edge_midpoint_drag_moves_that_edge_only() {
    let mut app = setup(rect(100.0, 50.0, 700.0, 450.0));
    drag(&mut app, [400.0, 450.0], [400.0, 400.0], Modifiers::NONE);
    assert_eq!(quad(&app), rect(100.0, 50.0, 700.0, 400.0), "the bottom edge up 50, the rest stays");
    // The right edge's midpoint, out to the right.
    drag(&mut app, [700.0, 225.0], [740.0, 225.0], Modifiers::NONE);
    assert_eq!(quad(&app), rect(100.0, 50.0, 740.0, 400.0));
}

#[test]
fn the_corners_slide_along_their_sides_by_the_same_fraction() {
    // Measured in the reference application, in screen units of a zoomed capture: a vertical left
    // side, a slanted right side and a slanted bottom edge; the bottom midpoint dragged up 97 moved
    // the left corner up 91 and the right corner up 103 and 14 to the right.
    let q = [[56.0, 48.0], [1272.0, 48.0], [1170.0, 824.0], [56.0, 727.0]];
    let mid = edge_mid(&q, 2);
    let out = edge_moved(q, 2, [0.0, -97.0]);
    assert!(near(out[3], [56.0, 727.0 - 91.0], 1.0), "left corner {:?}", out[3]);
    assert!(near(out[2], [1170.0 + 14.0, 824.0 - 103.0], 1.0), "right corner {:?}", out[2]);
    assert_eq!((out[0], out[1]), (q[0], q[1]), "the top edge stays");
    // Each corner stays on the line through its side.
    for (corner, other) in [(2, 1), (3, 0)] {
        let (s, t) = ([q[other][0] - q[corner][0], q[other][1] - q[corner][1]], [out[corner][0] - q[corner][0], out[corner][1] - q[corner][1]]);
        assert!((s[0] * t[1] - s[1] * t[0]).abs() < 1e-6, "corner {corner} left its side");
    }
    // The edge's midpoint follows the drag.
    let new_mid = edge_mid(&out, 2);
    assert!((new_mid[1] - (mid[1] - 97.0)).abs() < 1.0, "{new_mid:?}");
    // The same through the tool: a 50 px drag on a quad with a slanted right side.
    let mut app = setup([[100.0, 50.0], [760.0, 50.0], [700.0, 450.0], [100.0, 450.0]]);
    drag(&mut app, [400.0, 450.0], [400.0, 400.0], Modifiers::NONE);
    let got = quad(&app);
    assert!(near(got[3], [100.0, 400.3], 0.5) && near(got[2], [707.5, 400.3], 0.5), "{got:?}");
}

#[test]
fn shift_scales_the_quad_about_the_opposite_edge() {
    // 600 x 400 dragged to a height of 350: 525 x 350 (the reference application showed 526).
    let mut app = setup(rect(100.0, 50.0, 700.0, 450.0));
    drag(&mut app, [400.0, 450.0], [400.0, 400.0], SHIFT);
    let q = quad(&app);
    let (w, h) = (q[1][0] - q[0][0], q[3][1] - q[0][1]);
    assert!((w - 526.0).abs() <= 1.0 && (h - 350.0).abs() < 1e-9, "{w} x {h}");
    assert!(near(edge_mid(&q, 0), [400.0, 50.0], 1e-9), "anchored at the top edge's centre");
}

#[test]
fn alt_moves_the_opposite_edge_the_other_way() {
    let mut app = setup(rect(100.0, 50.0, 700.0, 450.0));
    drag(&mut app, [400.0, 450.0], [400.0, 400.0], ALT);
    assert_eq!(quad(&app), rect(100.0, 100.0, 700.0, 400.0), "bottom up 50, top down 50");
}

#[test]
fn ctrl_acts_as_a_plain_edge_drag() {
    let mut app = setup(rect(100.0, 50.0, 700.0, 450.0));
    drag(&mut app, [400.0, 450.0], [400.0, 400.0], Modifiers::CTRL);
    assert_eq!(quad(&app), rect(100.0, 50.0, 700.0, 400.0));
}

#[test]
fn a_drag_outside_turns_the_quad_about_its_centre() {
    let mut app = setup(rect(100.0, 50.0, 700.0, 450.0));
    // The centre is (400, 250): from straight right of it to straight below it, a quarter turn.
    drag(&mut app, [780.0, 250.0], [400.0, 480.0], Modifiers::NONE);
    let q = quad(&app);
    for (got, want) in q.iter().zip([[600.0, -50.0], [600.0, 550.0], [200.0, 550.0], [200.0, -50.0]]) {
        assert!(near(*got, want, 1e-6), "{q:?}");
    }
    // A small turn: the pointer's angle change around the centre.
    let mut app = setup(rect(100.0, 50.0, 700.0, 450.0));
    drag(&mut app, [780.0, 250.0], [780.0, 250.0 + 380.0 * 17.4_f64.to_radians().tan()], Modifiers::NONE);
    let q = quad(&app);
    let turned = (q[1][1] - q[0][1]).atan2(q[1][0] - q[0][0]).to_degrees();
    assert!((turned - 17.4).abs() < 1e-6, "{turned}");
}

#[test]
fn corners_win_over_midpoints_and_inside_still_moves() {
    // A small quad whose corner and edge midpoint are both within the tolerance of the press.
    let tol = crate::distort_ui::tolerance(&setup(rect(0.0, 0.0, 1.0, 1.0)));
    let side = tol;
    let mut app = setup(rect(200.0, 200.0, 200.0 + side, 200.0 + side));
    drag(&mut app, [200.0, 200.0], [190.0, 190.0], Modifiers::NONE);
    let q = quad(&app);
    assert_eq!(q[0], [190.0, 190.0], "the corner moved");
    assert_eq!(&q[1..], &rect(200.0, 200.0, 200.0 + side, 200.0 + side)[1..], "alone");
    // Inside, away from every handle: the whole quad moves.
    let mut app = setup(rect(100.0, 50.0, 700.0, 450.0));
    drag(&mut app, [300.0, 200.0], [320.0, 210.0], Modifiers::NONE);
    assert_eq!(quad(&app), rect(120.0, 60.0, 720.0, 460.0));
}

#[test]
fn degenerate_quads_never_panic() {
    let point = [[50.0, 50.0]; 4];
    assert_eq!(edge_moved(point, 2, [3.0, 4.0])[2], [53.0, 54.0], "no sides: the edge moves straight");
    assert_eq!(scale_from_edge(point, 2, [3.0, 4.0]), point);
    let _ = rotated(point, [50.0, 50.0], [60.0, 60.0]);
    for i in 0..4 {
        let _ = edge_moved([[0.0, 0.0], [f64::MAX, 0.0], [f64::MAX, f64::MAX], [0.0, f64::MAX]], i, [1.0, 1.0]);
    }
    // Through the tool, nothing non-finite is ever stored.
    let mut app = setup(point);
    drag(&mut app, [50.0, 50.0], [70.0, 80.0], SHIFT);
    assert!(quad(&app).iter().flatten().all(|v| v.is_finite()));
}

#[test]
fn eight_handles_are_drawn() {
    let app = setup(rect(100.0, 50.0, 700.0, 450.0));
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
        return;
    }
    let xf = ViewXform {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0)),
        zoom: 1.0,
        center: [400.0, 250.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    };
    let mut out = ctx.run_ui(Default::default(), |ui| {
        let painter = ui.ctx().layer_painter(egui::LayerId::background());
        draw_overlay(&app, &painter, &xf);
    });
    out.textures_delta.clear();
    let handles: Vec<egui::Pos2> = out
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.fill == Color32::WHITE && r.rect.width() == 8.0 => Some(r.rect.center()),
            _ => None,
        })
        .collect();
    assert_eq!(handles.len(), 8, "{handles:?}");
    for i in 0..4 {
        let m = edge_mid(&rect(100.0, 50.0, 700.0, 450.0), i);
        let at = xf.to_screen(m[0] as f32, m[1] as f32);
        assert!(handles.iter().any(|h| h.distance(at) < 0.01), "edge {i}'s midpoint handle at {at:?}");
    }
}

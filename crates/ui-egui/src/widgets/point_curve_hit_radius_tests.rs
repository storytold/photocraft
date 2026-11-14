//! #14443: in the Curves graph a click up to 15 screen px from an existing point selects that
//! point instead of adding a second one next to it, while a click clearly away still adds.
//! The other hosts of the shared widget were not measured in the reference application, so they
//! keep today's 9 px radius.

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, Sense, vec2};
use egui_kittest::Harness;

use crate::state::PointCurveState;
use crate::widgets::point_curve::{self, CURVES_HIT_RADIUS};

/// One screen pixel per input level, the scale the issue measures at: the graph is exactly 255 px
/// on a side, so a screen distance in px and an input distance in levels are the same number.
const SIDE: f32 = 255.0;
/// The radius of the hosts that stay as they are today.
const TODAY: f32 = 9.0;

#[derive(Clone, Debug)]
struct Host {
    pts: Vec<[f32; 2]>,
    gesture: PointCurveState,
    graph: Rect,
    radius: f32,
}

/// A bare point-curve graph of `radius` grab pixels: identity curve, one point at level 128.
fn graph(radius: f32) -> Harness<'static, Host> {
    let mut h = Harness::builder().with_size(vec2(600.0, 600.0)).build_ui_state(
        |ui, host: &mut Host| {
            let (rect, resp) = ui.allocate_exact_size(vec2(SIDE, SIDE), Sense::click_and_drag());
            host.graph = rect;
            point_curve::interact_with_hit_radius(ui, &resp, rect, &mut host.pts, &mut host.gesture, host.radius);
        },
        Host { pts: vec![[0.0, 0.0], [128.0, 128.0], [255.0, 255.0]], gesture: PointCurveState::default(), graph: Rect::NOTHING, radius },
    );
    h.run_steps(2); // The first frame lays the graph out.
    h
}

/// Screen position of the input/output level pair `level` (1 px per level).
fn at(host: &Host, level: [f32; 2]) -> Pos2 {
    Pos2::new(host.graph.left() + level[0], host.graph.bottom() - level[1])
}

/// Press and release at `pos`, a frame each: one click.
fn click(h: &mut Harness<'static, Host>, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    h.run_steps(1);
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// A click `d` px right of the middle point.
fn click_right_of_point(h: &mut Harness<'static, Host>, d: f32) {
    let pos = at(h.state(), [128.0 + d, 128.0]);
    click(h, pos);
}

/// The measured case: at 1 px per level a click 8, 13 or 15 px from a point selects it and adds
/// nothing. The point does not move either: the press keeps the grab offset through the release.
#[test]
fn curves_selects_a_point_clicked_up_to_fifteen_pixels_away() {
    for d in [8.0, 13.0, 15.0] {
        let mut h = graph(CURVES_HIT_RADIUS);
        click_right_of_point(&mut h, d);
        let host = h.state();
        assert_eq!(host.pts, vec![[0.0, 0.0], [128.0, 128.0], [255.0, 255.0]], "a click {d} px away adds a point next to the existing one");
        assert_eq!(host.gesture.selected, Some(1), "a click {d} px away does not select the existing point");
    }
}

/// The same 15 px measured diagonally (9 right, 12 up): the radius is a screen disc.
#[test]
fn curves_selects_the_point_across_a_diagonal_click() {
    let mut h = graph(CURVES_HIT_RADIUS);
    let pos = at(h.state(), [137.0, 140.0]);
    click(&mut h, pos);
    let host = h.state();
    assert_eq!(host.pts, vec![[0.0, 0.0], [128.0, 128.0], [255.0, 255.0]], "the 15 px diagonal click adds a point");
    assert_eq!(host.gesture.selected, Some(1), "the 15 px diagonal click does not select the point");
}

/// A click clearly away from every point still adds a point there and selects it.
#[test]
fn curves_still_adds_a_point_clicked_thirty_pixels_away() {
    let mut h = graph(CURVES_HIT_RADIUS);
    click_right_of_point(&mut h, 30.0);
    let host = h.state();
    assert_eq!(host.pts, vec![[0.0, 0.0], [128.0, 128.0], [158.0, 128.0], [255.0, 255.0]], "the 30 px click adds no new point");
    assert_eq!(host.gesture.selected, Some(2), "the added point is the selected one");
}

/// The radius has to reach the measured 15 px select and stay clear of the 30 px add.
#[test]
fn the_curves_hit_radius_sits_inside_the_measured_window() {
    assert!((15.0..30.0).contains(&CURVES_HIT_RADIUS), "the Curves grab radius is {CURVES_HIT_RADIUS} px, outside the measured 15..30 px window");
}

/// Not measured in the reference application, so the other hosts keep today's radius exactly: a
/// click 8 or 9 px away grabs the point, one 10 px away still adds a second point.
#[test]
fn the_unmeasured_hosts_keep_todays_nine_pixel_radius() {
    for (d, added) in [(8.0, false), (9.0, false), (10.0, true)] {
        let mut h = graph(TODAY);
        click_right_of_point(&mut h, d);
        let host = h.state();
        assert_eq!(host.pts.len(), if added { 4 } else { 3 }, "a click {d} px away through the default radius changes the point count");
        assert_eq!(host.gesture.selected, Some(if added { 2 } else { 1 }), "a click {d} px away selects the wrong point");
    }
}

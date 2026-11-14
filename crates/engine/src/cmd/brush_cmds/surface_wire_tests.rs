//! Surface Color and Follow Perspective through the commands, the live preview and the cursor.

use super::*;
use crate::vp_cmds::VP;
use photocraft_paint::surface::SurfaceMap;
use std::sync::Arc;

const FLOOR: [[f64; 2]; 4] = [[120.0, 120.0], [280.0, 120.0], [360.0, 280.0], [40.0, 280.0]];

/// A transparent document whose first layer is striped (dark and light columns), with an empty
/// layer on top, active.
fn striped(w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "transparent"})).unwrap();
    s.edit("stripes", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        for x in 0..w as i32 {
            let v = if x % 2 == 0 { 0.3 } else { 0.7 };
            surf.fill_rect(Rect::new(x, 0, x + 1, h as i32), &[v, v, v, 1.0]);
        }
        Ok(())
    })
    .unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

fn layer_surface(s: &Session) -> Surface {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().clone()
}

fn same(a: &Surface, b: &Surface, r: Rect) -> bool {
    (r.y0..r.y1).all(|y| (r.x0..r.x1).all(|x| a.pixel(x, y) == b.pixel(x, y)))
}

fn stroke(color: Value) -> Value {
    json!({"points": [[20, 50], [180, 50]], "color": "#808080", "target": "pixels", "brush": {"size": 40, "hardness": 1, "surface": color}})
}

#[test]
fn surface_color_shows_the_backgrounds_stripes_in_the_paint() {
    let mut s = striped(200, 100);
    s.execute("paint.stroke", stroke(json!({"color": true, "detail": 24}))).unwrap();
    let (dark, light) = (layer_surface(&s).rgba(100, 50), layer_surface(&s).rgba(101, 50));
    assert!(dark[0] < 0.4 && light[0] > 0.6, "{dark:?} {light:?}");
    assert!((dark[3] - 1.0).abs() < 0.01 && (light[3] - 1.0).abs() < 0.01);
}

#[test]
fn strength_zero_and_the_empty_current_layer_paint_plain() {
    let mut plain = striped(200, 100);
    plain.execute("paint.stroke", stroke(json!({}))).unwrap();
    let area = Rect::new(0, 0, 200, 100);
    for opts in [json!({"color": true, "strength": 0}), json!({"color": true, "sample": "currentLayer"}), json!({"color": false, "strength": 1})] {
        let mut s = striped(200, 100);
        s.execute("paint.stroke", stroke(opts.clone())).unwrap();
        assert!(same(&layer_surface(&s), &layer_surface(&plain), area), "{opts}");
    }
    // Half strength is in between.
    let mut half = striped(200, 100);
    half.execute("paint.stroke", stroke(json!({"color": true, "strength": 0.5}))).unwrap();
    let (d, l) = (layer_surface(&half).rgba(100, 50)[0], layer_surface(&half).rgba(101, 50)[0]);
    assert!(d < 0.5 && d > 0.35 && l > 0.5 && l < 0.65, "{d} {l}");
}

#[test]
fn the_eraser_and_other_targets_are_not_shaded() {
    let mut s = striped(200, 100);
    s.execute("paint.stroke", json!({"points": [[20, 50], [180, 50]], "color": "#808080", "target": "pixels", "brush": {"size": 40}})).unwrap();
    let mut shaded = striped(200, 100);
    shaded.execute("paint.stroke", json!({"points": [[20, 50], [180, 50]], "color": "#808080", "target": "pixels", "brush": {"size": 40}})).unwrap();
    let erase = |s: &mut Session, surface: Value| {
        s.execute("paint.stroke", json!({"points": [[20, 50], [180, 50]], "erase": true, "target": "pixels", "brush": {"size": 20, "surface": surface}}))
            .unwrap()
    };
    erase(&mut s, json!({}));
    erase(&mut shaded, json!({"color": true}));
    assert!(same(&layer_surface(&s), &layer_surface(&shaded), Rect::new(0, 0, 200, 100)));
}

#[test]
fn a_live_stroke_with_surface_color_commits_the_pixels_it_shows() {
    let p = json!({"points": [[20, 50]], "color": "#c04020", "seed": 7, "target": "pixels",
        "brush": {"size": 30, "surface": {"color": true, "strength": 0.8, "detail": 8}}});
    let mut s = striped(200, 100);
    let mut live = LiveStroke::begin_with(&s, "paint.stroke", &p).unwrap();
    live.push(&[StrokePoint::new(80.0, 40.0, 1.0), StrokePoint::new(150.0, 60.0, 1.0)]).unwrap();
    let mut commit = p.clone();
    commit["points"] = json!([[20, 50], [80, 40], [150, 60]]);
    commit["seed"] = json!(live.seed);
    let id = s.active().unwrap().active_layer.unwrap();
    let shown = live.doc.layer(id).unwrap().surface().unwrap().clone();
    s.offer_live_stroke(live).unwrap();
    let r = s.execute("paint.stroke", commit.clone()).unwrap();
    s.clear_live_stroke();
    assert_eq!(r["fromLive"], true);
    let area = Rect::new(0, 0, 200, 100);
    assert!(same(&shown, &layer_surface(&s), area), "the preview shows what commits");
    let mut fresh = striped(200, 100);
    fresh.execute("paint.stroke", commit).unwrap();
    assert!(same(&layer_surface(&fresh), &layer_surface(&s), area), "a replay renders the same pixels");
    // The shading is really there.
    assert!((layer_surface(&s).rgba(50, 44)[0] - layer_surface(&s).rgba(51, 44)[0]).abs() > 0.1);
}

#[test]
fn follow_perspective_needs_saved_planes() {
    let mut s = striped(400, 300);
    let p = json!({"points": [[200, 270], [200, 140]], "target": "pixels", "brush": {"size": 30, "surface": {"perspective": true}}});
    let e = s.execute("paint.stroke", p.clone()).unwrap_err().to_string();
    assert!(e.contains("no perspective planes"), "{e}");
    assert!(layer_surface(&s).content_bounds().is_empty());
    assert!(LiveStroke::begin_with(&s, "paint.stroke", &p).is_err());
    // Planes saved for another canvas size do not count either.
    s.execute("perspective.setPlanes", json!({"planes": [FLOOR]})).unwrap();
    s.execute("image.canvasSize", json!({"width": 500, "height": 300, "anchor": "topLeft"})).unwrap();
    assert!(s.execute("paint.stroke", p).is_err());
}

/// Painted pixels across row `y` of the active layer.
fn row_width(s: &Session, y: i32, w: i32) -> usize {
    let surf = layer_surface(s);
    (0..w).filter(|x| surf.rgba(*x, y)[3] > 0.5).count()
}

#[test]
fn dabs_shrink_towards_the_far_edge_of_a_plane() {
    let mut s = striped(400, 300);
    s.execute("perspective.setPlanes", json!({"planes": [FLOOR]})).unwrap();
    let p = json!({"points": [[200, 270], [200, 140]], "target": "pixels", "brush": {"size": 30, "hardness": 1, "surface": {"perspective": true}}});
    let mut plain = striped(400, 300);
    plain.execute("paint.stroke", json!({"points": [[200, 270], [200, 140]], "target": "pixels", "brush": {"size": 30, "hardness": 1}})).unwrap();
    s.execute("paint.stroke", p.clone()).unwrap();
    let (near, far) = (row_width(&s, 265, 400), row_width(&s, 150, 400));
    assert!(near > 20 && far > 0 && (far as f64) < near as f64 * 0.8, "near {near}, far {far}");
    // Without the option the stroke is a plain 30 px line.
    assert_eq!(row_width(&plain, 265, 400), row_width(&plain, 150, 400));
    // The first dab keeps the brush size where the stroke starts.
    assert!((near as f64 - 30.0).abs() <= 3.0, "{near}");
    // A live preview commits the same pixels.
    let live = LiveStroke::begin_with(&striped_with_planes(), "paint.stroke", &p).unwrap();
    let mut fresh = striped_with_planes();
    fresh.execute("paint.stroke", p).unwrap();
    let id = fresh.active().unwrap().active_layer.unwrap();
    assert!(same(live.doc.layer(id).unwrap().surface().unwrap(), &layer_surface(&fresh), Rect::new(0, 0, 400, 300)));
}

fn striped_with_planes() -> Session {
    let mut s = striped(400, 300);
    s.execute("perspective.setPlanes", json!({"planes": [FLOOR]})).unwrap();
    s
}

#[test]
fn the_cursor_shape_is_the_first_dab_and_absent_off_plane() {
    let mut s = striped_with_planes();
    // Off by default.
    assert!(brush_cursor_shape(&mut s, 200.0, 270.0, 15.0).is_none());
    s.execute("tools.setBrush", json!({"size": 30, "surface": {"perspective": true}})).unwrap();
    assert!(brush_cursor_shape(&mut s, 10.0, 10.0, 15.0).is_none(), "off every plane");
    assert!(brush_cursor_shape(&mut s, f64::NAN, 10.0, 15.0).is_none());
    for (x, y) in [(200.0, 270.0), (200.0, 140.0), (60.0, 270.0)] {
        let (r, a, k) = brush_cursor_shape(&mut s, x, y, 15.0).unwrap();
        // The first dab of a stroke that starts there.
        let map = super::surface_wire::plane_map(&s).unwrap() as Arc<dyn SurfaceMap>;
        let brush = BrushSettings { size: 30.0, ..BrushSettings::default() };
        let mut g = photocraft_paint::dynamics::DabGenerator::new(&brush, 1.0).with_surface(Some(map));
        let (mut dabs, mut dual) = (Vec::new(), Vec::new());
        g.push(&[StrokePoint::new(x, y, 1.0)], &mut dabs, &mut dual);
        g.finish(&mut dabs, &mut dual);
        let d = dabs.first().unwrap();
        assert_eq!((d.radius, d.proj_angle, d.proj_scale), (r, a, k), "({x}, {y})");
        assert!(k < 1.0 && k >= 0.05);
    }
    // No planes: nothing.
    s.execute("perspective.clearPlanes", json!({})).unwrap();
    assert!(brush_cursor_shape(&mut s, 200.0, 270.0, 15.0).is_none());
}

#[test]
fn the_cursor_shape_command_reads_and_fails_gracefully() {
    let mut s = striped_with_planes();
    s.execute("tools.setBrush", json!({"size": 30, "surface": {"perspective": true}})).unwrap();
    let journal = s.journal.len();
    let r = s.execute("brush.cursorShape", json!({"x": 200, "y": 270})).unwrap();
    assert_eq!(r["onPlane"], true);
    assert_eq!(r["radius"].as_f64(), Some(15.0));
    assert!(r["projScale"].as_f64().unwrap() < 1.0);
    assert!(r["projAngle"].is_number());
    assert_eq!(s.execute("brush.cursorShape", json!({"x": 5, "y": 5})).unwrap(), json!({"onPlane": false}));
    assert_eq!(s.journal.len(), journal, "a query is not journaled");
    for bad in [json!({}), json!({"x": 1}), json!({"x": "a", "y": 1}), json!({"x": null, "y": 2}), json!([1, 2]), json!("x")] {
        assert!(s.execute("brush.cursorShape", bad.clone()).is_err(), "{bad}");
    }
    assert!(Session::new().execute("brush.cursorShape", json!({"x": 1, "y": 1})).is_err());
}

#[test]
fn hostile_surface_params_are_errors_not_strokes() {
    let mut s = striped(100, 100);
    let before = layer_surface(&s);
    for opts in [
        json!({"color": true, "strength": 5}),
        json!({"color": true, "strength": -1}),
        json!({"color": true, "detail": 1e9}),
        json!({"color": true, "detail": 0}),
        json!({"strength": 2}),
        json!({"sample": "nowhere"}),
    ] {
        let p = json!({"points": [[10, 10], [50, 50]], "target": "pixels", "brush": {"size": 10, "surface": opts}});
        assert!(s.execute("paint.stroke", p.clone()).is_err(), "{opts}");
        assert!(s.execute("paint.pencil", p).is_err(), "{opts}");
        assert!(s.execute("tools.setBrush", json!({"surface": opts})).is_err(), "{opts}");
    }
    assert!(same(&before, &layer_surface(&s), Rect::new(0, 0, 100, 100)));
}

/// A document with a striped layer, a layer holding a grey patch to paste, and an empty target.
fn paste_session() -> (Session, photocraft_doc::LayerId, photocraft_doc::LayerId) {
    let mut s = striped(200, 200);
    let target = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let src = s.active().unwrap().active_layer.unwrap();
    s.edit("patch", |doc, _| {
        doc.layer_mut(src).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 40, 40), &[0.5, 0.5, 0.5, 1.0]);
        Ok(())
    })
    .unwrap();
    (s, src, target)
}

fn paste(src: photocraft_doc::LayerId, target: photocraft_doc::LayerId, extra: Value) -> Value {
    let mut item = json!({"plane": 0, "layer": src.0, "at": [0, 0], "width": 1.0});
    for (k, v) in extra.as_object().unwrap() {
        item[k] = v.clone();
    }
    json!({"planes": [{"corners": [[20, 20], [180, 20], [180, 180], [20, 180]]}], "paste": [item], "layer": target.0})
}

fn pasted(s: &Session, id: photocraft_doc::LayerId) -> Surface {
    s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().clone()
}

#[test]
fn vanishing_point_paste_shade_zero_is_todays_output_and_one_shows_the_stripes() {
    let (mut plain, src, target) = paste_session();
    plain.execute(VP, paste(src, target, json!({}))).unwrap();
    let (mut zero, src, target) = paste_session();
    zero.execute(VP, paste(src, target, json!({"shade": 0}))).unwrap();
    let area = Rect::new(0, 0, 200, 200);
    assert!(same(&pasted(&plain, target), &pasted(&zero, target), area));
    let (mut shaded, src, target) = paste_session();
    shaded.execute(VP, paste(src, target, json!({"shade": 1, "shadeDetail": 4}))).unwrap();
    let out = pasted(&shaded, target);
    let (dark, light) = (out.rgba(150, 100), out.rgba(151, 100));
    assert!(dark[0] < 0.4 && light[0] > 0.6, "{dark:?} {light:?}");
    // The plain paste is flat there.
    let flat = pasted(&plain, target);
    assert!((flat.rgba(150, 100)[0] - flat.rgba(151, 100)[0]).abs() < 0.01);
}

#[test]
fn vanishing_point_paste_rejects_hostile_shade_params() {
    let (mut s, src, target) = paste_session();
    let before = pasted(&s, target);
    let steps = s.active().unwrap().history.past_len();
    for extra in [
        json!({"shade": 2}),
        json!({"shade": -0.1}),
        json!({"shade": "x"}),
        json!({"shade": 1, "shadeDetail": 1e9}),
        json!({"shade": 1, "shadeDetail": 0}),
        json!({"shade": 1, "shadeDetail": "wide"}),
    ] {
        assert!(s.execute(VP, paste(src, target, extra.clone())).is_err(), "{extra}");
    }
    assert!(same(&before, &pasted(&s, target), Rect::new(0, 0, 200, 200)));
    assert_eq!(s.active().unwrap().history.past_len(), steps);
}

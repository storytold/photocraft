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
    let (pn, pf) = (row_width(&plain, 265, 400), row_width(&plain, 150, 400));
    assert!(pn.abs_diff(pf) <= 3, "plain {pn} vs {pf}");
    // The brush size is the size on the first plane's centre, wherever the stroke starts (checked
    // on the dabs: the painted width also has the edge's anti-aliasing and the dab spacing in it).
    let map = super::surface_wire::plane_map(&s).unwrap() as Arc<dyn SurfaceMap>;
    let brush = BrushSettings { size: 30.0, ..BrushSettings::default() };
    let mut g = photocraft_paint::dynamics::DabGenerator::new(&brush, 1.0).with_surface(Some(map));
    let (mut dabs, mut dual) = (Vec::new(), Vec::new());
    g.push(&[StrokePoint::new(200.0, 270.0, 1.0), StrokePoint::new(200.0, 140.0, 1.0)], &mut dabs, &mut dual);
    g.finish(&mut dabs, &mut dual);
    let reference = super::surface_wire::plane_map(&s).unwrap().reference_scale().unwrap();
    let at_start = super::surface_wire::plane_map(&s).unwrap().local(photocraft_geom::Point::new(200.0, 270.0)).unwrap().scale;
    let first = dabs.first().unwrap().radius;
    assert!((first - 15.0 * at_start / reference).abs() < 1e-3, "{first}");
    assert!(dabs.last().is_some_and(|d| d.radius < first * 0.8));
    // A stroke starting at the far edge starts small, not at the brush size.
    let far_start = dabs_with(&brush, super::surface_wire::plane_map(&s).unwrap(), &[StrokePoint::new(200.0, 140.0, 1.0)]);
    assert!(far_start.first().is_some_and(|d| d.radius < first * 0.8 && d.radius < 15.0), "{:?}", far_start.first().map(|d| d.radius));
    // A live preview commits the same pixels.
    // (Layer ids differ from one session to the next: each side reads its own layer.)
    let preview = striped_with_planes();
    let live = LiveStroke::begin_with(&preview, "paint.stroke", &p).unwrap();
    let live_id = preview.active().unwrap().active_layer.unwrap();
    let mut fresh = striped_with_planes();
    fresh.execute("paint.stroke", p).unwrap();
    assert!(same(live.doc.layer(live_id).unwrap().surface().unwrap(), &layer_surface(&fresh), Rect::new(0, 0, 400, 300)));
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
        let c = brush_cursor_shape(&mut s, x, y, 15.0).unwrap();
        let (r, a, k) = (c.radius, c.proj_angle, c.proj_scale);
        assert_eq!(c.offset, [0.0, 0.0]);
        // The first dab of a stroke that starts there.
        let map = super::surface_wire::plane_map(&s).unwrap() as Arc<dyn SurfaceMap>;
        let brush = BrushSettings { size: 30.0, ..BrushSettings::default() };
        let mut g = photocraft_paint::dynamics::DabGenerator::new(&brush, 1.0).with_surface(Some(map));
        let (mut dabs, mut dual) = (Vec::new(), Vec::new());
        g.push(&[StrokePoint::new(x, y, 1.0)], &mut dabs, &mut dual);
        g.finish(&mut dabs, &mut dual);
        let d = dabs.first().unwrap();
        assert_eq!((d.radius, d.proj_angle, d.proj_scale), (r, a, k), "({x}, {y})");
        assert!((0.05..1.0).contains(&k));
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
    // Nearer than the first plane's centre: larger than the brush size.
    assert!(r["radius"].as_f64().unwrap() > 15.0, "{r}");
    assert!(r["outline"].is_null(), "planes alone keep the ellipse");
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

/// The target layer's pixels after a Vanishing Point paste with `extra` on a fresh document. Layer
/// ids are not the same from one session to the next, so each run keeps its own.
fn paste_result(extra: Value) -> Surface {
    let (mut s, src, target) = paste_session();
    s.execute(VP, paste(src, target, extra)).unwrap();
    pasted(&s, target)
}

#[test]
fn vanishing_point_paste_shade_zero_is_todays_output_and_one_shows_the_stripes() {
    let plain = paste_result(json!({}));
    assert!(plain.content_bounds().width() > 100, "the paste landed: {:?}", plain.content_bounds());
    let zero = paste_result(json!({"shade": 0}));
    assert!(same(&plain, &zero, Rect::new(0, 0, 200, 200)));
    let shaded = paste_result(json!({"shade": 1, "shadeDetail": 4}));
    let (dark, light) = (shaded.rgba(150, 100), shaded.rgba(151, 100));
    assert!(dark[0] < 0.4 && light[0] > 0.6, "{dark:?} {light:?}");
    // The plain paste is flat there.
    assert!((plain.rgba(150, 100)[0] - plain.rgba(151, 100)[0]).abs() < 0.01);
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

// ---- Follow Depth ----

/// A document whose picture is a sinusoidal fold (columns of brightness `0.5 + 0.3 sin(2πx/16)`),
/// with an empty layer on top: a photo for Surface Color.
fn folds_doc(w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "transparent", "depth": 32})).unwrap();
    s.edit("folds", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        for x in 0..w as i32 {
            let v = 0.5 + 0.3 * (std::f64::consts::TAU * f64::from(x) / 16.0).sin() as f32;
            surf.fill_rect(Rect::new(x, 0, x + 1, h as i32), &[v, v, v, 1.0]);
        }
        Ok(())
    })
    .unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

/// A document with an empty layer and an alpha channel `name` holding `depth(x, y)`.
fn depth_doc(w: u32, h: u32, name: &str, depth: impl Fn(i32, i32) -> f32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "transparent", "depth": 32})).unwrap();
    s.edit("depth channel", |doc, _| {
        let fmt = photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, doc.depth, false);
        let mut surf = Surface::new(fmt);
        let data: Vec<f32> = (0..h as i32).flat_map(|y| (0..w as i32).map(move |x| (x, y))).map(|(x, y)| depth(x, y)).collect();
        surf.write_region(Rect::new(0, 0, w as i32, h as i32), &data);
        doc.channels.push(photocraft_doc::AlphaChannel::new(name.to_string(), surf));
        Ok(())
    })
    .unwrap();
    s
}

/// Depth rising from 0 at the left to 1 at the right edge of a 200 px wide image.
fn ramp_doc() -> Session {
    depth_doc(200, 100, "Depth", |x, _| x as f32 / 200.0)
}

fn depth_brush(extra: Value) -> Value {
    let mut surface = json!({"depth": true, "relief": 1.0, "sizeByDepth": false});
    for (k, v) in extra.as_object().unwrap() {
        surface[k] = v.clone();
    }
    json!({"size": 12, "surface": surface})
}

fn dabs_with(brush: &BrushSettings, map: Arc<dyn SurfaceMap>, pts: &[StrokePoint]) -> Vec<photocraft_paint::Dab> {
    let mut g = photocraft_paint::dynamics::DabGenerator::new(brush, 1.0).with_surface(Some(map));
    let (mut dabs, mut dual) = (Vec::new(), Vec::new());
    g.push(pts, &mut dabs, &mut dual);
    g.finish(&mut dabs, &mut dual);
    dabs
}

/// The stroke's own wiring map (built, and cached, from the session brush).
fn wiring_map(s: &Session) -> Arc<dyn SurfaceMap> {
    let layer = s.active().unwrap().active_layer;
    let wiring = super::surface_wire::Wiring::build(s, "test", &json!({}), &s.tools.brush, layer).unwrap();
    wiring.map().unwrap().clone()
}

/// The first dab of a stroke starting at `(x, y)` under the stroke's own wiring.
fn first_dab(s: &Session, x: f64, y: f64) -> Option<photocraft_paint::Dab> {
    let layer = s.active().unwrap().active_layer;
    let wiring = super::surface_wire::Wiring::build(s, "test", &json!({}), &s.tools.brush, layer).unwrap();
    let map = wiring.map()?.clone();
    dabs_with(&s.tools.brush, map, &[StrokePoint::new(x, y, 1.0)]).first().copied()
}

#[test]
fn dabs_on_a_depth_slope_are_squashed_along_it_and_flat_ones_are_not() {
    let mut s = ramp_doc();
    s.execute("tools.setBrush", depth_brush(json!({}))).unwrap();
    let d = first_dab(&s, 100.0, 50.0).unwrap();
    assert_eq!(d.radius, 6.0, "no size by depth: the brush size");
    assert!(d.proj_scale < 0.8, "{}", d.proj_scale);
    assert!(d.proj_angle.cos().abs() > 0.99, "squashed along the ramp (x): {}", d.proj_angle);
    // Relief 0 never squashes (and with no depth scale there is nothing to follow).
    s.execute("tools.setBrush", json!({"surface": {"relief": 0}})).unwrap();
    assert!(first_dab(&s, 100.0, 50.0).is_none_or(|d| d.proj_scale == 1.0));
    // A flat depth plane is not squashed.
    let mut flat = depth_doc(200, 100, "Depth", |_, _| 0.6);
    flat.execute("tools.setBrush", depth_brush(json!({}))).unwrap();
    assert!(first_dab(&flat, 100.0, 50.0).is_none_or(|d| d.proj_scale == 1.0 && d.radius == 6.0));
}

#[test]
fn size_by_depth_is_anchored_at_the_middle_distance_for_every_stroke() {
    let mut s = ramp_doc();
    s.execute("tools.setBrush", depth_brush(json!({"sizeByDepth": true, "relief": 0}))).unwrap();
    let brush = s.tools.brush.clone();
    let map = wiring_map(&s);
    // The median depth is 0.5: a dab at D = 0.5 is the brush size.
    let dabs = dabs_with(&brush, map, &[StrokePoint::new(100.0, 50.0, 1.0), StrokePoint::new(30.0, 50.0, 1.0)]);
    assert!((dabs.first().unwrap().radius - 6.0).abs() < 0.2, "{:?}", dabs.first().map(|d| d.radius));
    assert!(dabs.windows(2).all(|w| w[1].radius <= w[0].radius + 1e-6), "dabs only shrink towards low D");
    // D falls from 0.5 to 0.15 over the stroke: about 0.3 of the size.
    let last = dabs.last().unwrap().radius;
    assert!(last < 3.0 && last > 1.0, "{last}");
    // A stroke that starts at the far side starts small, not at the brush size.
    let far = dabs_with(&brush, wiring_map(&s), &[StrokePoint::new(30.0, 50.0, 1.0), StrokePoint::new(190.0, 50.0, 1.0)]);
    assert!(far.first().unwrap().radius < 2.5, "{:?}", far.first().map(|d| d.radius));
    assert!(far.last().unwrap().radius > far.first().unwrap().radius * 4.0);
}

fn assert_cursor_is_first_dab(s: &mut Session, x: f64, y: f64) {
    let radius = (s.tools.brush.size / 2.0).max(0.5);
    // The stroke's wiring builds (and caches) the geometry; the cursor only reads it.
    let dab = first_dab(s, x, y);
    let cursor = brush_cursor_shape(s, x, y, radius);
    match (cursor, dab) {
        (Some(c), Some(d)) => {
            assert_eq!((c.radius, c.proj_angle, c.proj_scale), (d.radius, d.proj_angle, d.proj_scale), "({x}, {y})");
            assert_eq!(c.offset, [0.0, 0.0]);
            assert_eq!((d.center.x, d.center.y), (x, y));
        }
        (None, d) => assert!(d.is_none_or(|d| d.proj_scale == 1.0 && d.center.x == x && d.center.y == y), "({x}, {y}) {d:?}"),
        (c, None) => panic!("cursor {c:?} but no dab at ({x}, {y})"),
    }
}

#[test]
fn the_cursor_is_the_first_dab_with_depth_and_with_both() {
    let mut s = depth_doc(400, 300, "Depth", |x, y| (x as f32 / 400.0) * 0.7 + (y as f32 / 300.0) * 0.3);
    s.execute("tools.setBrush", depth_brush(json!({"sizeByDepth": true}))).unwrap();
    let mut shaped = 0;
    for (x, y) in [(40.0, 50.0), (200.0, 150.0), (350.0, 280.0), (101.0, 200.0)] {
        assert_cursor_is_first_dab(&mut s, x, y);
        shaped += brush_cursor_shape(&mut s, x, y, 6.0).is_some_and(|c| c.proj_scale < 1.0) as u32;
    }
    assert!(shaped >= 3, "the slope shows");
    // With planes as well: on the plane both apply, off it nothing does.
    s.execute("perspective.setPlanes", json!({"planes": [FLOOR]})).unwrap();
    s.execute("tools.setBrush", json!({"surface": {"perspective": true}})).unwrap();
    for (x, y) in [(200.0, 270.0), (200.0, 200.0), (100.0, 250.0), (60.0, 270.0)] {
        assert_cursor_is_first_dab(&mut s, x, y);
    }
    assert!(brush_cursor_shape(&mut s, 10.0, 10.0, 6.0).is_none(), "off the plane");
    let both = brush_cursor_shape(&mut s, 202.0, 250.0, 6.0).unwrap();
    s.execute("tools.setBrush", json!({"surface": {"depth": false}})).unwrap();
    let plane = brush_cursor_shape(&mut s, 202.0, 250.0, 6.0).unwrap();
    assert!(both.proj_scale != plane.proj_scale || both.proj_angle != plane.proj_angle, "{both:?} {plane:?}");
}

fn depth_stroke(s: &mut Session, y: i32) -> Result<Value> {
    s.execute("paint.stroke", json!({"points": [[40, y], [160, y]], "color": "#c02020", "target": "pixels", "brush": depth_brush(json!({}))}))
}

#[test]
fn the_depth_cursor_builds_on_the_first_hover_and_a_stroke_reuses_it() {
    let mut s = ramp_doc();
    s.execute("tools.setBrush", depth_brush(json!({"sizeByDepth": true}))).unwrap();
    assert_eq!(s.surface_cache.depth_builds(), 0);
    // No stroke yet: the first hover gives the depth-derived size (median depth 0.5, radius 6).
    let far = brush_cursor_shape(&mut s, 30.0, 50.0, 6.0).expect("hover builds the depth geometry");
    assert_eq!(s.surface_cache.depth_builds(), 1);
    let (mid, near) = (brush_cursor_shape(&mut s, 100.0, 50.0, 6.0).unwrap(), brush_cursor_shape(&mut s, 170.0, 50.0, 6.0).unwrap());
    assert_eq!(s.surface_cache.depth_builds(), 1, "kept for the next hovers");
    assert!(far.radius < mid.radius && mid.radius < near.radius, "{} {} {}", far.radius, mid.radius, near.radius);
    assert!((mid.radius - 6.0).abs() < 0.2 && far.radius < 3.0 && near.radius > 7.5);
    depth_stroke(&mut s, 50).unwrap();
    assert_eq!(s.surface_cache.depth_builds(), 1, "the stroke reuses what the hover built");
    let r = s.execute("brush.cursorShape", json!({"x": 100, "y": 50})).unwrap();
    assert_eq!(r["onPlane"], true);
    assert_eq!(r["offset"], json!([0.0, 0.0]));
    assert_eq!(r["outline"].as_array().map(Vec::len), Some(32));
    // Without a depth channel the outline stays plain and nothing is built.
    let mut none = striped(100, 100);
    none.execute("tools.setBrush", depth_brush(json!({}))).unwrap();
    assert!(brush_cursor_shape(&mut none, 50.0, 50.0, 6.0).is_none());
    assert_eq!(none.surface_cache.depth_builds(), 0);
}

/// The dab of a stroke running from `from` to `to` that lies nearest to `to`.
fn dab_near(s: &Session, from: (f64, f64), to: (f64, f64)) -> photocraft_paint::Dab {
    let mut brush = s.tools.brush.clone();
    brush.spacing = 0.01;
    let dabs = dabs_with(&brush, wiring_map(s), &[StrokePoint::new(from.0, from.1, 1.0), StrokePoint::new(to.0, to.1, 1.0)]);
    dabs.into_iter().min_by(|a, b| (a.center.x - to.0).hypot(a.center.y - to.1).total_cmp(&(b.center.x - to.0).hypot(b.center.y - to.1))).unwrap()
}

#[test]
fn the_cursor_equals_the_dab_a_stroke_from_elsewhere_lays_down_there() {
    let mut s = depth_doc(400, 300, "Depth", |x, y| (x as f32 / 400.0) * 0.7 + (y as f32 / 300.0) * 0.3);
    s.execute("tools.setBrush", depth_brush(json!({"sizeByDepth": true}))).unwrap();
    for (from, to) in [((350.0, 250.0), (60.0, 40.0)), ((40.0, 40.0), (330.0, 240.0)), ((200.0, 150.0), (210.0, 150.0))] {
        let dab = dab_near(&s, from, to);
        let cursor = brush_cursor_shape(&mut s, dab.center.x, dab.center.y, 6.0).unwrap();
        assert!((cursor.radius - dab.radius).abs() < 1e-3 * dab.radius.max(1.0), "{from:?} -> {to:?}: {} vs {}", cursor.radius, dab.radius);
        let foot = dab.footprint.expect("a depth dab carries its footprint");
        let outline = cursor.outline.expect("and so does the cursor");
        assert!(foot.rays.iter().zip(&outline).all(|(a, b)| (a - b).abs() < 1e-3), "{:?} vs {outline:?}", foot.rays);
    }
    // With the plane as well the dab's size is the plane's times the depth's, and still equal.
    s.execute("perspective.setPlanes", json!({"planes": [FLOOR]})).unwrap();
    s.execute("tools.setBrush", json!({"surface": {"perspective": true}})).unwrap();
    for (from, to) in [((200.0, 270.0), (200.0, 160.0)), ((100.0, 150.0), (300.0, 270.0))] {
        let dab = dab_near(&s, from, to);
        let cursor = brush_cursor_shape(&mut s, dab.center.x, dab.center.y, 6.0).unwrap();
        assert!((cursor.radius - dab.radius).abs() < 1e-3 * dab.radius.max(1.0), "{} vs {}", cursor.radius, dab.radius);
    }
}

#[test]
fn a_ridge_bends_the_footprint_and_flat_depth_keeps_it_round() {
    // A sharp ridge at x = 100 (a gentle climb, a steep drop), then flat.
    let mut s = depth_doc(240, 100, "Depth", |x, _| if x < 100 { 0.5 * x as f32 / 100.0 } else { (0.5 - 0.5 * (x as f32 - 100.0) / 20.0).max(0.0) });
    s.execute("tools.setBrush", json!({"size": 40, "surface": {"depth": true, "relief": 1.0, "sizeByDepth": false}})).unwrap();
    let c = brush_cursor_shape(&mut s, 92.0, 50.0, 20.0).unwrap();
    let rays = c.outline.unwrap();
    let across = rays[8].min(rays[24]);
    assert!(rays[0] < across * 0.8 && rays[0] < rays[16] - 0.3, "east {} west {} across {across}", rays[0], rays[16]);
    // Flat depth with sizeByDepth: a perfect circle.
    let mut flat = depth_doc(100, 100, "Depth", |_, _| 0.6);
    flat.execute("tools.setBrush", json!({"size": 20, "surface": {"depth": true, "relief": 1.0, "sizeByDepth": true}})).unwrap();
    let f = brush_cursor_shape(&mut flat, 50.0, 50.0, 10.0).unwrap();
    assert!(f.outline.unwrap().iter().all(|r| (r - f.radius).abs() < 1e-4));
}

#[test]
fn painting_keeps_the_depth_cache_and_editing_the_channel_rebuilds_it() {
    let mut s = ramp_doc();
    depth_stroke(&mut s, 30).unwrap();
    depth_stroke(&mut s, 60).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    depth_stroke(&mut s, 80).unwrap();
    assert_eq!(s.surface_cache.depth_builds(), 1, "painting and adding layers keep the geometry");
    s.edit("edit the depth", |doc, _| {
        let ch = doc.channels.iter_mut().find(|c| c.name == "Depth").unwrap();
        ch.surface.fill_rect(Rect::new(0, 0, 50, 50), &[1.0]);
        Ok(())
    })
    .unwrap();
    depth_stroke(&mut s, 90).unwrap();
    assert_eq!(s.surface_cache.depth_builds(), 2);
}

#[test]
fn a_missing_depth_channel_is_an_error() {
    let mut s = striped(200, 100);
    let p = json!({"points": [[10, 10], [50, 50]], "target": "pixels", "brush": depth_brush(json!({}))});
    let e = s.execute("paint.stroke", p.clone()).unwrap_err().to_string();
    assert!(e.contains("no Depth channel: run Image"), "{e}");
    assert!(LiveStroke::begin_with(&s, "paint.stroke", &p).is_err());
    assert!(layer_surface(&s).content_bounds().is_empty());
    // Another name is looked up by that name.
    let mut s = ramp_doc();
    let other = json!({"points": [[10, 10], [50, 50]], "target": "pixels", "brush": depth_brush(json!({"depthChannel": "Relief"}))});
    assert!(s.execute("paint.stroke", other).unwrap_err().to_string().contains("no Relief channel"));
}

#[test]
fn hostile_depth_params_are_errors() {
    let mut s = ramp_doc();
    for opts in [
        json!({"relief": 2}),
        json!({"relief": -1}),
        json!({"depthChannel": ""}),
        json!({"depthChannel": "x".repeat(300)}),
        json!({"depth": "yes"}),
        json!({"sizeByDepth": "no"}),
    ] {
        let p = json!({"points": [[10, 10], [50, 50]], "target": "pixels", "brush": depth_brush(opts.clone())});
        assert!(s.execute("paint.stroke", p.clone()).is_err(), "{opts}");
        assert!(s.execute("paint.pencil", p).is_err(), "{opts}");
        assert!(s.execute("tools.setBrush", json!({"surface": opts})).is_err(), "{opts}");
    }
    assert!(layer_surface(&s).content_bounds().is_empty());
}

#[test]
fn a_live_stroke_with_depth_commits_the_pixels_it_shows() {
    let p = json!({"points": [[20, 50]], "color": "#c04020", "seed": 3, "target": "pixels", "brush": depth_brush(json!({"sizeByDepth": true}))});
    let mut s = ramp_doc();
    s.execute("layer.new.layer", json!({})).unwrap();
    let mut live = LiveStroke::begin_with(&s, "paint.stroke", &p).unwrap();
    live.push(&[StrokePoint::new(90.0, 40.0, 1.0), StrokePoint::new(170.0, 60.0, 1.0)]).unwrap();
    let mut commit = p.clone();
    commit["points"] = json!([[20, 50], [90, 40], [170, 60]]);
    commit["seed"] = json!(live.seed);
    let id = s.active().unwrap().active_layer.unwrap();
    let shown = live.doc.layer(id).unwrap().surface().unwrap().clone();
    s.offer_live_stroke(live).unwrap();
    assert_eq!(s.execute("paint.stroke", commit.clone()).unwrap()["fromLive"], true);
    s.clear_live_stroke();
    let area = Rect::new(0, 0, 200, 100);
    assert!(same(&shown, &layer_surface(&s), area));
    let mut fresh = ramp_doc();
    fresh.execute("layer.new.layer", json!({})).unwrap();
    fresh.execute("paint.stroke", commit).unwrap();
    assert!(same(&layer_surface(&fresh), &layer_surface(&s), area));
}

// ---- Speed: the cache, the cursor and the sample ----

fn folds_stroke(s: &mut Session, y: i32) {
    s.execute(
        "paint.stroke",
        json!({"points": [[20, y], [150, y]], "color": "#c02020", "target": "pixels",
        "brush": {"size": 12, "surface": {"color": true, "detail": 24}}}),
    )
    .unwrap();
}

#[test]
fn the_shading_cursor_path_never_builds_either() {
    // Surface Color builds on the stroke; hovering with Follow Perspective on reads no field.
    let mut s = folds_doc(200, 100);
    s.execute("tools.setBrush", json!({"size": 12, "surface": {"color": true}})).unwrap();
    for x in [10.0, 36.0, 150.0] {
        assert!(brush_cursor_shape(&mut s, x, 50.0, 6.0).is_none());
    }
    assert_eq!(s.surface_cache.field_builds(), 0);
    folds_stroke(&mut s, 50);
    assert_eq!(s.surface_cache.field_builds(), 1);
}

#[test]
fn a_second_stroke_reuses_the_field_and_an_edit_of_another_layer_invalidates_it() {
    let mut s = folds_doc(200, 100);
    let photo = s.active().unwrap().doc.layers[0].id;
    folds_stroke(&mut s, 30);
    assert_eq!(s.surface_cache.field_builds(), 1);
    // The first stroke changed the paint layer (and the document revision): the field stays.
    folds_stroke(&mut s, 60);
    folds_stroke(&mut s, 80);
    assert_eq!(s.surface_cache.field_builds(), 1, "painting on the target layer keeps the cache");
    // Selection changes do not matter either.
    s.execute("select.all", json!({})).unwrap();
    folds_stroke(&mut s, 70);
    assert_eq!(s.surface_cache.field_builds(), 1);
    // Another layer's pixels do.
    s.edit("touch the photo", |doc, _| {
        doc.layer_mut(photo).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 20, 20), &[0.9, 0.9, 0.9, 1.0]);
        Ok(())
    })
    .unwrap();
    folds_stroke(&mut s, 40);
    assert_eq!(s.surface_cache.field_builds(), 2, "editing another layer rebuilds");
    // A different detail is a different field.
    s.execute("paint.stroke", json!({"points": [[20, 90], [150, 90]], "target": "pixels", "brush": {"size": 12, "surface": {"color": true, "detail": 12}}}))
        .unwrap();
    assert_eq!(s.surface_cache.field_builds(), 3);
}

#[test]
fn all_layers_leaves_out_the_layer_being_painted() {
    let mut s = folds_doc(200, 100);
    let target = s.active().unwrap().active_layer.unwrap();
    let before = s.active().unwrap().doc.clone();
    folds_stroke(&mut s, 50);
    let after = s.active().unwrap().doc.clone();
    assert!(!layer_surface(&s).content_bounds().is_empty());
    let shade = |doc: &photocraft_doc::Document| {
        let f = super::surface_wire::build_field(doc, "test", photocraft_paint::SampleMode::AllLayers, 24.0, Some(target)).unwrap();
        let mut out = vec![0.0f32; 200 * 100];
        f.shade_into(Rect::new(0, 0, 200, 100), &mut out);
        out
    };
    // The red stroke does not change the shading of what is under it.
    let (a, b) = (shade(&before), shade(&after));
    assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
    // Without leaving the layer out, it would.
    let with = |doc: &photocraft_doc::Document| {
        let f = super::surface_wire::build_field(doc, "test", photocraft_paint::SampleMode::AllLayers, 24.0, None).unwrap();
        let mut out = vec![0.0f32; 200 * 100];
        f.shade_into(Rect::new(0, 0, 200, 100), &mut out);
        out
    };
    assert!(with(&before) != with(&after), "the stroke is in the composite");
}

#[test]
fn a_depth_stroke_next_to_an_occlusion_edge_does_not_spill_onto_the_far_surface() {
    // Near on the left, far on the right, the step at x = 100; a gentle ramp along y keeps the
    // photo's slope scale a normal slope.
    let mut s = depth_doc(200, 100, "Depth", |x, y| (if x < 100 { 0.8 } else { 0.2 }) + 0.0008 * y as f32);
    let brush = json!({"size": 40, "hardness": 1, "surface": {"depth": true, "relief": 1.0, "sizeByDepth": false}});
    s.execute("paint.stroke", json!({"points": [[92, 20], [92, 80]], "color": "#c02020", "target": "pixels", "brush": brush.clone()})).unwrap();
    let surf = layer_surface(&s);
    let painted = |x: i32, y: i32| surf.rgba(x, y)[3] > 0.0;
    assert!((20..80).any(|y| painted(92, y)), "the near side is painted");
    // Nothing past the edge (one pixel of antialiasing at most).
    let spill = (0..100).flat_map(|y| (102..200).map(move |x| (x, y))).filter(|&(x, y)| painted(x, y)).count();
    assert_eq!(spill, 0, "the dab must stop at the depth jump");
    // The cursor outline agrees: the east ray is cut at the edge.
    s.execute("tools.setBrush", brush).unwrap();
    let c = brush_cursor_shape(&mut s, 92.0, 50.0, 20.0).unwrap();
    assert!(c.outline.unwrap()[0] < 12.0);
}

#[test]
fn the_eraser_on_a_mask_has_no_surface_effects_in_commit_and_live() {
    let mut s = striped(400, 300);
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("mask", |doc, _| {
        doc.layer_mut(id).unwrap().mask = Some(photocraft_doc::LayerMask::reveal_all());
        Ok(())
    })
    .unwrap();
    // No saved planes: the eraser, turned into background painting on a mask, must not ask for them.
    let p =
        json!({"points": [[200, 270], [200, 140]], "erase": true, "target": "mask", "brush": {"size": 30, "hardness": 1, "surface": {"perspective": true}}});
    s.execute("paint.stroke", p.clone()).unwrap();
    assert!(LiveStroke::begin_with(&s, "paint.pencil", &p).is_ok());
    assert!(LiveStroke::begin_with(&s, "paint.stroke", &p).is_ok());
    // With planes the erasure is not deformed: the same width at both ends.
    s.execute("perspective.setPlanes", json!({"planes": [FLOOR]})).unwrap();
    s.execute("paint.stroke", json!({"points": [[200, 270], [200, 140]], "color": "#000000", "target": "mask", "brush": {"size": 30, "hardness": 1}})).unwrap();
    s.execute("paint.stroke", p).unwrap();
    let m = |x, y| s.active().unwrap().doc.layer(id).unwrap().mask.as_ref().unwrap().surface.pixel(x, y)[0];
    assert!(m(200, 270) > 0.99 && m(200, 150) > 0.99, "erased back to white");
    assert!(m(200 - 11, 150) > 0.99 && m(200 + 11, 150) > 0.99, "a plain 30 px erasure, not shrunk by the plane");
}

#[test]
fn follow_depth_and_perspective_keep_the_planes_foreshortening_in_the_footprint() {
    // Flat depth with size by depth: the depth footprint alone is a circle, so any squash in the
    // combined outline is the receding plane's (rays 0 and 16 run along x, 8 and 24 along y).
    let mut s = depth_doc(400, 300, "Depth", |_, _| 0.6);
    s.execute("perspective.setPlanes", json!({"planes": [FLOOR]})).unwrap();
    s.execute("tools.setBrush", depth_brush(json!({"sizeByDepth": true, "perspective": true}))).unwrap();
    let both = brush_cursor_shape(&mut s, 200.0, 200.0, 20.0).unwrap();
    let rays = both.outline.expect("a footprint with depth on");
    let (along_x, along_y) = (rays[0].min(rays[16]), rays[8].max(rays[24]));
    assert!(along_y < along_x * 0.8, "the plane's foreshortening is in the outline: x {along_x}, y {along_y}");
    assert!(rays.iter().all(|r| *r <= both.radius + 1e-4 && *r > 0.0));
    // Depth alone stays a circle there.
    s.execute("tools.setBrush", json!({"surface": {"perspective": false}})).unwrap();
    let depth_only = brush_cursor_shape(&mut s, 200.0, 200.0, 20.0).unwrap();
    assert!(depth_only.outline.unwrap().iter().all(|r| (r - depth_only.radius).abs() < 1e-4));
}

#[test]
fn an_all_layers_field_is_rebuilt_when_the_global_light_changes() {
    let mut s = folds_doc(200, 100);
    folds_stroke(&mut s, 30);
    folds_stroke(&mut s, 50);
    assert_eq!(s.surface_cache.field_builds(), 1);
    s.execute("layer.layerStyle.globalLight", json!({"angle": 45, "altitude": 60})).unwrap();
    folds_stroke(&mut s, 70);
    assert_eq!(s.surface_cache.field_builds(), 2, "the composite depends on the global light");
}

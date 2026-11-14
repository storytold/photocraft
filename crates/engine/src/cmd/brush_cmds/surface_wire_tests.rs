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
    // The first dab keeps the brush size exactly where the stroke starts (checked on the dabs: the
    // painted width also has the edge's anti-aliasing and the dab spacing in it).
    let map = super::surface_wire::plane_map(&s).unwrap() as Arc<dyn SurfaceMap>;
    let brush = BrushSettings { size: 30.0, ..BrushSettings::default() };
    let mut g = photocraft_paint::dynamics::DabGenerator::new(&brush, 1.0).with_surface(Some(map));
    let (mut dabs, mut dual) = (Vec::new(), Vec::new());
    g.push(&[StrokePoint::new(200.0, 270.0, 1.0), StrokePoint::new(200.0, 140.0, 1.0)], &mut dabs, &mut dual);
    g.finish(&mut dabs, &mut dual);
    assert_eq!(dabs.first().map(|d| d.radius), Some(15.0));
    assert!(dabs.last().is_some_and(|d| d.radius < 12.0));
    assert!((near as f64 - 30.0).abs() <= 4.0, "{near}");
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

// ---- Follow Folds ----

/// A document whose picture is a sinusoidal fold: columns of brightness `0.5 + 0.3 sin(2πx/16)`,
/// with an empty layer on top.
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

fn fold_map(s: &Session, base_radius: f32, depth: f32, bend: f32) -> Arc<dyn SurfaceMap> {
    let doc = &s.active().unwrap().doc;
    let field = super::surface_wire::build_field(doc, "test", photocraft_paint::SampleMode::AllLayers, 24.0, None).unwrap();
    super::surface_wire::make_map(None, Some(super::surface_wire::FoldMap::new(Arc::new(field), base_radius, depth, bend))).unwrap()
}

fn dabs_with(brush: &BrushSettings, map: Arc<dyn SurfaceMap>, pts: &[StrokePoint]) -> Vec<photocraft_paint::Dab> {
    let mut g = photocraft_paint::dynamics::DabGenerator::new(brush, 1.0).with_surface(Some(map));
    let (mut dabs, mut dual) = (Vec::new(), Vec::new());
    g.push(pts, &mut dabs, &mut dual);
    g.finish(&mut dabs, &mut dual);
    dabs
}

#[test]
fn dabs_on_a_steep_slope_are_squashed_along_it_and_flat_ones_are_not() {
    let s = folds_doc(200, 100);
    let map = fold_map(&s, 6.0, 0.5, 0.0);
    // x = 40: the brightness is mid-way and falling fastest; x = 36: a crest.
    let slope = map.local(photocraft_geom::Point::new(40.0, 50.0)).unwrap();
    assert!(slope.squash < 0.9, "{slope:?}");
    assert!(slope.squash_angle.cos().abs() > 0.99, "squashed along x: {slope:?}");
    assert_eq!(slope.offset, [0.0, 0.0], "no bend asked");
    let crest = map.local(photocraft_geom::Point::new(36.0, 50.0)).map_or(1.0, |l| l.squash);
    assert!(crest > 0.97, "{crest}");
    // Depth 0 never squashes.
    let none = fold_map(&s, 6.0, 0.0, 0.0);
    assert!(none.local(photocraft_geom::Point::new(40.0, 50.0)).is_none());
}

#[test]
fn bend_moves_dab_centres_with_the_fold_height() {
    let s = folds_doc(200, 100);
    let brush = BrushSettings { size: 12.0, ..BrushSettings::default() };
    let pts = [StrokePoint::new(10.0, 50.0, 1.0), StrokePoint::new(150.0, 50.0, 1.0)];
    let bent = dabs_with(&brush, fold_map(&s, 6.0, 0.5, 0.3), &pts);
    let ys: Vec<f64> = bent.iter().map(|d| d.center.y).collect();
    let (lo, hi) = ys.iter().fold((f64::MAX, f64::MIN), |(l, h), y| (l.min(*y), h.max(*y)));
    assert!(hi - lo > 2.0, "{lo} {hi}");
    let near = |x: f64| bent.iter().min_by(|a, b| (a.center.x - x).abs().total_cmp(&(b.center.x - x).abs())).map(|d| d.center.y).unwrap();
    // A bright crest (x = 36) rises, a dark trough (x = 44) sinks.
    assert!(near(36.0) < 50.0 - 1.0 && near(44.0) > 50.0 + 1.0, "{} {}", near(36.0), near(44.0));
    // No bend: the centres stay on the line.
    let straight = dabs_with(&brush, fold_map(&s, 6.0, 0.5, 0.0), &pts);
    assert!(straight.iter().all(|d| d.center.y == 50.0));
}

#[test]
fn folds_off_or_flat_paint_exactly_as_today() {
    let stroke = |surface: Value| {
        let mut s = folds_doc(200, 100);
        s.execute("paint.stroke", json!({"points": [[20, 50], [180, 50]], "target": "pixels", "brush": {"size": 12, "surface": surface}})).unwrap();
        layer_surface(&s)
    };
    let area = Rect::new(0, 0, 200, 100);
    let plain = stroke(json!({}));
    assert!(same(&plain, &stroke(json!({"folds": false, "foldDepth": 1, "foldBend": 1})), area));
    assert!(same(&plain, &stroke(json!({"folds": true, "foldDepth": 0, "foldBend": 0})), area));
    assert!(!same(&plain, &stroke(json!({"folds": true})), area), "folds change the stroke");
}

/// The first dab of a stroke starting at `(x, y)` under the stroke's own wiring.
fn first_dab(s: &Session, x: f64, y: f64) -> Option<photocraft_paint::Dab> {
    let brush = s.tools.brush.clone();
    let layer = s.active().unwrap().active_layer;
    let wiring = super::surface_wire::Wiring::build(s, "test", &json!({}), &brush, layer).unwrap();
    let map = wiring.map()?.clone();
    dabs_with(&brush, map, &[StrokePoint::new(x, y, 1.0)]).first().copied()
}

fn assert_cursor_is_first_dab(s: &mut Session, x: f64, y: f64) {
    let radius = (s.tools.brush.size / 2.0).max(0.5);
    let cursor = brush_cursor_shape(s, x, y, radius);
    let dab = first_dab(s, x, y);
    match (cursor, dab) {
        (Some(c), Some(d)) => {
            assert_eq!((c.radius, c.proj_angle, c.proj_scale), (d.radius, d.proj_angle, d.proj_scale), "({x}, {y})");
            assert_eq!(d.center.x, x + f64::from(c.offset[0]));
            assert_eq!(d.center.y, y + f64::from(c.offset[1]));
        }
        (None, d) => assert!(d.is_none_or(|d| d.proj_scale == 1.0 && d.center.x == x && d.center.y == y), "({x}, {y}) {d:?}"),
        (c, None) => panic!("cursor {c:?} but no dab at ({x}, {y})"),
    }
}

#[test]
fn the_cursor_shape_and_offset_are_the_first_dab_with_folds_and_with_both() {
    let mut s = folds_doc(400, 300);
    s.execute("tools.setBrush", json!({"size": 12, "surface": {"folds": true, "foldDepth": 0.8, "foldBend": 0.5}})).unwrap();
    let mut moved = 0;
    for (x, y) in [(40.0, 50.0), (36.0, 50.0), (44.0, 120.0), (101.0, 200.0), (200.0, 270.0)] {
        assert_cursor_is_first_dab(&mut s, x, y);
        moved += brush_cursor_shape(&mut s, x, y, 6.0).is_some_and(|c| c.offset[1] != 0.0 || c.proj_scale < 1.0) as u32;
    }
    assert!(moved >= 3, "the folds show");
    // With planes as well: on the plane both apply, off it nothing does.
    s.execute("perspective.setPlanes", json!({"planes": [FLOOR]})).unwrap();
    s.execute("tools.setBrush", json!({"surface": {"perspective": true}})).unwrap();
    for (x, y) in [(200.0, 270.0), (200.0, 200.0), (100.0, 250.0), (60.0, 270.0)] {
        assert_cursor_is_first_dab(&mut s, x, y);
    }
    assert!(brush_cursor_shape(&mut s, 10.0, 10.0, 6.0).is_none(), "off the plane");
    let both = brush_cursor_shape(&mut s, 202.0, 250.0, 6.0).unwrap();
    s.execute("tools.setBrush", json!({"surface": {"folds": false}})).unwrap();
    let plane = brush_cursor_shape(&mut s, 202.0, 250.0, 6.0).unwrap();
    assert!(both.proj_scale <= plane.proj_scale + 1e-6 || both.proj_angle != plane.proj_angle || both.offset != plane.offset, "{both:?} {plane:?}");
}

#[test]
fn a_live_stroke_with_folds_commits_the_pixels_it_shows() {
    let p = json!({"points": [[20, 50]], "color": "#c04020", "seed": 3, "target": "pixels",
        "brush": {"size": 12, "surface": {"folds": true, "foldDepth": 1, "foldBend": 0.6, "color": true}}});
    let mut s = folds_doc(200, 100);
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
    let mut fresh = folds_doc(200, 100);
    fresh.execute("paint.stroke", commit).unwrap();
    assert!(same(&layer_surface(&fresh), &layer_surface(&s), area));
}

#[test]
fn hostile_fold_params_are_errors_and_the_command_reports_the_offset() {
    let mut s = folds_doc(200, 100);
    for opts in [
        json!({"folds": true, "foldDepth": 2}),
        json!({"folds": true, "foldDepth": -1}),
        json!({"foldBend": 1.5}),
        json!({"foldBend": -0.1}),
        json!({"foldDepth": "deep"}),
    ] {
        let p = json!({"points": [[10, 10], [50, 50]], "target": "pixels", "brush": {"size": 10, "surface": opts}});
        assert!(s.execute("paint.stroke", p.clone()).is_err(), "{opts}");
        assert!(s.execute("paint.pencil", p).is_err(), "{opts}");
        assert!(s.execute("tools.setBrush", json!({"surface": opts})).is_err(), "{opts}");
    }
    assert!(layer_surface(&s).content_bounds().is_empty());
    s.execute("tools.setBrush", json!({"size": 12, "surface": {"folds": true, "foldBend": 0.6}})).unwrap();
    let r = s.execute("brush.cursorShape", json!({"x": 36, "y": 50})).unwrap();
    assert_eq!(r["onPlane"], true, "{r}");
    let off = r["offset"].as_array().unwrap();
    assert_eq!(off.len(), 2);
    assert!(off[1].as_f64().unwrap() < -1.0, "a crest moves up: {r}");
}

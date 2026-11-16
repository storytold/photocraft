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
    s.edit("planes for another canvas", |doc, _| {
        doc.perspective = photocraft_doc::PerspectivePlanes { planes: vec![FLOOR], focal: None, size: Some([500, 300]) };
        Ok(())
    })
    .unwrap();
    assert!(s.execute("paint.stroke", p.clone()).is_err());
    // A Canvas Size carries the planes along with the picture, so they still count.
    s.execute("perspective.setPlanes", json!({"planes": [FLOOR]})).unwrap();
    s.execute("image.canvasSize", json!({"width": 500, "height": 300, "anchor": "topLeft"})).unwrap();
    s.execute("paint.stroke", p).unwrap();
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

fn dabs_with(brush: &BrushSettings, map: Arc<dyn SurfaceMap>, pts: &[StrokePoint]) -> Vec<photocraft_paint::Dab> {
    let mut g = photocraft_paint::dynamics::DabGenerator::new(brush, 1.0).with_surface(Some(map));
    let (mut dabs, mut dual) = (Vec::new(), Vec::new());
    g.push(pts, &mut dabs, &mut dual);
    g.finish(&mut dabs, &mut dual);
    dabs
}

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
fn an_all_layers_field_is_rebuilt_when_the_global_light_changes() {
    let mut s = folds_doc(200, 100);
    folds_stroke(&mut s, 30);
    folds_stroke(&mut s, 50);
    assert_eq!(s.surface_cache.field_builds(), 1);
    s.execute("layer.layerStyle.globalLight", json!({"angle": 45, "altitude": 60})).unwrap();
    folds_stroke(&mut s, 70);
    assert_eq!(s.surface_cache.field_builds(), 2, "the composite depends on the global light");
}

#[test]
fn surface_color_at_strength_zero_builds_no_field_even_on_a_canvas_over_the_sample_limit() {
    let mut s = folds_doc(200, 100);
    s.execute("paint.stroke", stroke(json!({"color": true, "strength": 0}))).unwrap();
    assert_eq!(s.surface_cache.field_builds(), 0, "strength 0 shades nothing, so nothing is sampled");
    // A canvas over the sampler's limit refuses a shaded stroke, but a plain one still paints.
    let side = 8200u32;
    assert!(u64::from(side) * u64::from(side) > super::surface_wire::MAX_SAMPLE_PIXELS);
    let mut big = Session::new();
    big.execute("file.new", json!({"width": side, "height": side, "background": "transparent"})).unwrap();
    big.execute("layer.new.layer", json!({})).unwrap();
    let p = |strength: f64| {
        json!({"points": [[20, 50], [80, 50]], "color": "#808080", "target": "pixels",
            "brush": {"size": 10, "hardness": 1, "surface": {"color": true, "strength": strength}}})
    };
    assert!(big.execute("paint.stroke", p(1.0)).is_err(), "a shaded stroke needs a sample");
    big.execute("paint.stroke", p(0.0)).unwrap();
    assert!(!layer_surface(&big).content_bounds().is_empty());
    assert_eq!(big.surface_cache.field_builds(), 0);
}

#[test]
fn an_all_layers_field_is_rebuilt_when_the_documents_patterns_change() {
    let mut s = folds_doc(200, 100);
    folds_stroke(&mut s, 30);
    folds_stroke(&mut s, 50);
    assert_eq!(s.surface_cache.field_builds(), 1);
    s.edit("add a pattern", |doc, _| {
        let mut tile = Surface::new(photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8, true));
        tile.fill_rect(Rect::new(0, 0, 2, 2), &[0.2, 0.4, 0.6, 1.0]);
        doc.patterns.push(photocraft_doc::pattern::Pattern::new("tile", tile, 2, 2));
        Ok(())
    })
    .unwrap();
    folds_stroke(&mut s, 70);
    assert_eq!(s.surface_cache.field_builds(), 2, "layer styles read the document's patterns");
}

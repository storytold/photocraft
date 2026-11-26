//! A live stroke shown as a masked colour layer (`LiveStroke::begin_overlay`) draws what its
//! commit writes: flattening the live document after the last step and the committed document
//! agree to within the 8-bit dither the commit adds, and the commit itself takes exactly the
//! pixels `paint.stroke` renders, whether or not the stroke was settled along the way.

use super::*;
use crate::brush_cmds::LiveStrokeExt;
use photocraft_doc::{Layer, LayerId, LayerMask};

const W: i32 = 120;
const H: i32 = 80;
/// What the preview may differ from the commit by: the commit's quantisation and dither.
const TOLERANCE: f32 = 2.0 / 255.0;

type Tweak = Box<dyn Fn(&mut Session)>;

fn t(f: impl Fn(&mut Session) + 'static) -> Tweak {
    Box::new(f)
}

fn surface(s: &Session) -> Surface {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().clone()
}

fn same_pixels(a: &Surface, b: &Surface, r: Rect) -> bool {
    (r.y0..r.y1).all(|y| (r.x0..r.x1).all(|x| a.pixel(x, y) == b.pixel(x, y)))
}

/// A white canvas with a textured, partly transparent layer on top, then `setup`.
fn scene(depth: u64, setup: &dyn Fn(&mut Session)) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": W, "height": H, "depth": depth, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("texture", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let r = Rect::new(8, 8, 90, 64);
        let mut data = Vec::new();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let v = ((x * 7 + y * 13) % 23) as f32 / 22.0;
                let a = if (x + y) % 5 == 0 { 0.5 } else { 1.0 };
                data.extend(photocraft_raster::from_rgba(&fmt, [v, 1.0 - v, ((x * y) % 7) as f32 / 6.0, a]));
            }
        }
        surf.write_region(r, &data);
        Ok(())
    })
    .unwrap();
    setup(&mut s);
    s
}

fn active(s: &Session) -> LayerId {
    s.active().unwrap().active_layer.unwrap()
}

fn tweak(s: &mut Session, f: impl FnOnce(&mut Layer)) {
    s.edit("tweak", |doc, active| {
        f(doc.layer_mut(active.unwrap()).unwrap());
        Ok(())
    })
    .unwrap();
}

/// Puts the active layer into a pass-through group on top of the stack.
fn grouped(s: &mut Session) {
    s.edit("group", |doc, active| {
        let layer = doc.remove(active.unwrap()).unwrap();
        doc.layers.push(Layer::group("Group", vec![layer]));
        Ok(())
    })
    .unwrap();
}

fn flat(doc: &photocraft_doc::Document) -> Vec<[f32; 4]> {
    photocraft_compose::flatten(doc).into_px()
}

/// The largest difference between two composites, in premultiplied channels and alpha.
fn worst(a: &[[f32; 4]], b: &[[f32; 4]]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(p, q)| (0..3).map(|c| (p[c] * p[3] - q[c] * q[3]).abs()).chain([(p[3] - q[3]).abs()]).fold(0.0f32, f32::max)).fold(0.0, f32::max)
}

fn points(v: &[[f64; 3]]) -> Vec<StrokePoint> {
    v.iter().map(|q| StrokePoint::new(q[0], q[1], q[2] as f32)).collect()
}

const PTS: [[f64; 3]; 6] = [[14.0, 40.0, 1.0], [34.0, 20.0, 0.8], [56.0, 52.0, 1.0], [78.0, 24.0, 0.6], [100.0, 44.0, 1.0], [70.0, 70.0, 1.0]];

/// Draws `PTS` live in chunks of two as the canvas does, and commits the same stroke on a second
/// scene; returns the live stroke, the committed session and the live-versus-commit difference
/// after the last step.
fn run(build: &dyn Fn() -> Session, mut p: Value) -> (LiveStroke, Session, f32) {
    let s = build();
    p["points"] = json!([PTS[0]]);
    let mut live = LiveStroke::begin_overlay(&s, "paint.stroke", &p).unwrap();
    for c in PTS[1..].chunks(2) {
        live.push(&points(c)).unwrap();
    }
    let mut committed = build();
    p["points"] = json!(PTS);
    p["seed"] = json!(live.seed);
    committed.execute("paint.stroke", p).unwrap();
    let d = worst(&flat(&live.doc), &flat(&committed.active().unwrap().doc));
    (live, committed, d)
}

fn brush(extra: Value) -> Value {
    let mut p = json!({"brush": {"size": 22, "hardness": 0.3, "flow": 0.6}, "color": "#d02060", "opacity": 0.8, "smoothing": 0.0, "target": "pixels"});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    p
}

#[test]
fn the_overlay_shows_what_the_commit_writes() {
    let masked = t(|s| {
        tweak(s, |l| {
            let mut m = LayerMask::reveal_all();
            let r = Rect::new(0, 0, W, H);
            let data: Vec<f32> = (0..H).flat_map(|y| (0..W).map(move |x| 0.3 + 0.7 * (((x + 2 * y) % 17) as f32 / 16.0))).collect();
            m.surface.write_region(r, &data);
            l.mask = Some(m);
            l.blend = photocraft_color::BlendMode::Multiply;
            l.opacity = 0.5;
            l.fill_opacity = 0.8;
        })
    });
    let cases: Vec<(&str, u64, Tweak, Value)> = vec![
        ("plain 8-bit", 8, t(|_| ()), brush(json!({}))),
        ("plain 16-bit", 16, t(|_| ()), brush(json!({}))),
        ("lock transparency", 8, t(|s| tweak(s, |l| l.locks.transparency = true)), brush(json!({}))),
        ("erase", 8, t(|_| ()), brush(json!({"erase": true}))),
        ("erase 16-bit", 16, t(|_| ()), brush(json!({"erase": true, "opacity": 0.6}))),
        (
            "selection",
            8,
            t(|s| {
                s.execute("select.rect", json!({"x": 30, "y": 15, "width": 50, "height": 40})).unwrap();
            }),
            brush(json!({})),
        ),
        ("opacity, mode and mask", 8, masked, brush(json!({}))),
        ("inside a group", 8, t(grouped), brush(json!({}))),
        (
            "inside a group, locked",
            8,
            t(|s| {
                grouped(s);
                tweak(s, |l| l.locks.transparency = true);
            }),
            brush(json!({})),
        ),
        ("opacity only", 8, t(|s| tweak(s, |l| l.opacity = 0.6)), brush(json!({}))),
        ("opacity only, erase", 8, t(|s| tweak(s, |l| l.fill_opacity = 0.6)), brush(json!({"erase": true}))),
        ("Multiply only", 8, t(|s| tweak(s, |l| l.blend = photocraft_color::BlendMode::Multiply)), brush(json!({}))),
    ];
    for (name, depth, setup, p) in cases {
        let (live, committed, d) = run(&|| scene(depth, &*setup), p);
        assert!(live.is_overlay(), "{name}: shown as an overlay");
        assert!(d <= TOLERANCE, "{name}: the preview and the commit differ by {d}");
        // Something was drawn, so the comparison is not of two untouched documents.
        let untouched = scene(depth, &*setup);
        assert!(worst(&flat(&untouched.active().unwrap().doc), &flat(&committed.active().unwrap().doc)) > 0.05, "{name}: the stroke shows");
    }
}

#[test]
fn the_overlay_leaves_the_targets_pixels_alone_until_the_commit() {
    let s = scene(8, &|_| ());
    let id = active(&s);
    let before = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().clone();
    let mut p = brush(json!({}));
    p["points"] = json!(PTS);
    let live = LiveStroke::begin_overlay(&s, "paint.stroke", &p).unwrap();
    assert!(live.is_overlay());
    let shown = live.doc.layer(id).unwrap().surface().unwrap();
    assert!(same_pixels(shown, &before, Rect::new(0, 0, W, H)), "no stroke pixels in the target");
    assert!(live.overlay_bounds().is_some_and(|b| !b.is_empty()));
}

/// How the live document holds the stroke next to the target `id`.
#[derive(Debug, PartialEq)]
enum Shape {
    /// The target is untouched except for the mask an erase puts on it; a paint stroke is a layer
    /// right above it (clipped to it when `clipped`).
    NoGroup { clipped: bool },
    /// The target is inside an isolated group that holds the stroke too.
    Group,
}

fn shape(live: &LiveStroke, before: &photocraft_doc::Document, id: LayerId, erase: bool) -> Shape {
    use photocraft_doc::LayerContent;
    let doc = &*live.doc;
    let path = doc.path_of(id).unwrap();
    let parent_path = &path[..path.len() - 1];
    let (at, siblings): (usize, &[Layer]) =
        (*path.last().unwrap(), if parent_path.is_empty() { doc.layers.as_slice() } else { doc.layer_at(parent_path).unwrap().children().unwrap() });
    let was = before.path_of(id).unwrap();
    if was == path {
        // No group around the target: the same stack, plus the stroke's layer when painting.
        let groups = |d: &photocraft_doc::Document| d.walk().iter().filter(|(_, _, l)| l.is_group()).count();
        assert_eq!(groups(doc), groups(before), "no group was added");
        let layers = |d: &photocraft_doc::Document| d.walk().len();
        assert_eq!(layers(doc), layers(before) + usize::from(!erase), "a paint stroke adds one layer, an erase none");
        let target = doc.layer(id).unwrap();
        assert_eq!(target.mask.is_some(), erase, "only an erase puts a mask on the target");
        if erase {
            return Shape::NoGroup { clipped: false };
        }
        let above = &siblings[at + 1];
        assert!(matches!(above.content, LayerContent::Fill(_)), "the stroke's layer sits right above the target");
        assert!(above.mask.is_some());
        return Shape::NoGroup { clipped: above.clipped };
    }
    let group = doc.layer_at(parent_path).unwrap();
    assert!(group.is_group() && parent_path.len() == was.len(), "the target moved into a new group");
    Shape::Group
}

#[test]
fn a_plain_target_takes_no_group_and_any_other_keeps_the_isolated_one() {
    let nogroup = |clipped| Shape::NoGroup { clipped };
    let cases: Vec<(&str, Tweak, Value, Shape)> = vec![
        ("paint", t(|_| ()), brush(json!({})), nogroup(false)),
        ("lock transparency", t(|s| tweak(s, |l| l.locks.transparency = true)), brush(json!({})), nogroup(true)),
        ("erase", t(|_| ()), brush(json!({"erase": true})), nogroup(false)),
        ("inside a group", t(grouped), brush(json!({})), nogroup(false)),
        ("opacity", t(|s| tweak(s, |l| l.opacity = 0.5)), brush(json!({})), Shape::Group),
        ("fill opacity, erase", t(|s| tweak(s, |l| l.fill_opacity = 0.5)), brush(json!({"erase": true})), Shape::Group),
        ("Multiply", t(|s| tweak(s, |l| l.blend = photocraft_color::BlendMode::Multiply)), brush(json!({})), Shape::Group),
        ("layer mask", t(|s| tweak(s, |l| l.mask = Some(LayerMask::reveal_all()))), brush(json!({})), Shape::Group),
        ("layer mask, erase", t(|s| tweak(s, |l| l.mask = Some(LayerMask::reveal_all()))), brush(json!({"erase": true})), Shape::Group),
    ];
    for (name, setup, mut p, expected) in cases {
        let s = scene(8, &*setup);
        let (id, before) = (active(&s), s.active().unwrap().doc.clone());
        p["points"] = json!(PTS);
        let live = LiveStroke::begin_overlay(&s, "paint.stroke", &p).unwrap();
        assert!(live.is_overlay(), "{name}");
        let erase = p["erase"] == json!(true);
        assert_eq!(shape(&live, &before, id, erase), expected, "{name}");
    }
}

#[test]
fn the_mask_has_the_depth_of_the_document() {
    for (depth, fmt) in [(8, photocraft_color::PixelFormat::GRAY8), (16, photocraft_color::PixelFormat::GRAY8.with_sample(photocraft_color::SampleType::U16))] {
        let s = scene(depth, &|_| ());
        let mut p = brush(json!({}));
        p["points"] = json!(PTS);
        let live = LiveStroke::begin_overlay(&s, "paint.stroke", &p).unwrap();
        let path = live.doc.path_of(active(&s)).unwrap();
        let stroke = &live.doc.layers[path[0] + 1];
        assert_eq!(stroke.mask.as_ref().unwrap().surface.format(), fmt, "{depth}-bit");
    }
}

#[test]
fn an_overlay_commit_takes_the_pixels_paint_stroke_renders() {
    for (name, extra) in [("paint", json!({})), ("erase", json!({"erase": true}))] {
        let build = || scene(8, &|_| ());
        let mut s = build();
        let mut p = brush(extra);
        p["points"] = json!([PTS[0]]);
        let mut live = LiveStroke::begin_overlay(&s, "paint.stroke", &p).unwrap();
        for c in PTS[1..].chunks(2) {
            live.push(&points(c)).unwrap();
        }
        assert!(live.is_overlay());
        p["points"] = json!(PTS);
        p["seed"] = json!(live.seed);
        s.offer_live_stroke(live).unwrap();
        let r = s.execute("paint.stroke", p.clone()).unwrap();
        s.clear_live_stroke();
        assert_eq!(r["fromLive"], true, "{name}: the commit took the live pixels");
        let mut fresh = build();
        let r = fresh.execute("paint.stroke", p).unwrap();
        assert!(r.get("fromLive").is_none());
        let (a, b) = (surface(&s), surface(&fresh));
        assert!(same_pixels(&a, &b, Rect::new(0, 0, W, H)), "{name}: the commit's pixels are a fresh render's");
        assert!(s.active().unwrap().doc.layers.iter().all(|l| !l.is_group()), "{name}: the committed document has no overlay group");
    }
}

#[test]
fn the_smoothing_tail_shows_mid_stroke_as_finishing_would_draw_it() {
    let build = || scene(8, &|_| ());
    let s = build();
    let mut p = brush(json!({"smoothing": 0.7}));
    p["points"] = json!([PTS[0]]);
    let mut live = LiveStroke::begin_overlay(&s, "paint.stroke", &p).unwrap();
    assert!(live.is_overlay());
    for k in 1..PTS.len() {
        live.push(&points(&PTS[k..=k])).unwrap();
        let mut c = build();
        let mut q = p.clone();
        q["points"] = json!(&PTS[..=k]);
        q["seed"] = json!(live.seed);
        c.execute("paint.stroke", q).unwrap();
        let d = worst(&flat(&live.doc), &flat(&c.active().unwrap().doc));
        assert!(d <= TOLERANCE, "after {k} points the preview differs from a commit by {d}");
    }
}

#[test]
fn a_stroke_the_overlay_cannot_show_keeps_the_pixel_path() {
    let cases = [
        ("Dissolve", brush(json!({"mode": "dissolve"}))),
        ("Multiply", brush(json!({"mode": "multiply"}))),
        ("Behind", brush(json!({"mode": "behind"}))),
        ("colour dynamics", brush(json!({"brush": {"size": 22, "colorDynamics": {"enabled": true}}}))),
    ];
    for (name, p) in cases {
        let (live, committed, d) = run(&|| scene(8, &|_| ()), p);
        assert!(!live.is_overlay(), "{name}");
        assert!(live.doc.layers.iter().all(|l| !l.is_group()), "{name}: the tree is the document's own");
        assert_eq!(live.doc.layers.len(), committed.active().unwrap().doc.layers.len(), "{name}");
        assert!(d <= TOLERANCE, "{name}: still previews the commit, {d}");
    }
}

#[test]
fn a_layer_the_group_would_blend_differently_keeps_the_pixel_path() {
    let clipped_above = t(|s| {
        let below = active(s);
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("layer.setProps", json!({"clipped": true})).unwrap();
        s.edit("select", |_, active| {
            *active = Some(below);
            Ok(())
        })
        .unwrap();
    });
    let blend_if =
        t(|s| tweak(s, |l| l.blend_if.ranges = vec![[photocraft_doc::BlendRange { black: [40, 40], white: [255, 255] }, photocraft_doc::BlendRange::FULL]]));
    let soft_mask = t(|s| {
        tweak(s, |l| {
            let mut m = LayerMask::reveal_all();
            m.density = 0.5;
            l.mask = Some(m);
        })
    });
    for (name, setup) in [("a layer clipped above", clipped_above), ("Blend If", blend_if), ("mask density", soft_mask)] {
        let (live, _, d) = run(&|| scene(8, &*setup), brush(json!({})));
        assert!(!live.is_overlay(), "{name}");
        assert!(d <= TOLERANCE, "{name}: {d}");
    }
}

/// A longer stroke than `PTS`, for settling along the way.
fn long_stroke() -> Vec<[f64; 3]> {
    (0..40).map(|i| [10.0 + 2.6 * f64::from(i), 40.0 + 24.0 * (f64::from(i) / 5.0).sin(), 0.5 + 0.5 * f64::from((i * 3) % 5) / 4.0]).collect()
}

#[test]
fn a_stroke_settled_along_the_way_commits_the_pixels_paint_stroke_renders() {
    let pts = long_stroke();
    let cases = [
        ("plain", brush(json!({}))),
        ("dual brush", brush(json!({"brush": {"size": 26, "hardness": 0.3, "flow": 0.6, "dualBrush": {"enabled": true, "size": 9, "spacing": 0.4}}}))),
        ("wet edges", brush(json!({"brush": {"size": 26, "hardness": 0.3, "flow": 0.6, "wetEdges": true}}))),
        ("texture", brush(json!({"brush": {"size": 26, "hardness": 0.3, "flow": 0.6, "texture": {"enabled": true, "depth": 0.7}}}))),
        ("smoothing", brush(json!({"smoothing": 0.6}))),
        ("erase", brush(json!({"erase": true}))),
        ("selection and lock", brush(json!({}))),
    ];
    let mut backlog = false;
    for (name, mut p) in cases {
        let setup = |s: &mut Session| {
            if name == "selection and lock" {
                s.execute("select.rect", json!({"x": 20, "y": 15, "width": 70, "height": 50})).unwrap();
                tweak(s, |l| l.locks.transparency = true);
            }
        };
        let mut fresh: Option<Session> = None;
        p["points"] = json!(pts);
        // Settled every `every` pushes, a few tiles at a time or until nothing is left.
        for (every, max) in [(1, 2), (3, 1), (5, usize::MAX)] {
            let s = scene(8, &setup);
            let mut q = p.clone();
            q["points"] = json!([pts[0]]);
            let mut live = LiveStroke::begin_overlay(&s, "paint.stroke", &q).unwrap();
            assert!(live.is_overlay(), "{name}");
            for (n, pt) in pts[1..].iter().enumerate() {
                live.push(&points(std::slice::from_ref(pt))).unwrap();
                if n % every == 0 {
                    backlog |= live.settle(max);
                }
            }
            let seed = live.seed;
            let commit = live.into_commit().unwrap();
            // The commit is what `paint.stroke` renders, once.
            let fresh = fresh.get_or_insert_with(|| {
                p["seed"] = json!(seed);
                let mut f = scene(8, &setup);
                f.execute("paint.stroke", p.clone()).unwrap();
                f
            });
            assert!(
                same_pixels(&commit.surface, &surface(fresh), Rect::new(0, 0, W, H)),
                "{name}: settling every {every} pushes ({max} tiles) changed the commit"
            );
        }
    }
    assert!(backlog, "settle never left a tile for later, so the test did not settle partway");
}

#[test]
fn a_stroke_that_is_not_an_overlay_has_nothing_to_settle() {
    let s = scene(8, &|_| ());
    let mut p = brush(json!({"mode": "multiply"}));
    p["points"] = json!(PTS);
    let mut live = LiveStroke::begin_overlay(&s, "paint.stroke", &p).unwrap();
    assert!(!live.is_overlay());
    assert!(!live.settle(16));
}

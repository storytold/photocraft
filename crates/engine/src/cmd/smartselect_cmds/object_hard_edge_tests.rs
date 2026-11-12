//! Object Selection › Hard Edge (#12855): `select.object`'s `"hardEdge"`.

use super::*;

const W: i32 = 200;
const H: i32 = 160;

/// The object: a red rectangle on blue.
fn inside(x: i32, y: i32) -> bool {
    (60..140).contains(&x) && (50..110).contains(&y)
}

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": depth})).unwrap();
    s.edit("paint", |doc, _| {
        let bg = doc.layers[0].surface_mut().unwrap();
        let n = bg.channels();
        let mut data = Vec::with_capacity((W * H) as usize * n);
        for y in 0..H {
            for x in 0..W {
                let c: [f32; 3] = if inside(x, y) { [0.86, 0.12, 0.12] } else { [0.12, 0.12, 0.86] };
                data.extend_from_slice(&c[..n.min(3)]);
                if n == 4 {
                    data.push(1.0);
                }
            }
        }
        bg.write_region(Rect::new(0, 0, W, H), &data);
        Ok(())
    })
    .unwrap();
    s
}

fn cov(s: &Session, x: i32, y: i32) -> f32 {
    s.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))
}

/// The selection as 8-bit coverage, row by row.
fn sel(s: &Session) -> Vec<u8> {
    (0..H).flat_map(|y| (0..W).map(move |x| (x, y))).map(|(x, y)| (cov(s, x, y) * 255.0).round() as u8).collect()
}

fn bounds(s: &Session) -> Rect {
    s.active().unwrap().doc.selection.as_ref().map_or(Rect::new(0, 0, 0, 0), |m| m.content_bounds())
}

fn partial(m: &[u8]) -> usize {
    m.iter().filter(|v| (1..255).contains(*v)).count()
}

/// The selection `params` gives on a fresh document.
fn select(depth: u32, params: Value) -> (Session, Value) {
    let mut s = session(depth);
    let r = s.execute("select.object", params).unwrap();
    (s, r)
}

#[test]
fn hard_edge_is_on_by_default_and_leaves_the_result_alone() {
    let (plain, _) = select(8, json!({"point": [100, 80]}));
    let (on, r) = select(8, json!({"point": [100, 80], "hardEdge": true}));
    assert_eq!(r["changed"], true);
    assert_eq!(sel(&plain), sel(&on));
    assert_eq!(bounds(&plain), bounds(&on));
    let (plain, _) = select(8, json!({"rect": [50, 40, 100, 80]}));
    let (on, _) = select(8, json!({"rect": [50, 40, 100, 80], "hardEdge": true}));
    assert_eq!(sel(&plain), sel(&on));
}

#[test]
fn hard_edge_off_firms_the_left_and_top_and_softens_the_right_and_bottom_at_each_depth() {
    for depth in [8, 16, 32] {
        let (hard, _) = select(depth, json!({"point": [100, 80]}));
        let (soft, r) = select(depth, json!({"point": [100, 80], "hardEdge": false}));
        assert_eq!(r["changed"], true, "depth {depth}");
        let (hb, sb) = (bounds(&hard), bounds(&soft));
        // The object is x 60..140, y 50..110.
        assert_eq!(hb, Rect::new(59, 49, 141, 111), "depth {depth}: Hard Edge on, as before");
        assert_eq!(sb, Rect::new(60, 50, 141, 111), "depth {depth}");
        assert_eq!(r["bounds"], json!([sb.x0, sb.y0, sb.width(), sb.height()]), "depth {depth}");
        let m = sel(&soft);
        let at = |x: i32, y: i32| m[(y * W + x) as usize];
        // Along the middle row and column: hard on the left and top, one rim pixel on the right
        // and bottom.
        assert_eq!([at(59, 80), at(60, 80), at(139, 80), at(140, 80), at(141, 80)], [0, 255, 255, 195, 0], "depth {depth}");
        assert_eq!([at(100, 49), at(100, 50), at(100, 109), at(100, 110), at(100, 111)], [0, 255, 255, 195, 0], "depth {depth}");
        assert!(partial(&m) < partial(&sel(&hard)), "depth {depth}: fewer partly selected pixels");
    }
}

#[test]
fn hard_edge_off_softens_a_dragged_rectangle_too() {
    let (hard, _) = select(8, json!({"rect": [50, 40, 100, 80]}));
    let (soft, r) = select(8, json!({"rect": [50, 40, 100, 80], "hardEdge": false}));
    assert_eq!(r["changed"], true);
    assert_ne!(sel(&soft), sel(&hard));
    assert!(partial(&sel(&soft)) < partial(&sel(&hard)));
    assert!(cov(&soft, 100, 80) >= 0.99);
}

#[test]
fn a_wrong_type_hard_edge_is_the_default() {
    let (plain, _) = select(8, json!({"point": [100, 80]}));
    for v in [json!("false"), json!(0), json!(null), json!([false]), json!({"on": false})] {
        let (s, r) = select(8, json!({"point": [100, 80], "hardEdge": v}));
        assert_eq!(r["changed"], true, "hardEdge {v}");
        assert_eq!(sel(&s), sel(&plain), "hardEdge {v}");
    }
}

#[test]
fn the_object_finder_cache_keeps_the_object_as_found() {
    // A soft click must not leave a softened object behind for the next hard click.
    let (plain, _) = select(8, json!({"point": [100, 80]}));
    let (mut s, _) = select(8, json!({"point": [100, 80], "hardEdge": false}));
    let soft = sel(&s);
    s.execute("select.object", json!({"point": [90, 70]})).unwrap();
    assert_eq!(sel(&s), sel(&plain));
    s.execute("select.object", json!({"point": [90, 70], "hardEdge": false})).unwrap();
    assert_eq!(sel(&s), soft);
    // One history step each.
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(sel(&s), sel(&plain));
}

#[test]
fn hard_edge_off_finding_nothing_changes_nothing() {
    let (s, r) = select(8, json!({"point": [3, 3], "hardEdge": false}));
    assert_eq!(r["changed"], false);
    assert!(s.active().unwrap().doc.selection.is_none());
}

#[test]
fn soften_edge_stays_full_at_the_canvas_border() {
    let canvas = Rect::new(0, 0, 10, 10);
    // A 4x4 block in the top-left corner: its left and top sides are the document's, so the
    // pixels past the canvas repeat it and those sides stay full.
    let r = soften_edge(&Region { bbox: Rect::new(0, 0, 4, 4), mask: vec![255; 16] }, canvas);
    assert_eq!(r.bbox, Rect::new(0, 0, 5, 5));
    assert_eq!(r.mask.len(), 25);
    let at = |x: i32, y: i32| (r.at(x, y) * 255.0).round() as u8;
    assert_eq!([at(0, 0), at(3, 0), at(4, 0)], [255, 255, 178], "the top row, along the border: 7/16 of a full pixel");
    assert_eq!([at(0, 3), at(0, 4)], [255, 178], "the left column");
    // Away from the border the box grows only right and down.
    let r = soften_edge(&Region { bbox: Rect::new(4, 4, 6, 6), mask: vec![255; 4] }, canvas);
    assert_eq!(r.bbox, Rect::new(4, 4, 7, 7));
    // A block at the canvas' right and bottom edges doesn't grow past them.
    let r = soften_edge(&Region { bbox: Rect::new(7, 7, 10, 10), mask: vec![255; 9] }, canvas);
    assert_eq!(r.bbox, Rect::new(7, 7, 10, 10));
}

#[test]
fn soften_edge_survives_odd_regions() {
    let canvas = Rect::new(0, 0, 10, 10);
    // Empty, outside the canvas, and a mask shorter than its box: no panic, nothing invented.
    let empty = Region { bbox: Rect::new(3, 3, 3, 3), mask: Vec::new() };
    assert_eq!(soften_edge(&empty, canvas), empty);
    let outside = Region { bbox: Rect::new(20, 20, 24, 24), mask: vec![255; 16] };
    assert_eq!(soften_edge(&outside, canvas), outside);
    let short = soften_edge(&Region { bbox: Rect::new(2, 2, 6, 6), mask: vec![255; 3] }, canvas);
    assert_eq!(short.mask.len(), (short.bbox.width() * short.bbox.height()) as usize);
    assert_eq!(short.bbox, Rect::new(2, 2, 6, 4), "trimmed to the pixels the three samples reach");
    assert_eq!(short.at(8, 8), 0.0);
    let far = Region { bbox: Rect::new(i32::MAX - 2, i32::MAX - 2, i32::MAX, i32::MAX), mask: vec![255; 4] };
    assert_eq!(soften_edge(&far, canvas), far);
}

#[test]
fn the_params_string_documents_hard_edge() {
    let specs = specs();
    let spec = specs.iter().find(|c| c.id == "select.object").unwrap();
    assert!(spec.params.contains(r#""hardEdge":bool=true"#), "{}", spec.params);
}

/// The reference application's test document (#12855): 800x600 RGB/8, white Background, and a
/// transparent layer with a red (220,30,30) rectangle 100,100-300,250 and a blue (30,30,220)
/// anti-aliased disc, centre 500,175, radius 75 (8x8 samples a pixel).
fn reference_document() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "depth": 8})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("paint", |doc, _| {
        let Some(surf) = doc.layers.last_mut().and_then(Layer::surface_mut) else {
            return Err(EngineError::Other("the new layer has no surface".into()));
        };
        let mut data = Vec::with_capacity(800 * 600 * 4);
        for y in 0..600 {
            for x in 0..800 {
                if (100..300).contains(&x) && (100..250).contains(&y) {
                    data.extend_from_slice(&[220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0]);
                    continue;
                }
                let inside = (0..64)
                    .filter(|i| {
                        let (dx, dy) = (x as f32 + (i % 8) as f32 / 8.0 + 1.0 / 16.0 - 500.0, y as f32 + (i / 8) as f32 / 8.0 + 1.0 / 16.0 - 175.0);
                        dx * dx + dy * dy <= 75.0 * 75.0
                    })
                    .count();
                let a = inside as f32 / 64.0;
                data.extend_from_slice(&if a > 0.0 { [30.0 / 255.0, 30.0 / 255.0, 220.0 / 255.0, a] } else { [0.0; 4] });
            }
        }
        surf.write_region(Rect::new(0, 0, 800, 600), &data);
        Ok(())
    })
    .unwrap();
    s
}

/// `[left, top, right, bottom]`, fully selected and partly selected pixels of a click at
/// `point`, as the issue measures them, and the partly selected values.
fn measure(point: [i32; 2], hard_edge: bool) -> ([i32; 4], usize, usize, Vec<u8>) {
    let mut s = reference_document();
    s.execute("select.object", json!({"point": point, "hardEdge": hard_edge})).unwrap();
    let m = s.active().unwrap().doc.selection.as_ref().unwrap();
    let b = m.content_bounds();
    let v: Vec<u8> = (0..600).flat_map(|y| (0..800).map(move |x| (x, y))).map(|(x, y)| (m.sample_channel(x, y, 0) * 255.0).round() as u8).collect();
    let rim: Vec<u8> = v.iter().copied().filter(|v| (1..255).contains(v)).collect();
    ([b.x0, b.y0, b.x1, b.y1], v.iter().filter(|v| **v == 255).count(), rim.len(), rim)
}

/// The reference application's figures for Hard Edge off, against ours. Its rectangle: bounds
/// 99,99,302,252, 29988 fully and 356 partly selected, the rim mostly at 175-202. Its bounds come
/// from a few stray pixels: 29988 + 356 pixels in a 203x153 box leave 715 empty, where a solid
/// 201x151 body leaves 708, so the body is x 100..=300, y 100..=250, which is ours exactly.
/// Hence the tolerance: one pixel on each side for the bounds, 0.1 % for the fully selected
/// count; the partly selected count matches exactly.
#[test]
fn hard_edge_off_matches_the_reference_rectangle() {
    let (b, full, partial, rim) = measure([200, 175], false);
    assert_eq!(b, [100, 100, 301, 251]);
    for (ours, theirs) in b.iter().zip([99, 99, 302, 252]) {
        assert!((ours - theirs).abs() <= 1, "bounds {b:?}");
    }
    assert_eq!((full, partial), (29995, 356));
    assert!(full.abs_diff(29988) * 1000 <= 29988, "fully selected {full} against 29988");
    let in_range = rim.iter().filter(|v| (175..=202).contains(*v)).count();
    assert!(in_range * 100 >= rim.len() * 95, "rim mostly at 175-202: {in_range} of {}", rim.len());
    // Against Hard Edge on: more fully selected, fewer partly selected, right and bottom kept.
    let (hb, hfull, hpartial, _) = measure([200, 175], true);
    assert!(full > hfull && partial < hpartial);
    assert_eq!((hb[2], hb[3]), (b[2], b[3]));
}

/// The disc, where ours differs more: the reference keeps its bounds (423,99,578,252) and gives
/// 17298 fully and 592 partly selected. Ours starts from a narrower Hard Edge on result, and the
/// same step gives 425,100,576,251, 17441 (0.8 % more) and 344.
#[test]
fn hard_edge_off_on_the_reference_disc() {
    let (b, full, partial, _) = measure([500, 175], false);
    assert_eq!((b, full, partial), ([425, 100, 576, 251], 17441, 344));
    for (ours, theirs) in b.iter().zip([423, 99, 578, 252]) {
        assert!((ours - theirs).abs() <= 2, "bounds {b:?}");
    }
    assert!(full.abs_diff(17298) * 100 <= 17298, "fully selected {full} against 17298");
}

/// Hard Edge on is left as it is (#12855). Our figures differ from the reference application's
/// (rectangle 99,99,300,250 · 29651 · 698; disc 423,99,578,252 · 17305 · 512); that is the
/// Object Finder's result, which this option doesn't change.
#[test]
fn hard_edge_on_is_unchanged_on_the_reference_document() {
    let (b, full, partial, _) = measure([200, 175], true);
    assert_eq!((b, full, partial), ([99, 99, 301, 251], 29304, 1400));
    let (b, full, partial, _) = measure([500, 175], true);
    assert_eq!((b, full, partial), ([424, 99, 576, 251], 16888, 1200));
}

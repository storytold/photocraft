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

/// The 1-2-1 by 1-2-1 tent over a whole-canvas mask, written out pixel by pixel; past the canvas
/// the border pixel repeats.
fn tent(m: &[u8]) -> Vec<u8> {
    let at = |x: i32, y: i32| u32::from(m[(y.clamp(0, H - 1) * W + x.clamp(0, W - 1)) as usize]);
    let mut out = Vec::with_capacity(m.len());
    for y in 0..H {
        for x in 0..W {
            let mut sum = 0;
            for (dy, wy) in [(-1, 1), (0, 2), (1, 1)] {
                for (dx, wx) in [(-1, 1), (0, 2), (1, 1)] {
                    sum += wy * wx * at(x + dx, y + dy);
                }
            }
            out.push(((sum + 8) / 16) as u8);
        }
    }
    out
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
fn hard_edge_off_gives_a_softer_wider_rim_at_each_depth() {
    for depth in [8, 16, 32] {
        let (hard, _) = select(depth, json!({"point": [100, 80]}));
        let (soft, r) = select(depth, json!({"point": [100, 80], "hardEdge": false}));
        assert_eq!(r["changed"], true, "depth {depth}");
        let (hb, sb) = (bounds(&hard), bounds(&soft));
        assert_eq!(sb, Rect::new(hb.x0 - 1, hb.y0 - 1, hb.x1 + 1, hb.y1 + 1), "depth {depth}: one pixel wider on each side");
        assert_eq!(r["bounds"], json!([sb.x0, sb.y0, sb.width(), sb.height()]), "depth {depth}");
        assert!(partial(&sel(&soft)) > partial(&sel(&hard)), "depth {depth}: more partly selected pixels");
        // Every pixel is the hard result under the tent: the rim ramps over the pixels on
        // either side of the hard edge, the inside and the outside are untouched.
        assert_eq!(sel(&soft), tent(&sel(&hard)), "depth {depth}");
        assert!(cov(&soft, 100, 80) >= 0.99 && cov(&soft, 3, 3) == 0.0, "depth {depth}");
    }
}

#[test]
fn hard_edge_off_softens_a_dragged_rectangle_too() {
    let (hard, _) = select(8, json!({"rect": [50, 40, 100, 80]}));
    let (soft, r) = select(8, json!({"rect": [50, 40, 100, 80], "hardEdge": false}));
    assert_eq!(r["changed"], true);
    let (hb, sb) = (bounds(&hard), bounds(&soft));
    assert_eq!(sb, Rect::new(hb.x0 - 1, hb.y0 - 1, hb.x1 + 1, hb.y1 + 1));
    assert!(partial(&sel(&soft)) > partial(&sel(&hard)));
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
fn soften_edge_ramps_inside_the_canvas_and_stays_full_at_its_border() {
    let canvas = Rect::new(0, 0, 10, 10);
    // A 4x4 block in the top-left corner: its left and top sides are the document's.
    let r = soften_edge(&Region { bbox: Rect::new(0, 0, 4, 4), mask: vec![255; 16] }, canvas);
    assert_eq!(r.bbox, Rect::new(0, 0, 5, 5));
    assert_eq!(r.mask.len(), 25);
    let at = |x: i32, y: i32| (r.at(x, y) * 255.0).round() as u8;
    assert_eq!([at(0, 0), at(2, 0), at(3, 0), at(4, 0)], [255, 255, 191, 64], "the top row, along the border");
    assert_eq!([at(0, 3), at(0, 4)], [191, 64], "the left column");
    assert_eq!([at(3, 3), at(4, 4)], [143, 16], "the free corner");
    // Away from the border the box grows on every side.
    let r = soften_edge(&Region { bbox: Rect::new(4, 4, 6, 6), mask: vec![255; 4] }, canvas);
    assert_eq!(r.bbox, Rect::new(3, 3, 7, 7));
    assert_eq!(r.at(3, 3), r.at(6, 6));
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
    assert_eq!(short.mask.len(), 36);
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

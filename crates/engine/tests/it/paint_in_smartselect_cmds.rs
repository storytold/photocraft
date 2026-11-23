//! Tests that run paint commands, moved from the engine-core `smartselect_cmds` tests: 27 test(s).

mod quick_drop_transparent_tests {
    use photocraft_engine::Session;
    use serde_json::{Value, json};

    const W: i32 = 800;

    const H: i32 = 600;

    const RED1: [i32; 4] = [100, 100, 300, 250];

    const RED2: [i32; 4] = [500, 100, 600, 200];

    fn issue_document() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": W, "height": H, "depth": 8})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Shapes"})).unwrap();
        for [x0, y0, x1, y1] in [RED1, RED2] {
            s.execute("select.rect", json!({"x": x0, "y": y0, "width": x1 - x0, "height": y1 - y0})).unwrap();
            s.execute("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
        }
        s.execute("select.deselect", json!({})).unwrap();
        s
    }

    fn marquee(s: &mut Session, [x0, y0, x1, y1]: [i32; 4]) {
        s.execute("select.rect", json!({"x": x0, "y": y0, "width": x1 - x0, "height": y1 - y0})).unwrap();
    }

    /// One Quick Selection click with the default 30 px brush.
    fn click(s: &mut Session, at: [f32; 2], mode: &str, sample_all: bool) -> Value {
        s.execute("select.quick", json!({"points": [at], "size": 30, "mode": mode, "sampleAllLayers": sample_all})).unwrap()
    }

    fn at(s: &Session, x: i32, y: i32) -> f32 {
        s.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))
    }

    fn inside([x0, y0, x1, y1]: [i32; 4], x: i32, y: i32) -> bool {
        (x0..x1).contains(&x) && (y0..y1).contains(&y)
    }

    /// (fully selected, partly selected) pixels outside both red rectangles, where Shapes is fully
    /// transparent.
    fn on_transparency(s: &Session) -> (usize, usize) {
        let mut n = (0, 0);
        for y in 0..H {
            for x in 0..W {
                if inside(RED1, x, y) || inside(RED2, x, y) {
                    continue;
                }
                let v = at(s, x, y);
                if v >= 0.999 {
                    n.0 += 1;
                } else if v > 0.0 {
                    n.1 += 1;
                }
            }
        }
        n
    }

    /// Fully selected pixels of `rect`.
    fn full_in(s: &Session, [x0, y0, x1, y1]: [i32; 4]) -> usize {
        (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|(x, y)| at(s, *x, *y) >= 0.999).count()
    }

    #[test]
    fn adding_red_2_drops_the_marquees_transparent_frame() {
        let mut s = issue_document();
        marquee(&mut s, [50, 50, 350, 300]);
        let before = s.active().unwrap().history.past_len();
        click(&mut s, [550.0, 150.0], "add", false);
        assert_eq!(on_transparency(&s), (0, 0), "no pixel that is transparent on Shapes stays selected");
        assert_eq!(full_in(&s, RED1), 200 * 150, "red 1 with the marquee's hard edge");
        assert!(full_in(&s, RED2) >= 9000, "red 2 from the stroke");
        assert_eq!(s.active().unwrap().history.past_len(), before + 1);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Quick Selection"));
    }

    #[test]
    fn subtracting_red_1_leaves_nothing_of_the_marquee() {
        let mut s = issue_document();
        marquee(&mut s, [50, 50, 350, 300]);
        click(&mut s, [200.0, 175.0], "subtract", false);
        assert_eq!(on_transparency(&s), (0, 0), "the transparent frame is gone");
        assert_eq!(full_in(&s, RED1), 0, "and red 1 was subtracted");
    }

    #[test]
    fn a_transparent_only_selection_minus_red_1_is_no_selection() {
        let mut s = issue_document();
        marquee(&mut s, [400, 50, 480, 300]);
        click(&mut s, [200.0, 175.0], "subtract", false);
        assert!(s.active().unwrap().doc.selection.is_none(), "no selection");
    }

    #[test]
    fn a_new_stroke_drops_transparent_pixels_too() {
        let mut s = issue_document();
        click(&mut s, [550.0, 150.0], "new", false);
        assert_eq!(on_transparency(&s), (0, 0));
        assert!(full_in(&s, RED2) >= 9000);
    }

    #[test]
    fn sample_all_layers_keeps_the_transparent_frame() {
        let mut s = issue_document();
        marquee(&mut s, [50, 50, 350, 300]);
        click(&mut s, [200.0, 175.0], "subtract", true);
        assert!(at(&s, 60, 60) >= 0.999 && at(&s, 340, 290) >= 0.999, "the frame stays");
    }

    #[test]
    fn the_background_loses_only_the_brush_disc() {
        let mut s = issue_document();
        let d = s.active_mut().unwrap();
        d.set_active_layer(Some(d.doc.layers[0].id));
        marquee(&mut s, [50, 50, 350, 300]);
        click(&mut s, [70.0, 280.0], "subtract", false);
        assert!(at(&s, 70, 280) < 0.5, "the disc is subtracted");
        assert!(at(&s, 340, 60) >= 0.999 && at(&s, 200, 175) >= 0.999, "the rest of the marquee stays");
    }

    #[test]
    fn a_stroke_that_finds_nothing_changes_nothing() {
        let mut s = issue_document();
        marquee(&mut s, [50, 50, 350, 300]);
        let before = s.active().unwrap().history.past_len();
        // Off the canvas: no region, so the transparent frame stays and no step is added.
        let out = s.execute("select.quick", json!({"points": [[-500.0, -500.0]], "size": 30, "mode": "add"})).unwrap();
        assert_eq!(out["changed"], json!(false));
        assert!(at(&s, 60, 60) >= 0.999);
        assert_eq!(s.active().unwrap().history.past_len(), before);
    }

    /// CodeRabbit on #14672: only fully transparent pixels drop out. A faint patch (alpha 0.2) beside
    /// the red rectangles stays selected, and a similar-looking patch of fully transparent pixels next
    /// to it does not.
    #[test]
    fn partly_transparent_pixels_stay_selected() {
        let mut s = issue_document();
        // A faint patch on the Shapes layer: 20 % red over 350,100-450,200.
        marquee(&mut s, [350, 100, 450, 200]);
        s.execute("edit.fill", json!({"color": "#dc1e1e", "opacity": 20})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        marquee(&mut s, [50, 50, 480, 300]);
        click(&mut s, [550.0, 150.0], "add", false);
        assert_eq!(full_in(&s, [350, 100, 450, 200]), 100 * 100, "the faint patch stays selected");
        assert_eq!(full_in(&s, [450, 100, 480, 200]), 0, "the transparent pixels beside it drop out");
    }

    /// CodeRabbit on #14672: the filter works on the selection's own tiles, so two pieces far apart
    /// (in different tiles, with nothing selected between them) each lose only their transparent
    /// pixels.
    #[test]
    fn two_selected_pieces_far_apart_each_drop_their_transparent_pixels() {
        let mut s = issue_document();
        marquee(&mut s, [10, 10, 130, 130]);
        s.execute("select.rect", json!({"x": 560, "y": 440, "width": 120, "height": 120, "mode": "add"})).unwrap();
        // A stroke that adds nothing new on transparency.
        click(&mut s, [550.0, 150.0], "add", false);
        assert_eq!(full_in(&s, [100, 100, 130, 130]), 30 * 30, "red 1's corner under the first piece");
        assert_eq!(full_in(&s, [10, 10, 100, 100]), 0, "the first piece's transparent part drops out");
        assert_eq!(full_in(&s, [560, 440, 680, 560]), 0, "the second piece lies on transparency and drops out");
    }
}

mod quick_enhance_edge_tests {
    use photocraft_engine::Session;
    use serde_json::{Value, json};

    use photocraft_raster::Surface;

    use std::collections::BTreeMap;

    const W: i32 = 800;

    const H: i32 = 600;

    /// "red 1": the rectangle the issue measured, as [left, top, right, bottom].
    const RECT: [i32; 4] = [100, 100, 300, 250];

    /// The issue's click, inside red 1, at the default brush size.
    const CLICK: (f32, f32) = (200.0, 175.0);

    /// The issue's document: white Background, then the pixel layer Shapes (active) holding "red 1".
    fn shapes_document(w: i32, h: i32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": w, "height": h, "depth": 8})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Shapes"})).unwrap();
        fill_rect(&mut s, RECT, "#dc1e1e");
        s
    }

    /// Fills `[left, top, right, bottom)` on the active pixel layer and leaves nothing selected.
    fn fill_rect(s: &mut Session, rect: [i32; 4], color: &str) {
        select_rect(s, rect);
        s.execute("edit.fill", json!({"color": color})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
    }

    /// Selects the rectangle `[left, top, right, bottom)` and leaves it selected.
    fn select_rect(s: &mut Session, [l, t, r, b]: [i32; 4]) {
        s.execute("select.rect", json!({"x": l, "y": t, "width": r - l, "height": b - t})).unwrap();
    }

    /// One Quick Selection click at the default size 30, with Enhance Edge as given.
    fn quick_click(s: &mut Session, enhance: bool) -> Value {
        s.execute("select.quick", json!({"points": [[CLICK.0, CLICK.1]], "size": 30, "mode": "new", "enhanceEdge": enhance})).unwrap()
    }

    /// The selection as `[left, top, right, bottom]`.
    fn bounds(s: &Session) -> [i32; 4] {
        let sel = s.active().unwrap().doc.selection.clone().unwrap_or_else(|| Surface::new(photocraft_color::PixelFormat::GRAY8));
        let b = sel.content_bounds();
        [b.x0, b.y0, b.x1, b.y1]
    }

    /// The selection's 8-bit coverage at one document pixel.
    fn value(s: &Session, x: i32, y: i32) -> u8 {
        let sel = s.active().unwrap().doc.selection.clone().unwrap_or_else(|| Surface::new(photocraft_color::PixelFormat::GRAY8));
        (f64::from(sel.sample_channel(x, y, 0)) * 255.0).round() as u8
    }

    /// `(fully selected, partly selected, value: count of the partly selected pixels)`.
    fn tally(s: &Session) -> (usize, usize, BTreeMap<u8, usize>) {
        let sel = s.active().unwrap().doc.selection.clone().unwrap_or_else(|| Surface::new(photocraft_color::PixelFormat::GRAY8));
        let b = sel.content_bounds();
        let (mut full, mut hist) = (0usize, BTreeMap::new());
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let v = (f64::from(sel.sample_channel(x, y, 0)) * 255.0).round() as i32;
                if v >= 255 {
                    full += 1;
                } else if v > 0 {
                    *hist.entry(v as u8).or_default() += 1;
                }
            }
        }
        (full, hist.values().sum(), hist)
    }

    /// The selection's counts, after asserting every partly selected pixel sits in the band the issue
    /// measured, 248 to 254.
    fn band_tally(s: &Session) -> (usize, usize) {
        let (full, partial, hist) = tally(s);
        let out_of_band: Vec<(u8, usize)> = hist.iter().filter(|(v, _)| **v < 248 || **v > 254).map(|(v, c)| (*v, *c)).collect();
        assert!(out_of_band.is_empty(), "partly selected pixels outside the measured band 248-254: {out_of_band:?}");
        (full, partial)
    }

    // --- the measured rim ----------------------------------------------------------------------------

    /// Done when, item 1: Enhance Edge on, one click in red 1 at size 30, gives bounds
    /// 100,100,300,250, about 2100-2200 partly selected pixels, all of them at values 248-254.
    #[test]
    fn enhance_edge_on_a_click_in_red_1_gives_about_2100_soft_rim_pixels() {
        let mut s = shapes_document(W, H);
        let out = quick_click(&mut s, true);
        assert_eq!(out["selected"], json!(true));
        assert_eq!(bounds(&s), RECT, "Enhance Edge must not move the cut");
        let (full, partial) = band_tally(&s);
        assert!((2100..=2200).contains(&partial), "the reference application measures 2146 partly selected pixels, got {partial}");
        assert_eq!(full + partial, 30000, "the cut covers exactly red 1's 200 x 150 pixels");
    }

    /// The rim is about three pixels wide inside the edge on all four sides: the outer three pixels of
    /// the cut read 248-254, the fourth is fully selected.
    #[test]
    fn enhance_edge_rim_is_about_three_pixels_wide_inside_the_edge() {
        let mut s = shapes_document(W, H);
        quick_click(&mut s, true);
        let (l, t, r, b) = (RECT[0], RECT[1], RECT[2], RECT[3]);
        let (mx, my) = ((l + r) / 2, (t + b) / 2);
        let sides: [(&str, [(i32, i32); 4]); 4] = [
            ("left", [(l, my), (l + 1, my), (l + 2, my), (l + 3, my)]),
            ("right", [(r - 1, my), (r - 2, my), (r - 3, my), (r - 4, my)]),
            ("top", [(mx, t), (mx, t + 1), (mx, t + 2), (mx, t + 3)]),
            ("bottom", [(mx, b - 1), (mx, b - 2), (mx, b - 3), (mx, b - 4)]),
        ];
        for (side, along) in sides {
            for (k, (x, y)) in along.into_iter().enumerate() {
                let v = value(&s, x, y);
                if k < 3 {
                    assert!((248..=254).contains(&v), "{side} edge, {k} px in at ({x},{y}): value {v}, expected the soft rim 248-254");
                } else {
                    assert_eq!(v, 255, "{side} edge, 3 px in at ({x},{y}) must be fully selected");
                }
            }
        }
    }

    /// The rim only ever takes coverage away: nothing appears outside red 1, so the bounds stay put.
    #[test]
    fn enhance_edge_rim_adds_nothing_outside_the_cut() {
        let mut s = shapes_document(W, H);
        quick_click(&mut s, true);
        assert_eq!(bounds(&s), RECT);
        let (l, t, r, b) = (RECT[0], RECT[1], RECT[2], RECT[3]);
        let (mx, my) = ((l + r) / 2, (t + b) / 2);
        for x in [l - 2, l - 1, r, r + 1] {
            assert_eq!(value(&s, x, my), 0, "({x},{my}) lies outside red 1 and must stay unselected");
        }
        for y in [t - 1, t - 2, b, b + 1] {
            assert_eq!(value(&s, mx, y), 0, "({mx},{y}) lies outside red 1 and must stay unselected");
        }
    }

    /// The reference application's rim dips further where two edges meet (its measured 248-253 sit at
    /// the corners), so the corner of the cut is never more selected than the middle of a straight edge.
    #[test]
    fn enhance_edge_rim_dips_at_least_as_deep_at_the_corner() {
        let mut s = shapes_document(W, H);
        quick_click(&mut s, true);
        let corner = value(&s, RECT[0], RECT[1]);
        let straight = value(&s, (RECT[0] + RECT[2]) / 2, RECT[1]);
        assert!((248..=254).contains(&corner), "the corner of the rim reads {corner}, expected 248-254");
        assert!(corner <= straight, "the corner ({corner}) must not be more selected than the straight edge ({straight})");
        assert!(straight <= 254, "the outermost rim pixel must sit below fully selected, reads {straight}");
    }

    /// The rule is not keyed to the measured rectangle: the same click in a bigger document gives the
    /// same rim pixel for pixel (the rim is computed in bands, and the bands must not show).
    #[test]
    fn enhance_edge_rim_does_not_depend_on_the_canvas_size() {
        let mut small = shapes_document(W, H);
        quick_click(&mut small, true);
        let mut big = shapes_document(1600, 1200);
        quick_click(&mut big, true);
        assert_eq!(bounds(&big), RECT, "the same click in a bigger document gives the same bounds");
        assert_eq!(tally(&big), tally(&small), "the same click in a bigger document gives the same rim");
    }

    /// The rim is one rule, not a number tuned to the measured rectangle: on a smaller shape (60 x 40 px)
    /// it gives the same three-pixel band at the same values, and the cut still covers exactly the shape.
    /// Measured here: 1780 fully selected and 620 partly selected of its 2400 pixels.
    #[test]
    fn enhance_edge_gives_the_same_rim_on_a_smaller_shape() {
        let mut s = shapes_document(W, H);
        fill_rect(&mut s, [400, 400, 460, 440], "#1edc3c");
        s.execute("select.quick", json!({"points": [[430.0, 420.0]], "size": 30, "mode": "new", "enhanceEdge": true})).unwrap();
        assert_eq!(bounds(&s), [400, 400, 460, 440], "the cut covers exactly the smaller shape");
        let (full, partial) = band_tally(&s);
        assert_eq!((full, partial), (1780, 620));
        assert_eq!(full + partial, 2400, "all 2400 pixels of the shape are selected");
        for k in 0..3 {
            let v = value(&s, 400 + k, 420);
            assert!((248..=254).contains(&v), "the rim reaches in from the left edge: {k} px in reads {v}");
        }
        assert_eq!(value(&s, 403, 420), 255, "the interior of the smaller shape stays fully selected");
    }

    /// The rim works on selection coverage, which is 8-bit whatever the document's depth, so a 16-bit
    /// document gets the same rim as the measured 8-bit one: the same bounds and the same pixels.
    #[test]
    fn enhance_edge_gives_the_same_rim_in_a_16_bit_document() {
        let mut eight = shapes_document(W, H);
        quick_click(&mut eight, true);
        let mut s = Session::new();
        s.execute("file.new", json!({"width": W, "height": H, "depth": 16})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Shapes"})).unwrap();
        fill_rect(&mut s, RECT, "#dc1e1e");
        quick_click(&mut s, true);
        assert_eq!(bounds(&s), RECT, "the cut covers exactly red 1 in a 16-bit document");
        band_tally(&s);
        assert_eq!(tally(&s), tally(&eight), "the rim works on selection coverage, which is 8-bit at any document depth");
    }

    // --- neighbours that must not move --------------------------------------------------------------
    /// Done when, item 2: Enhance Edge off is unchanged, pinned exactly as main measures it: bounds
    /// 100,100,300,250 with 29304 fully selected and 696 partly (a 191 rim and the four corners at 143).
    /// The issue wrote 99,99,301,251 with 1400 partly selected; #12922 landed on main while this was in
    /// progress and moved the raw footprint, so the pin holds main's current output, not that older one.
    #[test]
    fn enhance_edge_off_is_unchanged() {
        let mut s = shapes_document(W, H);
        quick_click(&mut s, false);
        assert_eq!(bounds(&s), [100, 100, 300, 250]);
        let (full, partial, hist) = tally(&s);
        assert_eq!(full, 29304);
        assert_eq!(partial, 696);
        assert_eq!(hist.get(&191), Some(&692), "the inner rim keeps its value 191");
        assert_eq!(hist.get(&143), Some(&4), "the four corners keep their value 143");
        assert_eq!(hist.len(), 2, "nothing else is partly selected: {hist:?}");
    }

    /// `refine_surface` also serves Select and Mask, so the refinement itself must keep its results.
    /// Pinned with Enhance Edge's values as user parameters on the same rectangle: 29996 fully
    /// selected and four pixels at 231 with Smooth 15, and 30000 fully selected with Smooth 10.
    /// Guard test: passes before and after this change.
    #[test]
    fn select_and_mask_with_enhance_edge_values_is_unchanged() {
        let cases: [(_, usize, usize, (_, usize)); 2] = [
            (json!({"radius": 4, "smartRadius": true, "smooth": 15, "contrast": 25}), 29996, 4, (231u8, 4usize)),
            (json!({"radius": 4, "smartRadius": true, "smooth": 10, "contrast": 25}), 30000, 0, (0u8, 0usize)),
        ];
        for (params, full, partial, last) in cases {
            let mut s = shapes_document(W, H);
            select_rect(&mut s, RECT);
            s.execute("select.refineEdge", params.clone()).unwrap();
            assert_eq!(bounds(&s), RECT, "{params}");
            let (f, p, hist) = tally(&s);
            assert_eq!((f, p), (full, partial), "{params}");
            let expect: BTreeMap<u8, usize> = if last.1 == 0 { BTreeMap::new() } else { [(last.0, last.1)].into_iter().collect() };
            assert_eq!(hist, expect, "{params}");
        }
    }

    /// Feathering and Shift Edge, the other options that run the same refinement, keep their measured
    /// soft results. Guard test: passes before and after this change.
    #[test]
    fn select_and_mask_feathering_and_shift_edge_are_unchanged() {
        let cases: [(_, [i32; 4], usize, usize); 3] = [
            (json!({"radius": 4, "smartRadius": true, "smooth": 15, "contrast": 25, "feather": 2}), [99, 99, 301, 251], 29304, 1396),
            (json!({"feather": 3, "smooth": 20, "contrast": 40}), [99, 99, 301, 251], 29292, 1408),
            (json!({"shiftEdge": -30, "smooth": 20}), [99, 99, 301, 251], 29304, 1388),
        ];
        for (params, want_bounds, full, partial) in cases {
            let mut s = shapes_document(W, H);
            select_rect(&mut s, RECT);
            s.execute("select.refineEdge", params.clone()).unwrap();
            assert_eq!(bounds(&s), want_bounds, "{params}");
            let (f, p, _) = tally(&s);
            assert_eq!((f, p), (full, partial), "{params}");
        }
    }

    /// A cut lying on the document border is not measured by the issue, so it keeps today's behaviour
    /// there: pixels past the canvas repeat the canvas edge and the border stays fully selected, while
    /// the edges away from the border get the same soft rim. Guard on the border, red on the rim.
    #[test]
    fn enhance_edge_keeps_a_cut_at_the_document_border_fully_selected_there() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": W, "height": H, "depth": 8})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Blue"})).unwrap();
        fill_rect(&mut s, [0, 0, 300, 200], "#1e3cdc");
        s.execute("select.quick", json!({"points": [[100.0, 100.0]], "size": 30, "mode": "new", "enhanceEdge": true})).unwrap();
        assert_eq!(bounds(&s), [0, 0, 300, 200]);
        // Where the cut lies on the document border it keeps its full coverage: pixels past the canvas
        // repeat the canvas edge, so there is nothing outside the cut there to dip towards.
        for y in [0, 1, 100] {
            assert_eq!(value(&s, 0, y), 255, "the left document border at y={y} stays fully selected");
        }
        for x in [0, 100, 250] {
            assert_eq!(value(&s, x, 0), 255, "the top document border at x={x} stays fully selected");
        }
        // The edges of the cut that lie away from the border get the same soft rim as red 1, and an edge
        // that is inside the document is rimmed even where it crosses the document border.
        for k in 0..3 {
            let v = value(&s, 299 - k, 100);
            assert!((248..=254).contains(&v), "the rim reaches in from the right edge: {k} px in reads {v}");
            let w = value(&s, 100, 199 - k);
            assert!((248..=254).contains(&w), "the rim reaches in from the bottom edge: {k} px in reads {w}");
        }
        assert_eq!(value(&s, 296, 100), 255, "the rim stops about three pixels in");
        assert_eq!(value(&s, 100, 196), 255, "the rim stops about three pixels in");
        assert!((248..=254).contains(&value(&s, 299, 0)), "the cut's right edge is rimmed on the top row too: {}", value(&s, 299, 0));
    }
}

mod quick_group_target_tests {
    use photocraft_engine::Session;
    use serde_json::{Value, json};

    const W: i32 = 800;

    const H: i32 = 600;

    /// The centre of "red 2", the click the issue measured.
    const RED2_CLICK: [f32; 2] = [550.0, 150.0];

    /// The issue's document: white Background; pixel layer Shapes with red (220,30,30) rectangles
    /// 100,100-300,250 ("red 1") and 500,100-600,200 ("red 2"); an empty pixel layer Blue on top; then
    /// a new, empty group, which is left active.
    fn issue_document() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": W, "height": H, "depth": 8})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Shapes"})).unwrap();
        for [x, y, w, h] in [[100, 100, 200, 150], [500, 100, 100, 100]] {
            s.execute("select.rect", json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
            s.execute("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
        }
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Blue"})).unwrap();
        s.execute("layer.new.group", json!({})).unwrap();
        s
    }

    /// A Quick Selection click at `at` with the default 30 px brush.
    fn click(s: &mut Session, at: [f32; 2], sample_all: bool) -> Value {
        s.execute("select.quick", json!({"points": [at], "size": 30, "mode": "new", "sampleAllLayers": sample_all})).unwrap()
    }

    /// The kind of the active layer ("Group", "Adjustment", "Pixel", ...).
    fn active_kind(s: &Session) -> &'static str {
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap().content.kind_name()
    }

    /// The selection's coverage at every document pixel, row-major.
    fn coverage(s: &Session) -> Vec<f32> {
        let sel = s.active().unwrap().doc.selection.clone();
        (0..H).flat_map(|y| (0..W).map(move |x| (x, y))).map(|(x, y)| sel.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))).collect()
    }

    /// `[left, top, right, bottom]` of the selection, as the issue writes bounds.
    fn bounds(s: &Session) -> [i32; 4] {
        let b = s.active().unwrap().doc.selection.as_ref().unwrap().content_bounds();
        [b.x0, b.y0, b.x1, b.y1]
    }

    /// Asserts the selection covers the whole rectangle `[x0, y0, x1, y1)` and reaches at most
    /// `slack` pixels beyond it: the composite's shape, not a brush disc.
    fn assert_selects_rect(s: &Session, [x0, y0, x1, y1]: [i32; 4], slack: i32) {
        let cov = coverage(s);
        for y in y0..y1 {
            for x in x0..x1 {
                assert!(cov[(y * W + x) as usize] >= 0.5, "({x},{y}) inside the shape is not selected");
            }
        }
        let [l, t, r, b] = bounds(s);
        assert!(l >= x0 - slack && t >= y0 - slack && r <= x1 + slack && b <= y1 + slack, "bounds {:?} reach past the shape", [l, t, r, b]);
    }

    #[test]
    fn a_group_without_sample_all_layers_selects_only_the_brush_disc() {
        // The disc a click on a flat area gives: Background active, which is white everywhere.
        let mut flat = issue_document();
        let d = flat.active_mut().unwrap();
        d.set_active_layer(Some(d.doc.layers[0].id));
        assert_eq!(active_kind(&flat), "Pixel");
        click(&mut flat, RED2_CLICK, false);
        let disc = coverage(&flat);

        let mut s = issue_document();
        let before = s.active().unwrap().history.past_len();
        let out = click(&mut s, RED2_CLICK, false);
        assert_eq!(out["changed"], json!(true));
        assert_eq!(s.active().unwrap().history.past_len(), before + 1);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Quick Selection"));
        assert_eq!(active_kind(&s), "Group");
        // Pixel for pixel the flat-area disc, not red 2 (100 x 100) from the composite.
        assert!(coverage(&s) == disc, "a group click differs from the brush disc on a flat area");
        let [l, t, r, b] = bounds(&s);
        assert!((30..=40).contains(&(r - l)) && (30..=40).contains(&(b - t)), "bounds {:?} are not a 30 px brush disc", [l, t, r, b]);
        assert!(l < 550 && r > 550 && t < 150 && b > 150, "bounds {:?} miss the click", [l, t, r, b]);
    }

    #[test]
    fn a_group_with_sample_all_layers_reads_the_composite() {
        let mut s = issue_document();
        assert_eq!(active_kind(&s), "Group");
        click(&mut s, [200.0, 175.0], true);
        // Measured: 99,99,301,251, all of red 1.
        assert_selects_rect(&s, [100, 100, 300, 250], 3);
    }

    #[test]
    fn an_adjustment_layer_reads_the_composite() {
        let mut s = issue_document();
        s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
        assert_eq!(active_kind(&s), "Adjustment");
        click(&mut s, RED2_CLICK, false);
        assert_selects_rect(&s, [500, 100, 600, 200], 3);
    }

    #[test]
    fn an_artboard_or_a_frame_still_reads_the_composite() {
        // Not measured for these groups, so they keep reading the composite.
        for (cmd, params) in
            [("layer.new.artboard", json!({"rect": [700, 500, 50, 50]})), ("layer.new.frame", json!({"x": 700, "y": 500, "width": 50, "height": 50}))]
        {
            let mut s = issue_document();
            s.execute(cmd, params).unwrap();
            assert_eq!(active_kind(&s), "Group", "{cmd}");
            click(&mut s, RED2_CLICK, false);
            assert_selects_rect(&s, [500, 100, 600, 200], 3);
        }
    }
}

mod tests {
    use photocraft_doc::LayerId;
    use photocraft_engine::Session;
    use photocraft_engine::smartselect_cmds::*;
    use photocraft_geom::Rect;
    use serde_json::json;

    use photocraft_engine::smartselect_cmds::SmartSelectExt;

    use photocraft_algo::segment::Rng;

    /// A document whose background shows `px(x, y)` (RGB, 0–1), at the given depth.
    fn session_with(w: u32, h: u32, depth: u32, px: impl Fn(u32, u32) -> [f32; 3]) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": w, "height": h, "depth": depth})).unwrap();
        s.edit("paint", |doc, _| {
            let bg = doc.layers[0].surface_mut().unwrap();
            let n = bg.channels();
            let mut data = Vec::with_capacity((w * h) as usize * n);
            for y in 0..h {
                for x in 0..w {
                    let c = px(x, y);
                    data.extend_from_slice(&c[..n.min(3)]);
                    if n == 4 {
                        data.push(1.0);
                    }
                }
            }
            bg.write_region(Rect::new(0, 0, w as i32, h as i32), &data);
            Ok(())
        })
        .unwrap();
        s
    }

    fn noise_field(w: u32, h: u32, seed: u64) -> Vec<f32> {
        let mut rng = Rng::new(seed);
        (0..w * h).map(|_| rng.normal()).collect()
    }

    /// Textured disc (centre (100, 80), radius 45) on a noisy background.
    fn disc(depth: u32) -> (Session, impl Fn(i32, i32) -> bool) {
        let nz = noise_field(200, 160, 2);
        let inside = |x: i32, y: i32| {
            let (dx, dy) = (x as f32 + 0.5 - 100.0, y as f32 + 0.5 - 80.0);
            dx * dx + dy * dy <= 45.0 * 45.0
        };
        let s = session_with(200, 160, depth, move |x, y| {
            let n = 0.05 * nz[(y * 200 + x) as usize];
            if inside(x as i32, y as i32) {
                let t = ((x + y) as f32 * 0.35).sin() * 0.5 + 0.5;
                [0.85 + 0.05 * t + n, 0.2 + 0.3 * t + n, 0.15 + n]
            } else {
                [0.25 + n, 0.45 + n, 0.7 + n]
            }
            .map(|v| v.clamp(0.0, 1.0))
        });
        (s, inside)
    }

    fn cov(s: &Session, x: i32, y: i32) -> f32 {
        s.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))
    }

    fn sel_iou(s: &Session, w: i32, h: i32, truth: impl Fn(i32, i32) -> bool) -> f32 {
        let (mut i, mut u) = (0, 0);
        for y in 0..h {
            for x in 0..w {
                let (a, b) = (cov(s, x, y) >= 0.5, truth(x, y));
                i += (a && b) as i32;
                u += (a || b) as i32;
            }
        }
        i as f32 / u.max(1) as f32
    }

    #[test]
    fn object_finder_cache_serves_the_whole_object_until_pixels_change() {
        let (mut s, _) = disc(8);
        let d = s.active().unwrap();
        let doc = std::sync::Arc::clone(&d.doc);
        let target = sampled_target(&doc, d.active_layer, false);
        assert!(s.object_finder().lookup(&doc, target, 100.0, 80.0).is_none());
        let r = std::sync::Arc::new(find_object(&doc, d.active_layer, false, 100.0, 80.0).unwrap());
        s.object_finder_mut().insert(&doc, target, std::sync::Arc::clone(&r));
        // Any point on the object hits; the background and another target miss.
        assert!(std::sync::Arc::ptr_eq(&s.object_finder().lookup(&doc, target, 120.0, 90.0).unwrap(), &r));
        assert!(s.object_finder().lookup(&doc, target, 3.0, 3.0).is_none());
        assert!(s.object_finder().lookup(&doc, Some(LayerId(u64::MAX)), 100.0, 80.0).is_none());
        // The click reuses it, and a selection-only edit keeps it.
        s.execute("select.object", json!({"point": [110, 70]})).unwrap();
        assert_eq!(s.object_finder().len(), 1);
        s.execute("select.deselect", json!({})).unwrap();
        let d = s.active().unwrap();
        assert!(s.object_finder().lookup(&std::sync::Arc::clone(&d.doc), target, 100.0, 80.0).is_some());
        // A pixel edit drops it.
        s.execute("paint.pencil", json!({"points": [[5, 5], [30, 5]], "size": 4, "color": "#000000"})).unwrap();
        assert!(s.object_finder().is_empty());
    }

    #[test]
    fn object_selection_on_an_empty_active_layer_says_so() {
        // #5246: an empty layer above the photo used to find nothing without a word.
        let (mut s, inside) = disc(8);
        s.execute("layer.new.layer", json!({})).unwrap();
        for p in [json!({"rect": [40, 20, 120, 120]}), json!({"point": [100, 80]})] {
            let e = s.execute("select.object", p.clone()).unwrap_err().to_string();
            assert!(e.contains("active layer is empty") && e.contains("sampleAllLayers"), "{p}: {e}");
            assert!(s.active().unwrap().doc.selection.is_none());
        }
        // Sampling all layers finds the disc below.
        s.execute("select.object", json!({"rect": [40, 20, 120, 120], "sampleAllLayers": true})).unwrap();
        assert!(sel_iou(&s, 200, 160, &inside) > 0.95);
        // A layer with pixels elsewhere: the rectangle holds none of them.
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("paint.pencil", json!({"points": [[5, 5], [10, 5]], "size": 2, "color": "#000000"})).unwrap();
        let e = s.execute("select.object", json!({"rect": [60, 60, 100, 80]})).unwrap_err().to_string();
        assert!(e.contains("no pixels in that area"), "{e}");
    }

    /// Select Subject looks at the selected layer only; with no layer selected it looks at the
    /// whole image.
    #[test]
    fn select_subject_samples_the_selected_layer_else_the_composite() {
        let (mut s, inside) = disc(8);
        // A layer above holding only a small square in the corner: the disc is not on it.
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("paint.pencil", json!({"points": [[10, 10], [30, 10]], "size": 3, "color": "#000000"})).unwrap();
        s.execute("select.subject", json!({})).unwrap();
        assert!(cov(&s, 100, 80) < 0.5, "the disc on the layer below was selected");
        s.execute("select.deselect", json!({})).unwrap();
        // No layer selected: the whole image, so the disc is found.
        s.execute("select.deselectLayers", json!({})).unwrap();
        s.execute("select.subject", json!({})).unwrap();
        assert!(sel_iou(&s, 200, 160, &inside) > 0.9);
    }
}

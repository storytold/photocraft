//! Tests that run painting commands, moved from the engine-core `smartselect_cmds` tests: 16 test(s).

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

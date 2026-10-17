use photocraft_doc::{LayerContent, LayerId, LiveShape, Path};
use photocraft_engine::Session;
use photocraft_geom::Affine;
use serde_json::json;

fn shape(session: &Session, id: LayerId) -> &photocraft_doc::ShapeLayer {
    match &session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => sh,
        _ => panic!("expected shape"),
    }
}

fn same_outline(a: &Path, b: &Path) {
    assert_eq!(a.subpaths.len(), b.subpaths.len());
    for (a, b) in a.subpaths.iter().zip(&b.subpaths) {
        assert_eq!(a.knots.len(), b.knots.len());
        // Reflections reverse the two sides of a symmetric live arrow's contour.
        for knot in &a.knots {
            assert!(
                b.knots.iter().any(|q| (knot.anchor.x - q.anchor.x).abs() < 1e-8 && (knot.anchor.y - q.anchor.y).abs() < 1e-8),
                "missing transformed knot {:?}",
                knot.anchor
            );
        }
    }
}

#[test]
fn live_line_dimensions_follow_scale_and_remain_stable_on_next_edit() {
    for depth in [8, 16, 32] {
        for (from, to, m, length_scale, width_scale) in [
            ([20.0, 30.0], [60.0, 30.0], [2.0, 0.0, 0.0, 2.0, 10.0, 10.0], 2.0, 2.0),
            ([20.0, 30.0], [60.0, 50.0], [2.0, 0.0, 0.0, 2.0, 10.0, 10.0], 2.0, 2.0),
            ([20.0, 30.0], [60.0, 30.0], [2.0, 0.0, 0.0, 3.0, 10.0, 10.0], 2.0, 3.0),
            ([20.0, 20.0], [20.0, 60.0], [3.0, 0.0, 0.0, 2.0, 10.0, 10.0], 2.0, 3.0),
            ([20.0, 30.0], [60.0, 50.0], [-2.0, 0.0, 0.0, 2.0, 200.0, 10.0], 2.0, 2.0),
            ([20.0, 30.0], [60.0, 30.0], [-2.0, 0.0, 0.0, 3.0, 200.0, 10.0], 2.0, 3.0),
            ([20.0, 20.0], [20.0, 60.0], [0.0, 3.0, -2.0, 0.0, 200.0, 10.0], 2.0, 3.0),
        ] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":256,"height":256,"depth":depth,"background":"transparent"})).unwrap();
            let id=LayerId(s.execute("shape.create",json!({"kind":"line","from":from,"to":to,"weight":4,"fill":"#fdca22","arrowheads":{"start":true,"end":true,"width":12,"length":10,"concavity":35}})).unwrap()["layer"].as_u64().unwrap());
            let original = shape(&s, id).path.clone();
            s.execute("shape.edit", json!({"layer":id.0,"transform":m})).unwrap();
            let before = shape(&s, id).clone();
            let LiveShape::Line { from: transformed_from, to: transformed_to, weight, arrowheads } = before.live.as_ref().unwrap() else {
                panic!("line remains editable")
            };
            assert_eq!(*weight, 4.0 * width_scale);
            let heads = arrowheads.as_ref().unwrap();
            assert_eq!(heads.width, 12.0 * width_scale);
            assert_eq!(heads.length, 10.0 * length_scale);
            assert_eq!(heads.concavity, 35.0);
            assert!(heads.start && heads.end);
            let pt = |p: [f64; 2]| [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]];
            assert_eq!(*transformed_from, pt(from));
            assert_eq!(*transformed_to, pt(to));
            assert_eq!(before.path, original.transform(&Affine { m }));
            same_outline(&before.path, &photocraft_vector::shapes::live_path(before.live.as_ref().unwrap()));
            s.execute(
                "shape.edit",
                json!({"layer":id.0,"arrowheads":{"start":true,"end":true,"width":heads.width,"length":heads.length,"concavity":heads.concavity}}),
            )
            .unwrap();
            let after = shape(&s, id);
            same_outline(&before.path, &after.path);
            let a = before.cache.as_ref().unwrap();
            let b = after.cache.as_ref().unwrap();
            assert_eq!(a.content_bounds(), b.content_bounds(), "no bounds shrink: depth {depth}, matrix {m:?}");
            assert_eq!(a.format(), b.format());
            let area = a.content_bounds();
            let pixels_a = a.read_region(area);
            let pixels_b = b.read_region(area);
            for (i, (a, b)) in pixels_a.as_chunks::<4>().0.iter().zip(pixels_b.as_chunks::<4>().0).enumerate() {
                // Tiny coverage can quantize to zero alpha while retaining straight RGB.
                // Compare all visible channels and alpha, as the compositor consumes them.
                for channel in 0..4 {
                    let va = if channel == 3 { a[3] } else { a[channel] * a[3] };
                    let vb = if channel == 3 { b[3] } else { b[channel] * b[3] };
                    assert!((va - vb).abs() < 1e-5, "no visible pixel change: depth {depth}, matrix {m:?}, pixel {i}, channel {channel}: {a:?} vs {b:?}");
                }
            }
            // A real concavity edit changes only the head notch and remains one undo step.
            let prior = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            s.execute("shape.edit", json!({"layer":id.0,"arrowheads":{"start":true,"end":true,"width":heads.width,"length":heads.length,"concavity":-20}}))
                .unwrap();
            assert_eq!(s.active().unwrap().history.past_len(), history + 1);
            let edited = shape(&s, id);
            let LiveShape::Line { weight, arrowheads, .. } = edited.live.as_ref().unwrap() else { panic!("line remains live") };
            assert_eq!(*weight, 4.0 * width_scale);
            assert_eq!(arrowheads.as_ref().unwrap().width, heads.width);
            assert_eq!(arrowheads.as_ref().unwrap().length, heads.length);
            let changed = s.active().unwrap().doc.clone();
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc, prior);
            assert!(s.redo());
            assert_eq!(s.active().unwrap().doc, changed);
        }
    }
}

#[test]
fn anisotropic_diagonal_lines_keep_exact_path_without_inaccurate_live_metadata() {
    let live = LiveShape::Line {
        from: [20.0, 30.0],
        to: [60.0, 50.0],
        weight: 4.0,
        arrowheads: Some(photocraft_doc::vector::LineArrowheads { start: true, end: true, width: 12.0, length: 10.0, concavity: 35.0 }),
    };
    let path = photocraft_vector::shapes::live_path(&live);
    for m in [[2.0, 0.0, 0.0, 3.0, 10.0, 10.0], [-2.0, 0.0, 0.0, 3.0, 200.0, 10.0], [0.0, 3.0, -2.0, 0.0, 200.0, 10.0], [0.0, 0.0, 0.0, 2.0, 0.0, 0.0]] {
        let mut sh = photocraft_doc::ShapeLayer { path: path.clone(), live: Some(live.clone()), ..Default::default() };
        photocraft_engine::vector_cmds::transform_shape(&mut sh, &Affine { m });
        assert!(sh.live.is_none(), "unrepresentable transform {m:?}");
        assert_eq!(sh.path, path.transform(&Affine { m }));
    }
}

#[test]
fn quarter_turn_of_zero_length_stroked_line_keeps_path_and_rejects_live_geometry_edit() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":128,"height":128,"depth":depth,"background":"transparent"})).unwrap();
        let id = LayerId(
            s.execute("shape.create", json!({"kind":"line","from":[40,40],"to":[40,40],"weight":4,"fill":"#fdca22","stroke":{"width":2,"fill":"#000000"}}))
                .unwrap()["layer"]
                .as_u64()
                .unwrap(),
        );
        let original = shape(&s, id).path.clone();
        let m = [0.0, 1.0, -1.0, 0.0, 120.0, 10.0];
        s.execute("shape.edit", json!({"layer":id.0,"transform":m})).unwrap();
        let transformed = shape(&s, id);
        assert!(transformed.live.is_none());
        assert_eq!(transformed.path, original.transform(&Affine { m }));
        let cache = transformed.cache.as_ref().unwrap();
        assert!(
            cache.read_region(cache.content_bounds()).chunks_exact(cache.format().channels()).any(|p| p.last().copied().unwrap() > 0.5),
            "degenerate line stroke remains visible at depth {depth}"
        );
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        for edit in [json!({"weight":4}), json!({"arrowheads":{"end":true,"width":12,"length":10}})] {
            assert!(s.execute("shape.edit", edit).is_err(), "requires an explicit kind or path after orientation is lost");
            assert_eq!(s.active().unwrap().doc, before);
            assert_eq!(s.active().unwrap().history.past_len(), history);
        }
    }
}

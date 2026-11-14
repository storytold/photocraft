use super::shapes::render;
use super::*;

fn flat(w: usize, h: usize, f: impl Fn(usize, usize) -> [f32; 3]) -> Vec<[f32; 4]> {
    (0..w * h)
        .map(|i| {
            let c = f(i % w, i / w);
            [c[0], c[1], c[2], 1.0]
        })
        .collect()
}

fn at(v: &[f32], w: usize, x: usize, y: usize) -> f32 {
    v[y * w + x]
}

fn comp(shape: Shape) -> Component {
    Component { op: Op::Add, invert: false, shape }
}

#[test]
fn linear_gradient_runs_from_zero_to_full() {
    let (w, h) = (100, 50);
    let px = flat(w, h, |_, _| [0.5; 3]);
    let v = render(&Shape::Linear { zero: [0.2, 0.5], full: [0.8, 0.5] }, &px, w, h);
    assert!(at(&v, w, 5, 25) < 1e-6);
    assert!(at(&v, w, 95, 25) > 0.999);
    assert!((at(&v, w, 50, 25) - 0.5).abs() < 0.05);
}

#[test]
fn radial_gradient_is_on_inside_and_off_outside() {
    let (w, h) = (100, 80);
    let px = flat(w, h, |_, _| [0.5; 3]);
    let v = render(&Shape::Radial { center: [0.5, 0.5], radius: [0.3, 0.3], angle: 0.0, feather: 50.0 }, &px, w, h);
    assert!(at(&v, w, 50, 40) > 0.999);
    assert!(at(&v, w, 2, 2) < 1e-6);
    let inv = Mask {
        components: vec![Component { invert: true, ..comp(Shape::Radial { center: [0.5, 0.5], radius: [0.3, 0.3], angle: 0.0, feather: 50.0 }) }],
        ..Mask::default()
    };
    let c = coverage(&inv, &px, w, h);
    assert!(at(&c, w, 50, 40) < 1e-6 && at(&c, w, 2, 2) > 0.999);
}

#[test]
fn brush_builds_up_to_density_and_erases() {
    let (w, h) = (60, 60);
    let px = flat(w, h, |_, _| [0.5; 3]);
    let dab = Dab { x: 0.5, y: 0.5, size: 0.2, feather: 0.0, flow: 50.0, density: 80.0, erase: false, auto: false };
    let v = render(&Shape::Brush { dabs: vec![dab; 60] }, &px, w, h);
    assert!((at(&v, w, 30, 30) - 0.8).abs() < 0.01, "{}", at(&v, w, 30, 30));
    assert_eq!(at(&v, w, 1, 1), 0.0);
    let erased = render(&Shape::Brush { dabs: vec![dab, Dab { erase: true, flow: 100.0, ..dab }] }, &px, w, h);
    assert!(at(&erased, w, 30, 30) < 1e-6);
}

#[test]
fn auto_mask_keeps_the_brush_on_its_colour() {
    let (w, h) = (60, 40);
    let px = flat(w, h, |x, _| if x < 30 { [0.8, 0.2, 0.2] } else { [0.2, 0.3, 0.8] });
    let dab = Dab { x: 0.45, y: 0.5, size: 0.3, feather: 0.0, flow: 100.0, density: 100.0, erase: false, auto: true };
    let v = render(&Shape::Brush { dabs: vec![dab] }, &px, w, h);
    assert!(at(&v, w, 25, 20) > 0.99, "same colour painted");
    assert!(at(&v, w, 35, 20) < 0.01, "other colour left alone");
    let plain = render(&Shape::Brush { dabs: vec![Dab { auto: false, ..dab }] }, &px, w, h);
    assert!(at(&plain, w, 35, 20) > 0.99);
}

#[test]
fn colour_and_luminance_ranges_pick_their_pixels() {
    let (w, h) = (40, 20);
    let px = flat(w, h, |x, _| if x < 20 { [0.9, 0.1, 0.1] } else { [0.1, 0.3, 0.9] });
    let v = render(&Shape::Color { samples: vec![[0.9, 0.1, 0.1]], amount: 30.0 }, &px, w, h);
    assert!(at(&v, w, 5, 5) > 0.99 && at(&v, w, 35, 5) < 0.01);
    let ramp = flat(w, h, |x, _| [x as f32 / 39.0; 3]);
    let v = render(&Shape::Luminance { range: [60.0, 100.0], smoothness: 0.0 }, &ramp, w, h);
    assert!(at(&v, w, 39, 0) > 0.99 && at(&v, w, 2, 0) < 0.01);
}

#[test]
fn components_add_subtract_and_intersect() {
    let (w, h) = (100, 10);
    let px = flat(w, h, |_, _| [0.5; 3]);
    let left = Shape::Linear { zero: [0.6, 0.5], full: [0.4, 0.5] }; // on at the left
    let right = Shape::Linear { zero: [0.4, 0.5], full: [0.6, 0.5] }; // on at the right
    let m = |ops: Vec<(Op, Shape)>| Mask { components: ops.into_iter().map(|(op, shape)| Component { op, invert: false, shape }).collect(), ..Mask::default() };
    let add = coverage(&m(vec![(Op::Add, left.clone()), (Op::Add, right.clone())]), &px, w, h);
    assert!(at(&add, w, 2, 5) > 0.99 && at(&add, w, 97, 5) > 0.99);
    let sub = coverage(&m(vec![(Op::Add, Shape::Linear { zero: [0.0, 0.5], full: [0.0001, 0.5] }), (Op::Subtract, right.clone())]), &px, w, h);
    assert!(at(&sub, w, 2, 5) > 0.99 && at(&sub, w, 97, 5) < 0.01);
    let both = coverage(&m(vec![(Op::Add, left), (Op::Intersect, right)]), &px, w, h);
    assert!(both.iter().all(|v| *v < 0.3));
    // A first component that subtracts leaves nothing.
    let none = coverage(&m(vec![(Op::Subtract, Shape::Subject)]), &px, w, h);
    assert!(none.iter().all(|v| *v == 0.0));
}

#[test]
fn local_exposure_brightens_only_under_the_mask() {
    let (w, h) = (64, 32);
    let mut px = flat(w, h, |_, _| [0.4, 0.4, 0.4]);
    let p = CameraRaw {
        masks: vec![Mask {
            components: vec![comp(Shape::Linear { zero: [0.45, 0.5], full: [0.55, 0.5] })],
            adjust: Local { exposure: 1.0, ..Local::default() },
            ..Mask::default()
        }],
        ..CameraRaw::default()
    };
    assert!(!p.is_identity());
    apply(&mut px, w, h, &p, false);
    assert!((px[16 * w + 2][0] - 0.4).abs() < 1e-4, "left untouched");
    assert!(px[16 * w + 60][0] > 0.5, "right brighter: {}", px[16 * w + 60][0]);
    // Amount 0 or a hidden mask: nothing.
    let mut q = flat(w, h, |_, _| [0.4; 3]);
    let mut hidden = p.clone();
    hidden.masks[0].visible = false;
    assert!(hidden.is_identity());
    apply(&mut q, w, h, &hidden, false);
    assert!(q.iter().all(|c| (c[0] - 0.4).abs() < 1e-6));
}

#[test]
fn ai_components_run_on_any_image() {
    let (w, h) = (48, 36);
    let px = flat(w, h, |x, y| {
        if (12..36).contains(&x) && (8..30).contains(&y) {
            [0.9, 0.5, 0.2]
        } else if y < 8 {
            [0.5, 0.7, 0.95]
        } else {
            [0.2, 0.25, 0.2]
        }
    });
    for s in [
        Shape::Subject,
        Shape::Sky,
        Shape::Background,
        Shape::People { part: PeoplePart::Person },
        Shape::People { part: PeoplePart::Face },
        Shape::People { part: PeoplePart::Skin },
        Shape::Objects { rect: [0.2, 0.2, 0.8, 0.8] },
        Shape::Objects { rect: [f32::NAN; 4] },
    ] {
        let v = render(&s, &px, w, h);
        assert_eq!(v.len(), w * h);
        assert!(v.iter().all(|x| x.is_finite() && (0.0..=1.0).contains(x)), "{s:?}");
    }
}

#[test]
fn validation_rejects_hostile_masks() {
    let ok = Mask { components: vec![comp(Shape::Subject)], ..Mask::default() };
    assert!(validate(std::slice::from_ref(&ok)).is_ok());
    let bad = [
        vec![ok.clone(); MAX_MASKS + 1],
        vec![Mask { components: vec![comp(Shape::Subject); MAX_COMPONENTS + 1], ..Mask::default() }],
        vec![Mask { components: vec![comp(Shape::Depth { range: [0.0, 1.0] })], ..Mask::default() }],
        vec![Mask { components: vec![comp(Shape::Color { samples: vec![], amount: 50.0 })], ..Mask::default() }],
        vec![Mask { components: vec![comp(Shape::Linear { zero: [f32::NAN, 0.0], full: [1.0, 1.0] })], ..Mask::default() }],
        vec![Mask { components: vec![comp(Shape::Brush { dabs: vec![Dab { size: f32::INFINITY, ..Dab::default() }] })], ..Mask::default() }],
    ];
    for b in bad {
        assert!(validate(&b).is_err());
    }
}

#[test]
fn shapes_survive_hostile_numbers() {
    let (w, h) = (16, 16);
    let px = flat(w, h, |_, _| [f32::NAN, 0.5, 2.0]);
    for s in [
        Shape::Linear { zero: [0.5, 0.5], full: [0.5, 0.5] },
        Shape::Radial { center: [0.5, 0.5], radius: [0.0, 0.0], angle: 1e30, feather: -5.0 },
        Shape::Brush { dabs: vec![Dab { size: 1e9, x: -1e9, ..Dab::default() }, Dab { x: f32::NAN, ..Dab::default() }] },
        Shape::Color { samples: vec![[9.0, -3.0, 0.5]], amount: 1e9 },
        Shape::Luminance { range: [200.0, -50.0], smoothness: -1.0 },
    ] {
        let v = render(&s, &px, w, h);
        assert!(v.iter().all(|x| x.is_finite()), "{s:?}");
    }
    assert!(render(&Shape::Subject, &[], 10, 10).iter().all(|v| *v == 0.0));
}

#[test]
fn masks_round_trip_through_json() {
    let m = Mask {
        name: "Sky".into(),
        components: vec![
            comp(Shape::Sky),
            Component { op: Op::Subtract, invert: true, shape: Shape::Radial { center: [0.3, 0.2], radius: [0.1, 0.2], angle: 15.0, feather: 30.0 } },
        ],
        adjust: Local { exposure: -0.5, dehaze: 20.0, ..Local::default() },
        ..Mask::default()
    };
    let j = serde_json::to_value(&m).unwrap();
    assert_eq!(j["components"][0]["kind"], "sky");
    assert_eq!(j["components"][1]["op"], "subtract");
    let back: Mask = serde_json::from_value(j).unwrap();
    assert_eq!(back, m);
    let parsed: Mask = serde_json::from_str(r#"{"components":[{"kind":"linear","zero":[0,0],"full":[0,1]}],"adjust":{"exposure":1}}"#).unwrap();
    assert!(parsed.visible && parsed.amount == 100.0 && parsed.is_active());
}

#[test]
fn a_tiny_dab_on_a_cell_corner_still_paints() {
    let (w, h) = (100, 100);
    let px = flat(w, h, |_, _| [0.5; 3]);
    let dab = Dab { x: 0.5, y: 0.5, size: 0.001, feather: 0.0, flow: 100.0, density: 100.0, erase: false, auto: false };
    let v = render(&Shape::Brush { dabs: vec![dab] }, &px, w, h);
    assert!(v.iter().copied().fold(0.0, f32::max) > 0.0);
}

#[test]
fn a_tiny_dab_paints_above_the_brush_grid_too() {
    // Wider than the brush grid: painted on the grid, then interpolated up. (0.5, 0.5) lands on
    // a grid-cell corner.
    let (w, h) = (2048, 1024);
    let px = flat(w, h, |_, _| [0.5; 3]);
    let dab = Dab { x: 0.5, y: 0.5, size: 0.0001, feather: 0.0, flow: 100.0, density: 100.0, erase: false, auto: false };
    let v = render(&Shape::Brush { dabs: vec![dab] }, &px, w, h);
    assert!(v.iter().copied().fold(0.0, f32::max) > 0.0, "a hard tiny dab");
    // Soft and light: each dab adds only about 1e-5 at the cells it reaches.
    let soft = Dab { feather: 100.0, flow: 20.0, ..dab };
    let v = render(&Shape::Brush { dabs: vec![soft; 100] }, &px, w, h);
    assert!(v.iter().copied().fold(0.0, f32::max) > 0.0, "soft tiny dabs");
}

#[test]
fn low_flow_dabs_build_up() {
    // Flow 2 % adds 0.02^2.75 ≈ 2e-5 a dab: small, but repeated strokes must still build up.
    let (w, h) = (100, 100);
    let px = flat(w, h, |_, _| [0.5; 3]);
    let dab = Dab { x: 0.5, y: 0.5, size: 0.1, feather: 0.0, flow: 2.0, density: 100.0, erase: false, auto: false };
    let v = render(&Shape::Brush { dabs: vec![dab; 2000] }, &px, w, h);
    assert!(at(&v, w, 50, 50) > 0.02, "{}", at(&v, w, 50, 50));
}

#[test]
fn local_contrast_and_whites_follow_the_images_tones_not_the_masks_extent() {
    // Contrast and raised Whites adapt to the tones of what is developed. A distant second
    // component widens the mask's box over brighter pixels; those the first covers must not change.
    let (w, h) = (256, 64);
    let px = flat(w, h, |x, y| [0.05 + 0.85 * x as f32 / w as f32 + 0.05 * ((x * 7 + y * 13) % 5) as f32 / 5.0; 3]);
    let near = comp(Shape::Radial { center: [0.15, 0.5], radius: [0.06, 0.15], angle: 0.0, feather: 0.0 });
    let far = comp(Shape::Radial { center: [0.9, 0.9], radius: [0.02, 0.02], angle: 0.0, feather: 0.0 });
    for adjust in [Local { contrast: 60.0, ..Local::default() }, Local { whites: 60.0, ..Local::default() }] {
        let run = |components: Vec<Component>| {
            let mut out = px.clone();
            let p = CameraRaw { masks: vec![Mask { components, adjust: adjust.clone(), ..Mask::default() }], ..CameraRaw::default() };
            apply(&mut out, w, h, &p, false);
            out
        };
        let (one, two) = (run(vec![near.clone()]), run(vec![near.clone(), far.clone()]));
        assert!(one.iter().zip(&px).any(|(a, b)| a != b), "{adjust:?} changed something");
        for y in 30..34 {
            for x in 34..42 {
                let (a, b) = (one[y * w + x], two[y * w + x]);
                assert!(a.iter().zip(&b).all(|(p, q)| (p - q).abs() < 1e-5), "{adjust:?} ({x}, {y}): {a:?} vs {b:?}");
            }
        }
    }
}

#[test]
fn local_detail_radii_follow_the_image_not_the_masks_extent() {
    // Texture's radii come from the long side of what is developed. A second, distant component
    // widens the mask's box; the pixels the first one covers must not change.
    let (w, h) = (256, 64);
    let px = flat(w, h, |x, y| [0.3 + 0.4 * ((x * 7 + y * 13) % 17) as f32 / 17.0; 3]);
    let near = comp(Shape::Radial { center: [0.15, 0.5], radius: [0.06, 0.15], angle: 0.0, feather: 0.0 });
    let far = comp(Shape::Radial { center: [0.9, 0.9], radius: [0.02, 0.02], angle: 0.0, feather: 0.0 });
    let adjust = Local { texture: 80.0, ..Local::default() };
    let run = |components: Vec<Component>| {
        let mut out = px.clone();
        let p = CameraRaw { masks: vec![Mask { components, adjust: adjust.clone(), ..Mask::default() }], ..CameraRaw::default() };
        apply(&mut out, w, h, &p, false);
        out
    };
    let (one, two) = (run(vec![near.clone()]), run(vec![near.clone(), far]));
    assert!(one.iter().zip(&px).any(|(a, b)| a != b), "texture changed something");
    for y in 30..34 {
        for x in 34..42 {
            let (a, b) = (one[y * w + x], two[y * w + x]);
            assert!(a.iter().zip(&b).all(|(p, q)| (p - q).abs() < 1e-5), "({x}, {y}): {a:?} vs {b:?}");
        }
    }
}

#[test]
fn segmentation_ignores_the_colour_of_transparent_pixels() {
    // Same visible image, different colours stored under fully transparent pixels.
    let (w, h) = (48, 36);
    let subject = |x: usize, y: usize| (12..36).contains(&x) && (8..30).contains(&y);
    let image = |hidden: [f32; 3]| -> Vec<[f32; 4]> {
        (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                if subject(x, y) { [0.9, 0.5, 0.2, 1.0] } else { [hidden[0], hidden[1], hidden[2], 0.0] }
            })
            .collect()
    };
    let (a, b) = (image([0.9, 0.5, 0.2]), image([0.05, 0.05, 0.05]));
    for s in [Shape::Subject, Shape::Background] {
        assert_eq!(render(&s, &a, w, h), render(&s, &b, w, h), "{s:?}");
    }
}

#[test]
fn validation_bounds_the_brushes_total_work() {
    let dab = Dab { x: 0.5, y: 0.5, size: MAX_DAB_SIZE, feather: 0.0, flow: 100.0, density: 100.0, erase: false, auto: false };
    let brush = || Component { op: Op::Add, invert: false, shape: Shape::Brush { dabs: vec![dab; MAX_DABS] } };
    let one = Mask { components: vec![brush()], ..Mask::default() };
    assert!(validate(std::slice::from_ref(&one)).is_ok());
    let full = Mask { components: (0..MAX_COMPONENTS).map(|_| brush()).collect(), ..Mask::default() };
    assert!(validate(&vec![full; MAX_MASKS]).is_err());
}

#[test]
fn people_without_face_detection_covers_nothing_even_inverted() {
    let (w, h) = (40, 30);
    let px = flat(w, h, |_, _| [0.5; 3]);
    let people = Component { op: Op::Add, invert: true, shape: Shape::People { part: PeoplePart::Person } };
    for invert in [false, true] {
        let m = Mask { components: vec![people.clone()], invert, ..Mask::default() };
        assert!(coverage_with(&m, &px, w, h, false, Encoding::of(0.0)).iter().all(|v| *v == 0.0), "mask invert {invert}");
    }
    // With a detector, an inverted People component that finds no one covers everything.
    let m = Mask { components: vec![people], ..Mask::default() };
    if crate::faces::AVAILABLE {
        assert!(coverage_with(&m, &px, w, h, true, Encoding::of(0.0)).iter().all(|v| *v > 0.99));
    }
}

#[test]
fn colour_and_luminance_ranges_read_the_document_encoding() {
    // The same picture as an sRGB, a ProPhoto and a linear-sRGB document selects the same pixels.
    let (w, h) = (64, 16);
    let srgb = flat(w, h, |x, y| [x as f32 / (w - 1) as f32, 0.3 + 0.4 * y as f32 / (h - 1) as f32, 0.6 - 0.5 * x as f32 / (w - 1) as f32]);
    let sample = [0.55, 0.45, 0.3];
    let shapes = |to: &dyn Fn([f32; 3]) -> [f32; 3]| {
        let s = to(sample);
        [Shape::Color { samples: vec![s], amount: 30.0 }, Shape::Luminance { range: [40.0, 70.0], smoothness: 20.0 }]
    };
    let reference = shapes(&|c| c).map(|s| coverage(&Mask { components: vec![comp(s)], ..Mask::default() }, &srgb, w, h));
    for gamma in [1.8, 1.0] {
        let enc = Encoding::of(gamma);
        let doc: Vec<[f32; 4]> = srgb
            .iter()
            .map(|q| {
                let c = enc.back_from_srgb_encoded([q[0], q[1], q[2]]);
                [c[0], c[1], c[2], q[3]]
            })
            .collect();
        let p = CameraRaw { encoding_gamma: gamma, ..CameraRaw::default() };
        for (s, want) in shapes(&|c| enc.back_from_srgb_encoded(c)).into_iter().zip(&reference) {
            let got = coverage_for(&Mask { components: vec![comp(s.clone())], ..Mask::default() }, &doc, w, h, &p);
            let worst = got.iter().zip(want).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
            assert!(worst < 0.02, "gamma {gamma} {s:?}: differs by {worst}");
            assert!(want.iter().any(|v| *v > 0.9) && want.iter().any(|v| *v < 0.1), "{s:?} selects part of the picture");
        }
    }
    // A mask's local develop keeps the document's primaries.
    let base = CameraRaw { encoding_gamma: 1.8, document_primaries: Some([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]), ..CameraRaw::default() };
    assert_eq!(as_global(&Local::default(), &base).document_primaries, base.document_primaries);
}

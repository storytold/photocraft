use super::*;

fn prepared_crops(variant: impl Fn(f32) -> f32) -> Vec<PreparedImage> {
    let (w, h) = (700, 300);
    let mut source = tests::scene(w, h, 123);
    let mut rng = Rng(9);
    for v in &mut source {
        let noise = ((rng.next() % 21) as f32 - 10.0) / 255.0;
        *v = ((*v + noise).clamp(0.0, 1.0) * 255.0).round() / 255.0;
    }
    [0, 200, 400]
        .into_iter()
        .map(|x| {
            let mut pixels = Vec::new();
            for row in source.chunks_exact(w) {
                pixels.extend(row[x..x + 300].iter().map(|&v| if x == 200 { variant(v) } else { v }));
            }
            (300, 300, pixels, None)
        })
        .collect()
}

#[test]
fn auto_uses_a_straight_shift_when_overlaps_only_change_tone() {
    let cases: [fn(f32) -> f32; 3] = [|p| p, |p| ((p * 2f32.sqrt()).min(1.0) * 255.0).round() / 255.0, |p| (p * 224.0 + 12.0).round() / 255.0];
    for change in cases {
        let inputs = prepared_crops(change);
        let feats = detect_many(&inputs, 800);
        let matches = match_pairs(&feats);
        let options = AlignOptions { layout: Layout::Auto, focal: None, reference: None, geometric: false };
        let alignment = align_translated_crops(&inputs, &feats, &matches, &options).expect("the overlaps support a pure shift");
        let bounds: Vec<_> = alignment.placements.iter().map(|p| p.as_ref().unwrap().bounds(300.0, 300.0)).collect();
        for (i, b) in bounds.iter().enumerate() {
            let expected = [bounds[0][0] + 200.0 * i as f64, bounds[0][1], bounds[0][0] + 200.0 * i as f64 + 300.0, bounds[0][1] + 300.0];
            assert!(b.iter().zip(expected).all(|(a, e)| (a - e).abs() < 1e-6), "{bounds:?}");
        }
    }
}

#[test]
fn verified_features_do_not_force_a_shift_when_the_overlap_disagrees() {
    let mut inputs = prepared_crops(|p| p);
    let feats = detect_many(&inputs, 800);
    let matches = match_pairs(&feats);
    inputs[1].2 = tests::scene(300, 300, 55);
    let options = AlignOptions { layout: Layout::Auto, focal: None, reference: None, geometric: false };
    assert!(align_translated_crops(&inputs, &feats, &matches, &options).is_none());
}

#[test]
fn a_dominant_integer_offset_does_not_hide_scale_in_a_narrow_overlap() {
    let (width, height) = (1200, 160);
    let (shift, scale) = (1100.0, 1.015);
    let image = |shift: f64, scale: f64| {
        let pixels = (0..height)
            .flat_map(|y| {
                (0..width).map(move |x| {
                    let x = shift + scale * x as f64;
                    let y = y as f64;
                    (0.5 + 0.20 * (x / 13.0 + y / 9.0).sin() + 0.15 * (x / 7.0 - y / 11.0).cos()) as f32
                })
            })
            .collect();
        (width, height, pixels, None)
    };
    let images = [image(0.0, 1.0), image(shift, scale)];
    assert!(confirms_translation(&images[0], &images[1], shift as i32, 0));

    let pts_b: Vec<_> = (0..16).map(|i| [(if i < 10 { i * 3 } else { 45 + (i - 10) * 10 }) as f64, (i * 47 % height) as f64]).collect();
    let pts_a = pts_b.iter().map(|&[x, y]| [(shift + scale * x).round(), y]).collect();
    let pair = PairMatch { a: 0, b: 1, pts_a, pts_b, homography: Homography([scale, 0.0, shift, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]) };
    let feats = [ImageFeatures { w: width, h: height, ..Default::default() }, ImageFeatures { w: width, h: height, ..Default::default() }];
    let options = AlignOptions { layout: Layout::Auto, focal: None, reference: None, geometric: false };
    assert!(align_translated_crops(&images, &feats, &[pair], &options).is_none());
}

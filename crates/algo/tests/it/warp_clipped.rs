use photocraft_algo::transform::{Homography, Interp, warp_surface, warp_surface_clipped};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

#[test]
fn clipped_warp_matches_the_existing_kernel_at_every_depth() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
        source.fill_rect(Rect::new(8, 12, 40, 44), &[0.8, 0.2, 0.4, 0.7]);
        source.fill_rect(Rect::new(20, 24, 32, 36), &[0.1, 0.7, 0.3, 0.4]);
        let clip = Rect::new(10, 10, 52, 52);
        for interpolation in [Interp::Nearest, Interp::Bilinear, Interp::Bicubic] {
            for h in [Homography([1.2, -0.3, 7.0, 0.3, 1.2, -5.0, 0.0, 0.0, 1.0]), Homography([0.125, 0.0, 24.0, 0.0, 0.125, 30.0, 0.0, 0.0, 1.0])] {
                let full = warp_surface(&source, source.content_bounds(), &h, interpolation).unwrap();
                let bounded = warp_surface_clipped(&source, source.content_bounds(), &h, interpolation, clip).unwrap();
                assert_eq!(full.read_region(clip), bounded.read_region(clip));
                assert!(clip.contains_rect(&bounded.content_bounds()));
                assert!(bounded.tiles().count() <= clip.tiles().count());
            }
        }
    }
}

#[test]
fn huge_scaled_warp_allocates_only_the_destination_clip() {
    let mut source = Surface::new(PixelFormat::RGBA8);
    source.fill_rect(Rect::new(8, 8, 12, 12), &[0.2, 0.7, 0.4, 1.0]);
    let clip = Rect::new(0, 0, 64, 64);
    let h = Homography([10_000.0, 0.0, -99_968.0, 0.0, 10_000.0, -99_968.0, 0.0, 0.0, 1.0]);
    let output = warp_surface_clipped(&source, source.content_bounds(), &h, Interp::Bicubic, clip).unwrap();
    assert!(clip.contains_rect(&output.content_bounds()));
    assert!(output.tiles().count() <= clip.tiles().count());
    assert_eq!(output.pixel(32, 32), source.pixel(10, 10));
    assert!(warp_surface_clipped(&source, source.content_bounds(), &h, Interp::Bicubic, Rect::EMPTY).unwrap().tiles().next().is_none());
}

#[test]
fn inverse_footprint_handles_extreme_finite_affines() {
    let mut source = Surface::new(PixelFormat::RGBA8);
    source.fill_rect(Rect::new(0, 0, 8, 8), &[1.0, 0.0, 0.0, 1.0]);
    // A strongly skewed inverse maps a small visible tile beyond i32 limits.
    let h = Homography([1.0, 1e12, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    let clip = Rect::new(0, 0, 32, 32);
    let output = warp_surface_clipped(&source, source.content_bounds(), &h, Interp::Nearest, clip).unwrap();
    assert!(clip.contains_rect(&output.content_bounds()));
    assert!(output.tiles().count() <= clip.tiles().count());
}

#[test]
fn source_center_does_not_add_extreme_coordinates_in_i32() {
    let mut source = Surface::new(PixelFormat::RGBA8);
    let origin = 1_500_000_000;
    let bounds = Rect::new(origin, origin, origin + 4, origin + 4);
    source.fill_rect(bounds, &[0.2, 0.7, 0.4, 1.0]);
    let h = Homography([1.0, 0.0, -f64::from(origin), 0.0, 1.0, -f64::from(origin), 0.0, 0.0, 1.0]);
    let output = warp_surface_clipped(&source, bounds, &h, Interp::Nearest, Rect::new(0, 0, 8, 8)).unwrap();
    assert_eq!(output.pixel(2, 2), source.pixel(origin + 2, origin + 2));
}

/// #4864: a Bicubic Distort filters each pixel for its own reduction, so a clip, anywhere on the
/// quad (its narrow side too, or straddling the horizon), keeps exactly the pixels of a larger
/// warp that selects the same side of the horizon.
#[test]
fn perspective_clip_matches_full() {
    let mut source = Surface::new(PixelFormat::RGBA8);
    let px: Vec<f32> =
        (0..300).flat_map(|y| (0..400).flat_map(move |x| [((x * 7 + y * 3) % 23) as f32 / 22.0, ((x / 3) & 1) as f32, ((y / 2) & 1) as f32, 1.0])).collect();
    source.write_region(Rect::new(0, 0, 400, 300), &px);
    let r = Rect::new(0, 0, 400, 300);
    let wedge = Homography::rect_to_quad([0.0, 0.0, 400.0, 300.0], [[20.0, 20.0], [352.0, 240.0], [360.0, 260.0], [24.0, 280.0]]).unwrap();
    let full = warp_surface(&source, r, &wedge, Interp::Bicubic).unwrap();
    for clip in
        [Rect::new(300, 200, 370, 270), Rect::new(330, 220, 361, 262), Rect::new(17, 13, 129, 201), Rect::new(0, 0, 400, 300), Rect::new(255, 1, 258, 299)]
    {
        let bounded = warp_surface_clipped(&source, r, &wedge, Interp::Bicubic, clip).unwrap();
        assert_eq!(full.read_region(clip), bounded.read_region(clip), "{clip:?}");
        assert!(clip.contains_rect(&bounded.content_bounds()));
    }
    let past = Homography::rect_to_quad([0.0, 0.0, 400.0, 300.0], [[0.0, 0.0], [900.0, 0.0], [180.0, 180.0], [0.0, 900.0]]).unwrap();
    let big = Rect::new(-1200, -1200, 2100, 2100);
    let whole = warp_surface_clipped(&source, r, &past, Interp::Bicubic, big).unwrap();
    let same_side = Rect::new(256, 256, 1024, 768);
    let inv = past.inverse().unwrap();
    let front = past.front_side([0.0, 0.0, 400.0, 300.0]);
    assert!(
        [(same_side.x0, same_side.y0), (same_side.x1, same_side.y0), (same_side.x1, same_side.y1), (same_side.x0, same_side.y1)]
            .into_iter()
            .all(|(x, y)| front * inv.w(x as f64, y as f64) > 0.0)
    );
    for clip in [Rect::new(-50, -40, 230, 260), Rect::new(100, 100, 400, 400), same_side] {
        let bounded = warp_surface_clipped(&source, r, &past, Interp::Bicubic, clip).unwrap();
        assert_eq!(whole.read_region(clip), bounded.read_region(clip), "{clip:?}");
    }
}

/// #7557/#6376: a clip wholly on the opposite side of the horizon deliberately draws that side,
/// not the source majority chosen by a larger horizon-crossing clip. Nested clips on that side
/// still agree byte for byte, including the per-pixel Bicubic path.
#[test]
fn perspective_clips_select_their_own_side_of_the_horizon() {
    let r = Rect::new(0, 0, 400, 300);
    let h = Homography::rect_to_quad([0.0, 0.0, 400.0, 300.0], [[0.0, 0.0], [900.0, 0.0], [180.0, 180.0], [0.0, 900.0]]).unwrap();
    let inv = h.inverse().unwrap();
    let front = h.front_side([0.0, 0.0, 400.0, 300.0]);
    let clip = Rect::new(-64, -64, 0, 0);
    let inner = Rect::new(-48, -48, -16, -16);
    assert!([(clip.x0, clip.y0), (clip.x1, clip.y0), (clip.x1, clip.y1), (clip.x0, clip.y1)].into_iter().all(|(x, y)| front * inv.w(x as f64, y as f64) < 0.0));
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
        source.fill_rect(r, &[0.25, 0.5, 0.75, 1.0]);
        let expected = source.pixel(32, 24);
        for interpolation in [Interp::Nearest, Interp::Bilinear, Interp::Bicubic] {
            let whole = warp_surface_clipped(&source, r, &h, interpolation, Rect::new(-64, -64, 300, 300)).unwrap();
            let bounded = warp_surface_clipped(&source, r, &h, interpolation, clip).unwrap();
            let nested = warp_surface_clipped(&source, r, &h, interpolation, inner).unwrap();
            assert_eq!(whole.pixel(-32, -32)[3], 0.0, "{sample:?} {interpolation:?}: source majority");
            let got = bounded.pixel(-32, -32);
            assert!(got.iter().zip(&expected).all(|(a, b)| (a - b).abs() < 1e-6), "{sample:?} {interpolation:?}: {got:?}");
            assert_eq!(bounded.read_region(inner), nested.read_region(inner), "{sample:?} {interpolation:?}");
            assert!(clip.contains_rect(&bounded.content_bounds()));
        }
    }
}

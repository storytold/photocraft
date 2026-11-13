use super::*;

#[test]
fn tile_admission_checks_offsets_and_native_formats_before_writing() {
    for (mode, depth, alpha) in [
        (ColorMode::Rgb, SampleType::U8, true),
        (ColorMode::Rgb, SampleType::U16, true),
        (ColorMode::Rgb, SampleType::F32, true),
        (ColorMode::Grayscale, SampleType::U8, false),
        (ColorMode::Grayscale, SampleType::F32, false),
    ] {
        let format = PixelFormat::new(mode, depth, alpha);
        let mut surface = Surface::new(format);
        // Two logical pixels straddle the tile boundary at y=0, including negative placement.
        let rect = Rect::new(-1, -1, 0, 1);
        let cost = usize::try_from(photocraft_raster::tile_storage_bytes(rect, &format)).unwrap();
        let pixels =
            Pixels { width: 1, height: 2, channels: surface.channels(), data: vec![0.5; 2 * surface.channels()], underflow: false, coverage_clamped: false };
        let mut shortage = cost - 1;
        let error = write_pixels(&mut surface, &pixels, -1, -1, &mut shortage).unwrap_err();
        assert!(error.to_string().contains("pixel limit"));
        assert_eq!(surface.tile_count(), 0, "reject before allocating tiles");
        assert_eq!(shortage, cost - 1);
        let mut exact = cost;
        write_pixels(&mut surface, &pixels, -1, -1, &mut exact).unwrap();
        assert_eq!(surface.tile_count(), rect.tile_count() as usize);
        assert_eq!(exact, 0);
    }
}

#[test]
fn retained_tiles_share_one_budget_but_pruned_tiles_do_not_spend_it() {
    let format = PixelFormat::RGBA8;
    let cost = usize::try_from(photocraft_raster::tile_storage_bytes(Rect::new(0, 0, 1, 1), &format)).unwrap();
    let mut budget = 2 * cost;
    let mut pixels = Pixels { width: 1, height: 1, channels: 4, data: vec![0.; 4], underflow: false, coverage_clamped: false };
    for _ in 0..4 {
        let mut surface = Surface::new(format);
        write_pixels(&mut surface, &pixels, 0, 0, &mut budget).unwrap();
        assert_eq!(surface.tile_count(), 0);
        assert_eq!(budget, 2 * cost);
    }
    pixels.data = vec![1.; 4];
    let mut retained = Vec::new();
    for remaining in [cost, 0] {
        let mut surface = Surface::new(format);
        write_pixels(&mut surface, &pixels, 0, 0, &mut budget).unwrap();
        retained.push(surface);
        assert_eq!(budget, remaining);
    }
    let mut third = Surface::new(format);
    assert!(write_pixels(&mut third, &pixels, 0, 0, &mut budget).is_err());
    assert_eq!(third.tile_count(), 0);
    assert_eq!(retained.len(), 2);
}

#[test]
fn hostile_drawable_shapes_fail_before_tile_allocation() {
    let mut surface = Surface::new(PixelFormat::RGBA8);
    let mut budget = MAX_PIXEL_BYTES;
    for (width, height, channels, data) in [(u32::MAX, 1, 4, Vec::new()), (0, 1, 4, Vec::new()), (1, 1, usize::MAX, vec![0.; 4]), (1, 1, 4, Vec::new())] {
        let pixels = Pixels { width, height, channels, data, underflow: false, coverage_clamped: false };
        assert!(write_pixels(&mut surface, &pixels, 0, 0, &mut budget).is_err());
        assert_eq!(surface.tile_count(), 0);
        assert_eq!(budget, MAX_PIXEL_BYTES);
    }
}

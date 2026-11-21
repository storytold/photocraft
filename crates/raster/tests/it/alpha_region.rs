use photocraft_color::{ColorMode, PixelFormat, SampleType, write_sample};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Surface, encode_pixel};

const FORMATS: [PixelFormat; 3] = [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F];

fn oracle(surface: &Surface, area: Rect) -> bool {
    (area.y0..area.y1).any(|y| (area.x0..area.x1).any(|x| surface.rgba(x, y)[3] > 0.0))
}

#[test]
fn alpha_regions_match_pixel_oracle_for_moved_rectangles() {
    let _scratch = crate::lock(&crate::SCRATCH);
    for format in FORMATS {
        let mut surface = Surface::new(format);
        surface.fill_rect(Rect::new(-300, -270, 540, 280), &[0.7, 0.4, 0.9, 0.0]);
        assert!(!surface.has_positive_alpha(Rect::new(-300, -270, 540, 280)));
        for (i, x) in [-257, -256, -193, -192, -129, -128, -65, -64, -1, 0, 1, 63, 64, 127, 128, 191, 192, 255, 256, 511].into_iter().enumerate() {
            let y = (i as i32 * 37).rem_euclid(513) - 257;
            surface.write_pixel(x, y, &[0.0, 0.0, 0.0, 0.5]);
            for area in [Rect::new(x, y, x + 1, y + 1), Rect::new(x - 1, y, x, y + 1), Rect::new(x, y - 1, x + 1, y)] {
                assert_eq!(surface.has_positive_alpha(area), oracle(&surface, area), "{format:?}, {area:?}");
            }
        }
        // Reuse warm indexes for unrelated rectangles, not just a memoized previous footprint.
        let mut random = 0x83a2_729d_u32;
        for _ in 0..160 {
            random = random.wrapping_mul(1664525).wrapping_add(1013904223);
            let x = (random % 900) as i32 - 350;
            random = random.wrapping_mul(1664525).wrapping_add(1013904223);
            let y = (random % 650) as i32 - 320;
            let area = Rect::new(x, y, x + (random % 45) as i32, y + (random.rotate_left(9) % 39) as i32);
            assert_eq!(surface.has_positive_alpha(area), oracle(&surface, area), "{format:?}, {area:?}");
        }
        // These crops cross all four bit words and exclude ink on their half-open far edges.
        for x in 0..256 {
            let area = Rect::new(x, -270, 256, 280);
            assert_eq!(surface.has_positive_alpha(area), oracle(&surface, area), "{format:?}, {area:?}");
        }
    }
}

fn write_with(surface: &mut Surface, writer: &str, area: Rect, pixel: &[f32]) {
    let format = surface.format();
    let mut encoded = vec![0; format.bytes_per_pixel()];
    encode_pixel(&format, pixel, &mut encoded);
    match writer {
        "pixel" => surface.write_pixel(area.x0, area.y0, pixel),
        "region" => surface.write_region(area, pixel),
        "parallel_region" => surface.write_region_par(area, pixel),
        "fallible_region" => surface.try_write_region(area, pixel).unwrap(),
        "interleaved" => surface.write_interleaved(area, &encoded),
        "fallible_interleaved" => surface.try_write_interleaved(area, &encoded).unwrap(),
        "rows" => surface
            .try_write_rows_with(area, |_, _, row| {
                for out in row.chunks_exact_mut(encoded.len()) {
                    out.copy_from_slice(&encoded);
                }
            })
            .unwrap(),
        "raw" => {
            let coord = TileCoord::containing(area.x0, area.y0);
            let mut bytes = surface.tile_mut(coord).bytes_mut();
            let offset = (area.y0.rem_euclid(TILE_SIZE) as usize * TILE_SIZE as usize + area.x0.rem_euclid(TILE_SIZE) as usize) * format.bytes_per_pixel();
            bytes[offset..offset + encoded.len()].copy_from_slice(&encoded);
        }
        "fallible_raw" => {
            let coord = TileCoord::containing(area.x0, area.y0);
            let mut bytes = surface.try_tile_mut(coord).unwrap().bytes_mut();
            let base = (area.y0.rem_euclid(TILE_SIZE) as usize * TILE_SIZE as usize + area.x0.rem_euclid(TILE_SIZE) as usize) * format.channels();
            for (i, &value) in pixel.iter().enumerate() {
                write_sample(&mut bytes, format.sample, base + i, value);
            }
        }
        "fill" | "whole_tile_fill" => surface.fill_rect(area, pixel),
        _ => panic!("unknown test writer"),
    }
}

#[test]
fn alpha_index_tracks_surface_and_raw_byte_writes_after_warming() {
    let _scratch = crate::lock(&crate::SCRATCH);
    let tile = Rect::new(0, 0, TILE_SIZE, TILE_SIZE);
    for format in FORMATS {
        for writer in [
            "pixel",
            "region",
            "parallel_region",
            "fallible_region",
            "interleaved",
            "fallible_interleaved",
            "rows",
            "raw",
            "fallible_raw",
            "fill",
            "whole_tile_fill",
        ] {
            let mut surface = Surface::new(format);
            surface.fill_rect(tile, &[0.3, 0.7, 0.9, 0.0]);
            assert!(!surface.has_positive_alpha(tile));
            let area = if writer == "whole_tile_fill" { tile } else { Rect::new(63, 127, 64, 128) };
            write_with(&mut surface, writer, area, &[0.3, 0.7, 0.9, 1.0]);
            assert!(surface.has_positive_alpha(area), "{format:?}, {writer}: empty index must be invalidated");
            assert!(surface.has_positive_alpha(tile), "{format:?}, {writer}");
            write_with(&mut surface, writer, area, &[0.3, 0.7, 0.9, 0.0]);
            assert!(!surface.has_positive_alpha(tile), "{format:?}, {writer}: positive index must be invalidated");
        }
    }
}

#[test]
fn alpha_indexes_follow_clone_cow_replacement_and_translation() {
    let _scratch = crate::lock(&crate::SCRATCH);
    let tile = Rect::new(0, 0, TILE_SIZE, TILE_SIZE);
    let point = Rect::new(127, 64, 128, 65);
    for format in FORMATS {
        let mut original = Surface::new(format);
        original.fill_rect(Rect::new(0, 0, 512, 256), &[0.4, 0.7, 0.9, 0.0]);
        assert!(!original.has_positive_alpha(Rect::new(0, 0, 512, 256)));
        for writer in ["raw", "fallible_raw"] {
            let mut copy = original.clone();
            write_with(&mut copy, writer, point, &[0.0, 0.0, 0.0, 1.0]);
            assert!(copy.has_positive_alpha(point), "{format:?}, {writer}");
            assert!(!original.has_positive_alpha(tile), "COW cannot contaminate the original index");
            assert!(!copy.has_positive_alpha(Rect::new(256, 0, 512, 256)), "shared coordinates cannot contaminate each other");
        }
        let mut donor = Surface::new(format);
        donor.write_pixel(point.x0, point.y0, &[0.0, 0.0, 0.0, 1.0]);
        assert!(donor.has_positive_alpha(point));
        original.put_tiles(donor.tiles().map(|(coord, tile)| (*coord, tile.clone())));
        assert!(original.has_positive_alpha(point), "replacement uses the donor backing's coverage");
        original.fill_rect(tile, &[0.6, 0.4, 0.8, 0.0]);
        assert!(!original.has_positive_alpha(point), "a replacement cannot inherit the old index");
        for (dx, dy) in [(256, -512), (17, -19)] {
            let moved = donor.translated(dx, dy, tile);
            assert!(moved.has_positive_alpha(point.translate(dx, dy)), "{format:?}, shift {dx},{dy}");
            assert!(!moved.has_positive_alpha(point), "queries remain in document coordinates");
        }
    }
}

#[test]
fn alpha_index_keys_shared_bytes_by_pixel_format() {
    let _scratch = crate::lock(&crate::SCRATCH);
    let coord = TileCoord::new(0, 0);
    let area = coord.rect();
    let mut rgba = Surface::new(PixelFormat::RGBA8);
    rgba.fill_rect(area, &[0.0, 0.0, 1.0, 0.0]);
    assert!(!rgba.has_positive_alpha(area));
    // Both formats have four bytes per pixel, but blue becomes part of 16-bit gray alpha.
    let mut gray = Surface::new(PixelFormat::GRAYA8.with_sample(SampleType::U16));
    gray.put_tiles([(coord, rgba.tile(coord).unwrap().clone())]);
    for _ in 0..3 {
        assert!(gray.has_positive_alpha(area));
        assert!(!rgba.has_positive_alpha(area));
    }
}

#[test]
fn alpha_regions_preserve_strict_positive_float_semantics() {
    let _scratch = crate::lock(&crate::SCRATCH);
    for format in FORMATS {
        let mut surface = Surface::new(format);
        for (x, alpha) in
            [0.0, -0.0, -1.0, f32::NAN, f32::NEG_INFINITY, f32::from_bits(1), f32::MIN_POSITIVE, 1.0 / 65535.0, f32::INFINITY].into_iter().enumerate()
        {
            let x = x as i32;
            surface.write_pixel(x, 3, &[0.7, 0.8, 0.9, alpha]);
            let area = Rect::new(x, 3, x + 1, 4);
            assert_eq!(surface.has_positive_alpha(area), oracle(&surface, area), "{format:?}, {alpha:?}");
        }
    }
}

#[test]
fn alpha_regions_handle_sparse_extremes_and_overridden_default_alpha() {
    let _scratch = crate::lock(&crate::SCRATCH);
    let huge = Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    for format in FORMATS {
        let mut sparse = Surface::new(format);
        sparse.write_pixel(i32::MIN + 3, i32::MIN + 7, &[0.9, 0.2, 0.8, 0.0]);
        sparse.write_pixel(i32::MAX - 4, i32::MAX - 8, &[0.9, 0.2, 0.8, 0.0]);
        assert!(!sparse.has_positive_alpha(huge));
        sparse.write_pixel(i32::MAX - 2, i32::MIN + 1, &[0.0, 0.0, 0.0, 0.5]);
        assert!(sparse.has_positive_alpha(huge));
        assert!(sparse.has_positive_alpha(Rect::new(i32::MAX - 2, i32::MIN, i32::MAX, i32::MIN + 2)));
        assert!(!sparse.has_positive_alpha(Rect::new(i32::MIN, 0, i32::MAX, i32::MAX)));
        let mut defaults = Surface::with_default(format, &[0.1, 0.2, 0.3, 0.5]);
        let covered = Rect::new(-256, 0, 256, 256);
        defaults.fill_rect(covered, &[0.9, 0.7, 0.5, 0.0]);
        assert!(!defaults.has_positive_alpha(covered), "stored transparent tiles override visible defaults");
        assert!(!defaults.has_positive_alpha(Rect::new(-200, 10, 200, 200)));
        assert!(defaults.has_positive_alpha(Rect::new(-256, 0, 257, 256)), "one missing tile contributes default alpha");
        assert!(defaults.has_positive_alpha(huge));
    }
}

#[test]
fn alpha_regions_without_alpha_are_opaque_for_stored_and_missing_pixels() {
    let _scratch = crate::lock(&crate::SCRATCH);
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk] {
        for sample in SampleType::ALL {
            let format = PixelFormat::new(mode, sample, false);
            let mut surface = Surface::new(format);
            surface.fill_rect(Rect::new(-256, 0, 0, 256), &vec![0.0; format.channels()]);
            for area in [Rect::new(-200, 10, -100, 20), Rect::new(500, 500, 501, 501), Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX)] {
                assert!(surface.has_positive_alpha(area), "{format:?}, {area:?}");
            }
            assert!(!surface.has_positive_alpha(Rect::EMPTY));
            assert!(!surface.has_positive_alpha(Rect::new(5, 5, 4, 6)));
        }
    }
}

#[test]
fn alpha_regions_ignore_unaddressable_tile_coordinates() {
    let _scratch = crate::lock(&crate::SCRATCH);
    for format in FORMATS {
        let mut surface = Surface::new(format);
        let opaque = surface.solid_tile(&[0.0, 0.0, 0.0, 1.0]);
        surface.put_tiles([(TileCoord::new(i32::MIN, i32::MIN), opaque)]);
        let area = Rect::new(i32::MIN, i32::MIN, i32::MIN + 512, i32::MIN + 512);
        assert!(!surface.has_positive_alpha(area), "out-of-range tile rects cannot alias the first addressable tile");
        surface.write_pixel(i32::MIN, i32::MIN, &[0.0, 0.0, 0.0, 1.0]);
        assert!(surface.has_positive_alpha(area), "the actual minimum-coordinate pixel remains addressable");
    }
}

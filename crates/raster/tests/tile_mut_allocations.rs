//! Keep allocation accounting in a separate test binary with one non-ignored test so other
//! tests cannot allocate concurrently inside a measured region.
use std::alloc::System;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use photocraft_color::{PixelFormat, SampleType};
use photocraft_geom::TileCoord;
use photocraft_raster::Surface;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

#[test]
fn existing_unique_tiles_do_not_allocate_on_mutable_access_or_pixel_writes() {
    for sample in SampleType::ALL {
        for format in [PixelFormat::GRAY8, PixelFormat::RGBA8].map(|f| f.with_sample(sample)) {
            let default = vec![1.0; format.channels()];
            let changed = vec![0.0; format.channels()];
            let mut surface = Surface::with_default(format, &default);
            let coord = TileCoord::containing(-3, 700);

            // The vacant path must still initialize the entire tile with the encoded default.
            surface.tile_mut(coord);
            assert_eq!(surface.pixel(-3, 700), default);
            assert_eq!(surface.pixel(-4, 700), default);
            let original = surface.clone();
            assert!(Arc::ptr_eq(surface.tile(coord).unwrap(), original.tile(coord).unwrap()));

            // Detaching a shared tile remains necessary and must preserve the snapshot.
            surface.write_pixel(-3, 700, &changed);
            assert!(!Arc::ptr_eq(surface.tile(coord).unwrap(), original.tile(coord).unwrap()));
            assert_eq!(original.pixel(-3, 700), default);
            assert_eq!(surface.pixel(-3, 700), changed);
            assert_eq!(surface.pixel(-4, 700), default);
            let tile_bytes = surface.tile(coord).unwrap().bytes().as_ptr();

            let region = Region::new(GLOBAL);
            for _ in 0..256 {
                black_box(surface.tile_mut(coord));
                surface.write_pixel(-3, 700, black_box(&changed));
            }
            let stats = region.change();
            assert_eq!(stats.allocations, 0, "{format:?}: {stats:?}");
            assert_eq!(stats.reallocations, 0, "{format:?}: {stats:?}");
            assert_eq!(surface.tile(coord).unwrap().bytes().as_ptr(), tile_bytes);
            assert_eq!(surface.tile_count(), 1);
        }
    }
}

#[test]
#[ignore = "24 MP release benchmark: run alone with --release --ignored --nocapture"]
fn ellipse_mask_24mp() {
    // Synthetic center-sampled ellipse exercising the same per-pixel write path as frame masks.
    // This excludes the Frame tool's 4x4 antialiasing, command dispatch and compositing costs.
    const WIDTH: i32 = 6000;
    const HEIGHT: i32 = 4000;
    for _ in 0..3 {
        let mut mask = Surface::new(PixelFormat::GRAY8);
        let region = Region::new(GLOBAL);
        let start = Instant::now();
        for y in 0..HEIGHT {
            let dy = (y as f64 + 0.5 - HEIGHT as f64 * 0.5) / (HEIGHT as f64 * 0.5);
            for x in 0..WIDTH {
                let dx = (x as f64 + 0.5 - WIDTH as f64 * 0.5) / (WIDTH as f64 * 0.5);
                if dx * dx + dy * dy <= 1.0 {
                    mask.write_pixel(x, y, black_box(&[1.0]));
                }
            }
        }
        let elapsed = start.elapsed();
        let stats = region.change();
        println!("24 MP ellipse: {:.3} ms, {} allocations, {} tiles", elapsed.as_secs_f64() * 1000.0, stats.allocations, mask.tile_count());
        black_box(mask);
    }
}

//! Scratch disk: tiles over the budget go to disk and come back unchanged.
//!
//! One test function: the spill manager is process-wide, so its steps must not interleave.

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Surface, spill};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const TILE_BYTES: usize = (TILE_SIZE * TILE_SIZE) as usize * 4;

fn rgba8() -> PixelFormat {
    PixelFormat { mode: ColorMode::Rgb, sample: SampleType::U8, alpha: true }
}

/// Incompressible bytes, different per tile and per seed.
fn noise(seed: u32, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

/// A surface of `n × 1` tiles of noise; tile `i` holds `noise(seed + i)`.
fn surface(seed: u32, n: i32) -> Surface {
    let mut s = Surface::new(rgba8());
    for i in 0..n {
        let r = Rect::from_xywh(i * TILE_SIZE, 0, TILE_SIZE as u32, TILE_SIZE as u32);
        s.write_interleaved(r, &noise(seed + i as u32, TILE_BYTES));
    }
    s
}

fn check(s: &Surface, seed: u32, n: i32) {
    for i in 0..n {
        let t = s.tile(TileCoord { tx: i, ty: 0 }).unwrap();
        assert!(*t.bytes() == *noise(seed + i as u32, TILE_BYTES), "tile {i} of seed {seed} changed");
    }
}

fn mixed_alpha_surface(format: PixelFormat, count: usize) -> Surface {
    let mut surface = Surface::new(format);
    for i in 0..count {
        surface.write_pixel(i as i32 * TILE_SIZE + 63, 127, &[0.0, 0.0, 0.0, 1.0]);
    }
    surface
}

fn alpha_index_accounting(format: PixelFormat) {
    const COUNT: usize = 32;
    const INDEX_BYTES: usize = (TILE_SIZE * TILE_SIZE) as usize / 8;
    let point = Rect::new(63, 127, 64, 128);
    spill::configure(None, 0).unwrap();
    let len = (TILE_SIZE * TILE_SIZE) as usize * format.bytes_per_pixel();
    let mut surface = mixed_alpha_surface(format, COUNT);
    assert_eq!(spill::stats().resident_bytes, COUNT * len);
    for i in 0..COUNT {
        assert!(surface.has_positive_alpha(point.translate(i as i32 * TILE_SIZE, 0)));
    }
    let warmed = COUNT * (len + INDEX_BYTES);
    assert_eq!(spill::stats().resident_bytes, warmed, "mixed indexes must count as reclaimable tile memory");
    if format == PixelFormat::RGBA8 {
        let mut gray = Surface::new(PixelFormat::GRAYA8.with_sample(SampleType::U16));
        gray.put_tiles([(TileCoord::new(0, 0), surface.tile(TileCoord::new(0, 0)).unwrap().clone())]);
        for _ in 0..3 {
            assert!(gray.has_positive_alpha(point));
            assert!(surface.has_positive_alpha(point));
            assert_eq!(spill::stats().resident_bytes, warmed, "format-key replacement must release the previous index");
        }
    }
    surface.write_pixel(64, 127, &[0.0, 0.0, 0.0, 1.0]);
    assert_eq!(spill::stats().resident_bytes, warmed - INDEX_BYTES, "writing releases the invalidated index");
    assert!(surface.has_positive_alpha(Rect::new(64, 127, 65, 128)));
    assert_eq!(spill::stats().resident_bytes, warmed);
    drop(surface);
    assert_eq!(spill::stats().resident_bytes, 0, "dropping backings must release their indexes too");
}

fn mixed_alpha_indexes_are_charged_reclaimed_and_rebuilt(dir: &Path, format: PixelFormat) {
    alpha_index_accounting(format);
    const COUNT: usize = 32;
    const INDEX_BYTES: usize = (TILE_SIZE * TILE_SIZE) as usize / 8;
    let point = Rect::new(63, 127, 64, 128);
    let len = (TILE_SIZE * TILE_SIZE) as usize * format.bytes_per_pixel();
    let surface = mixed_alpha_surface(format, COUNT);
    spill::configure(Some(dir), COUNT * len).unwrap();
    let before = spill::stats().evictions;
    assert!(surface.has_positive_alpha(point));
    assert!(spill::stats().evictions > before, "building an index must itself trigger budget enforcement");
    assert!(spill::stats().resident_bytes <= COUNT * len);
    let budget = len * 2;
    spill::set_budget(budget);
    for _ in 0..2 {
        for i in 0..COUNT {
            let area = point.translate(i as i32 * TILE_SIZE, 0);
            assert!(surface.has_positive_alpha(area));
            assert!(!surface.has_positive_alpha(area.translate(1, 0)), "rebuilding must remain exact across bitmap words");
            assert!(spill::stats().resident_bytes <= budget, "many queried mixed tiles cannot retain unbounded indexes");
        }
    }
    spill::set_budget(1);
    spill::settle();
    assert_eq!(spill::stats().resident_bytes, 0, "spilling pixels must reclaim all heap-backed coverage");
    spill::set_budget(len + INDEX_BYTES);
    let bytes = surface.tile(TileCoord::new(0, 0)).unwrap().try_bytes().unwrap();
    assert_eq!(spill::stats().resident_bytes, len, "a cold read must not retain a spilled mixed index");
    assert!(surface.has_positive_alpha(point));
    assert!(!surface.has_positive_alpha(point.translate(1, 0)));
    assert_eq!(spill::stats().resident_bytes, len + INDEX_BYTES, "coverage is rebuilt and charged only once");
    drop(bytes);
    spill::set_budget(1);
    spill::settle();
    assert_eq!(spill::stats().resident_bytes, 0);
    assert!(surface.has_positive_alpha(point), "a query remains exact even when its rebuilt index must immediately spill");
    assert_eq!(spill::stats().resident_bytes, 0);
    drop(surface);
    assert_eq!(spill::stats().resident_bytes, 0);
}

fn warmed_alpha_recovers_from_crc_failure_without_a_write(dir: &Path) {
    let coord = TileCoord::new(0, 0);
    let point = Rect::new(63, 127, 64, 128);
    for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        for mixed in [false, true] {
            spill::configure(None, 0).unwrap();
            let mut surface = Surface::new(format);
            surface.fill_rect(coord.rect(), &[1.0, 0.0, 0.0, if mixed { 0.0 } else { 1.0 }]);
            if mixed {
                surface.write_pixel(point.x0, point.y0, &[1.0, 0.0, 0.0, 1.0]);
            }
            assert!(surface.has_positive_alpha(point));
            let scratch_dir = dir.join(format!("alpha-read-{:?}-{mixed}", format.sample));
            spill::configure(Some(&scratch_dir), 1).unwrap();
            spill::settle();
            assert_eq!(spill::stats().resident_bytes, 0);
            if !mixed {
                let reloads = spill::stats().reloads;
                assert!(surface.has_positive_alpha(point));
                assert_eq!(spill::stats().reloads, reloads, "healthy uniform coverage keeps its allocation-free spilled fast path");
            }
            let mut file = spill::scratch_handle_for_tests().unwrap();
            file.seek(SeekFrom::Start(0)).unwrap();
            let mut good = Vec::new();
            file.read_to_end(&mut good).unwrap();
            assert!(!good.is_empty());
            file.seek(SeekFrom::Start(0)).unwrap();
            file.write_all(&[good[0] ^ 1]).unwrap();
            file.sync_all().unwrap();

            let tile = surface.tile(coord).unwrap();
            let error = tile.try_bytes().err().unwrap();
            assert!(error.contains("CRC mismatch"), "{error}");
            let failed_generation = spill::read_error_generation();
            assert!(tile.scratch_health().is_err());
            assert!(!surface.has_positive_alpha(point), "known poison must override even a warmed uniform index");
            assert!(tile.bytes().iter().all(|&byte| byte == 0), "failed reads expose only transparent placeholder bytes");
            assert_eq!(surface.rgba(point.x0, point.y0), [0.0; 4]);
            assert_eq!(spill::read_error_generation(), failed_generation, "placeholder queries are not new failures");

            file.seek(SeekFrom::Start(0)).unwrap();
            file.write_all(&good).unwrap();
            file.sync_all().unwrap();
            tile.try_bytes().unwrap();
            assert!(tile.scratch_health().is_ok());
            assert!(surface.has_positive_alpha(point), "a successful read restores ink without bytes_mut or any pixel write");
            assert_eq!(surface.rgba(point.x0, point.y0), [1.0, 0.0, 0.0, 1.0]);
            assert!(spill::check_since(failed_generation).is_err(), "read-only recovery invalidates placeholder-derived computations");
            assert!(spill::check_integrity().is_ok());
            drop((surface, file));
            assert_eq!(spill::stats().resident_bytes, 0);
        }
    }
}

#[test]
fn tiles_spill_and_come_back() {
    let _scratch = crate::lock(&crate::SCRATCH);
    let dir = std::env::temp_dir().join(format!("photocraft-spill-test-{}", std::process::id()));
    let budget = 8 * TILE_BYTES;
    spill::configure(Some(&dir), budget).unwrap();
    assert!(spill::enabled());

    // 1. Over budget: the oldest tiles go to disk, resident memory drops to the budget.
    let a = surface(1, 32);
    let b = surface(1000, 32);
    spill::settle();
    let st = spill::stats();
    assert!(st.resident_bytes <= budget, "resident {} > budget {budget}", st.resident_bytes);
    assert!(st.evictions >= 48, "only {} evictions", st.evictions);
    assert!(st.scratch_bytes > 0 && st.last_error.is_none(), "{st:?}");

    // 2. Every pixel reads back exactly (CRC-checked, LZ4 round trip), through every reader.
    check(&a, 1, 32);
    check(&b, 1000, 32);
    assert!(spill::stats().reloads > 0);
    let mut row = vec![[0u8; 4]; TILE_SIZE as usize];
    a.read_rgba8_into(Rect::from_xywh(5 * TILE_SIZE, 7, TILE_SIZE as u32, 1), &mut row);
    let want = noise(6, TILE_BYTES);
    let off = 7 * TILE_SIZE as usize * 4;
    assert!(row.iter().flatten().eq(want[off..off + TILE_SIZE as usize * 4].iter()));

    // Upstream's fallible readers and clones must reload evicted storage too.
    spill::settle();
    let rect = Rect::from_xywh(0, 0, TILE_SIZE as u32, TILE_SIZE as u32);
    assert_eq!(a.try_to_interleaved(rect).unwrap(), noise(1, TILE_BYTES));
    assert_eq!(a.try_read_region(rect).unwrap(), a.read_region(rect));
    let copied = a.tile(TileCoord::new(0, 0)).unwrap().try_clone().unwrap();
    assert_eq!(&*copied.try_bytes().unwrap(), &noise(1, TILE_BYTES));
    drop(copied);

    // 3. Copy-on-write still works on spilled tiles: the clone changes, the original doesn't.
    spill::settle();
    let mut c = a.clone();
    c.fill_rect(Rect::from_xywh(0, 0, 10, 10), &[1.0, 0.0, 0.0, 1.0]);
    assert_eq!(c.pixel(3, 3), vec![1.0, 0.0, 0.0, 1.0]);
    check(&a, 1, 32);
    spill::settle();
    // The edit survives its own eviction (the stale disk copy was released, a new one written).
    assert_eq!(c.pixel(3, 3), vec![1.0, 0.0, 0.0, 1.0]);
    check(&a, 1, 32);

    // 4. Parallel readers of spilled tiles all see the right bytes.
    spill::settle();
    std::thread::scope(|scope| {
        for k in 0..8 {
            let (a, b) = (&a, &b);
            scope.spawn(move || {
                for round in 0..4 {
                    let (s, seed) = if (k + round) % 2 == 0 { (a, 1) } else { (b, 1000) };
                    check(s, seed, 32);
                }
            });
        }
    });

    // 5. Dropping everything frees memory and scratch space.
    drop((a, b, c));
    let st = spill::stats();
    assert_eq!(st.scratch_bytes, 0, "scratch extents leaked: {st:?}");

    // 6. Off: nothing more is evicted, and the budget no longer applies.
    spill::configure(None, 0).unwrap();
    assert!(!spill::enabled());
    let before = spill::stats().evictions;
    let d = surface(5000, 32);
    assert_eq!(spill::stats().evictions, before);
    check(&d, 5000, 32);
    drop(d);

    // 7. Encoded bytes and COW edits survive eviction at every supported depth.
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let fmt = PixelFormat { mode: ColorMode::Rgb, sample, alpha: true };
        let len = (TILE_SIZE * TILE_SIZE) as usize * fmt.bytes_per_pixel();
        spill::configure(Some(&dir), len * 2).unwrap();
        let mut s = Surface::new(fmt);
        for i in 0..12 {
            s.tile_mut(TileCoord::new(i, 0)).bytes_mut().copy_from_slice(&noise(8000 + i as u32, len));
        }
        let original = s.clone();
        spill::settle();
        assert!(spill::stats().resident_bytes <= len * 2);
        s.fill_rect(Rect::from_xywh(0, 0, 4, 4), &[1.0, 0.0, 0.0, 1.0]);
        spill::settle();
        assert_eq!(s.pixel(1, 1), vec![1.0, 0.0, 0.0, 1.0]);
        for i in 0..12 {
            assert_eq!(&*original.tile(TileCoord::new(i, 0)).unwrap().bytes(), &noise(8000 + i as u32, len));
        }
        let mut converted = [[0; 4]; 4];
        s.read_rgba8_into(Rect::from_xywh(0, 0, 4, 1), &mut converted);
        assert_eq!(converted, [[255, 0, 0, 255]; 4]);
    }
    // A tiny budget must still allow a pinned reader to finish instead of evicting itself.
    spill::configure(Some(&dir), 1).unwrap();
    let tiny = surface(9000, 2);
    spill::settle();
    check(&tiny, 9000, 2);
    drop(tiny);
    spill::configure(None, 0).unwrap();
    for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        mixed_alpha_indexes_are_charged_reclaimed_and_rebuilt(&dir, format);
    }
    warmed_alpha_recovers_from_crc_failure_without_a_write(&dir);
    spill::configure(None, 0).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

//! Streamed native allocation controls; fixtures never allocate a whole source image.
use super::*;
use craft_xcf::samples::{SampleOrder, SourceDrawable, SourcePrecision, SourceTile, Transfer};

/// Direct, specification-framed solid RLE runs. Only the small pointer table and
/// compressed records are stored, including for the opt-in 8192-square case.
pub(super) fn solid_fixture(width: u32, height: u32, samples: &[u8], layers: usize) -> Vec<u8> {
    assert!(matches!(samples.len(), 3 | 4));
    let mut w = W::new(26);
    for word in [width, height, 0, 150] {
        w.u32(word);
    }
    w.prop(17, &[1]);
    w.end();
    let layers: Vec<_> = (0..layers).map(|_| w.slot()).collect();
    w.ptr(0);
    w.ptr(0);
    w.ptr(0);
    for (index, layer) in layers.into_iter().enumerate() {
        let at = w.out.len();
        w.patch(layer, at);
        for word in [width, height, if samples.len() == 3 { 0 } else { 1 }] {
            w.u32(word);
        }
        w.string(&format!("Solid {index}"));
        w.end();
        let hierarchy = w.slot();
        w.ptr(0);
        w.ptr(0);
        let at = w.out.len();
        w.patch(hierarchy, at);
        for word in [width, height, samples.len() as u32] {
            w.u32(word);
        }
        let level = w.slot();
        w.ptr(0);
        let at = w.out.len();
        w.patch(level, at);
        w.u32(width);
        w.u32(height);
        let across = width.div_ceil(TILE) as usize;
        let down = height.div_ceil(TILE) as usize;
        let tiles: Vec<_> = (0..across * down).map(|_| w.slot()).collect();
        w.ptr(0);
        for (index, tile) in tiles.into_iter().enumerate() {
            let tw = (width - (index % across) as u32 * TILE).min(TILE);
            let th = (height - (index / across) as u32 * TILE).min(TILE);
            let count = tw * th;
            let at = w.out.len();
            w.patch(tile, at);
            for sample in samples {
                if count <= 127 {
                    w.out.push((count - 1) as u8);
                } else {
                    w.out.push(127);
                    w.out.extend_from_slice(&(count as u16).to_be_bytes());
                }
                w.out.push(*sample);
            }
        }
    }
    w.out
}

#[test]
fn solid_rle_fixture_covers_edge_tiles_and_appends_only_opaque_alpha() {
    for color in [&[17, 31, 57][..], &[17, 31, 57, 128][..]] {
        let bytes = solid_fixture(65, 67, color, 1);
        let imported = open(&bytes).unwrap();
        let alpha = if color.len() == 3 { 1.0 } else { 128. / 255. };
        for (x, y) in [(0, 0), (63, 63), (64, 66)] {
            assert_eq!(pixel(&imported.document.layers[0], x, y), [17. / 255., 31. / 255., 57. / 255., alpha]);
        }
    }
}

fn first_tile(data: &[u8]) -> SourceTile<'_> {
    SourceTile {
        x: 0,
        y: 0,
        width: 64,
        height: 64,
        drawable: SourceDrawable {
            width: 256,
            height: 256,
            channels: 3,
            indexed: false,
            precision: SourcePrecision { storage: craft_xcf::Precision::GAMMA8, transfer: Transfer::Nonlinear },
            sample_order: SampleOrder::BigEndian,
        },
        data,
    }
}

#[test]
fn streamed_opaque_hierarchy_admits_the_exact_logical_budget() {
    let data = [17, 31, 57].repeat(64 * 64);
    let tile = first_tile(&data);
    let logical = 256 * 256 * 4 * std::mem::size_of::<f32>();
    let mut writer = TileWriter::new(PixelFormat::RGBA8, (0, 0), &[], true, false, logical);
    writer.write(tile).unwrap();
    let mut budget = logical;
    let (surface, clamped) = writer.finish(&mut budget);
    assert!(!clamped);
    assert_eq!(surface.pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    assert_eq!(logical - budget, photocraft_raster::stored_tiles_bytes(surface.tile_count() as u64, &PixelFormat::RGBA8) as usize);
    let mut writer = TileWriter::new(PixelFormat::RGBA8, (0, 0), &[], true, false, logical - 1);
    assert!(writer.write(tile).is_err());
    let mut budget = logical - 1;
    let (surface, _) = writer.finish(&mut budget);
    assert_eq!(surface.tile_count(), 0);
    assert_eq!(budget, logical - 1);
}

#[test]
fn streamed_offset_tiles_include_live_scratch_in_native_allocation_admission() {
    let data = [17, 31, 57].repeat(64 * 64);
    let tile = first_tile(&data);
    let native = photocraft_raster::tile_storage_bytes(Rect::new(1, 1, 257, 257), &PixelFormat::RGBA8) as usize;
    let scratch = 64 * 64 * 4 * std::mem::size_of::<f32>() + data.len();
    let physical = native + scratch;
    let mut writer = TileWriter::new(PixelFormat::RGBA8, (1, 1), &[], true, false, physical - 1);
    assert!(writer.write(tile).is_err());
    let mut budget = physical - 1;
    let (surface, _) = writer.finish(&mut budget);
    assert_eq!(surface.tile_count(), 0);
    assert_eq!(budget, physical - 1);
    let mut writer = TileWriter::new(PixelFormat::RGBA8, (1, 1), &[], true, false, physical);
    writer.write(tile).unwrap();
    let mut budget = physical;
    let (surface, _) = writer.finish(&mut budget);
    assert_eq!(surface.pixel(0, 0), vec![0.; 4]);
    assert_eq!(surface.pixel(1, 1), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
}

/// Opt-in actual boundary import, with about 320 KiB of compressed source data.
/// Run in release: cargo test -p photocraft-io --release --lib
/// xcf::tests::streamed_raster::opaque_rgb_8192_square_imports_at_the_logical_limit -- --ignored --exact
#[test]
#[ignore = "allocates the real 256 MiB native surface; explicit release qualification"]
fn opaque_rgb_8192_square_imports_at_the_logical_limit() {
    let bytes = solid_fixture(8192, 8192, &[17, 31, 57], 1);
    assert!(bytes.len() < 1 << 20, "the fixture itself must remain compact");
    let started = std::time::Instant::now();
    let imported = open(&bytes).unwrap();
    assert_eq!((imported.document.size.width, imported.document.size.height), (8192, 8192));
    assert_eq!(imported.document.layers.len(), 1);
    let layer = &imported.document.layers[0];
    for (x, y) in [(0, 0), (63, 63), (64, 64), (4096, 4096), (8191, 8191)] {
        assert_eq!(pixel(layer, x, y), [17. / 255., 31. / 255., 57. / 255., 1.]);
    }
    let LayerContent::Raster(surface) = &layer.content else { panic!("editable native pixels") };
    assert_eq!(surface.content_bounds(), Rect::new(0, 0, 8192, 8192));
    assert_eq!(photocraft_raster::stored_tiles_bytes(surface.tile_count() as u64, &surface.format()), 256 << 20);
    eprintln!("opaque-rgb-8192: source_bytes={} native_bytes={} elapsed_ms={}", bytes.len(), 256 << 20, started.elapsed().as_secs_f64() * 1000.);
}

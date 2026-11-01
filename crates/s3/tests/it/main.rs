#![cfg(not(target_arch = "wasm32"))]

mod fake;
mod minio;
mod store;

use std::sync::Arc;

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, Size};
use photocraft_format::{LoadOptions, PcraftWriter, Result, SaveOptions, load_bytes_source};
use photocraft_raster::Rgba8Image;
use photocraft_s3::{S3Config, S3Store, is_conflict};

/// A small document over three tiles, its pixels and ICC profile varied by `seed`.
pub fn doc(seed: u8) -> Document {
    let mut d = Document::with_background("S3", Size::new(600, 300), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    d.icc_profile = Some(Arc::new(vec![seed; 300]));
    let mut paint = Layer::raster("Paint", d.pixel_format());
    let surface = paint.surface_mut().unwrap();
    for x0 in [0, 260, 520] {
        for i in 0..64 {
            let v = f32::from(seed.wrapping_add(i as u8)) / 255.0;
            surface.write_pixel(x0 + i % 8, i / 8, &[v, 1.0 - v, 0.5, 1.0]);
        }
    }
    d.layers.push(paint);
    d
}

pub fn with_previews() -> SaveOptions {
    SaveOptions { thumbnail: Some(Rgba8Image::new(4, 3)), composite: Some(Rgba8Image::new(9, 5)) }
}

pub fn load(store: &S3Store) -> Result<Document> {
    load_bytes_source(store, &LoadOptions::default())
}

/// What any S3-compatible service must do for a bundle: save, reopen unchanged, save again
/// uploading nothing new, refuse a save over someone else's newer version, and share the pool.
/// `pool` and `prefix` must be unused keys.
pub fn bundle_lifecycle(config: &S3Config, pool: &str, prefix: &str) {
    let document = format!("{prefix}doc/");
    let (original, edited, other_edit) = (doc(1), doc(2), doc(3));

    let mut alice = S3Store::new(config.clone(), pool, document.as_str()).unwrap();
    let mut alice_writer = PcraftWriter::new();
    let first = alice_writer.save_to(&original, &mut alice, &with_previews()).unwrap();
    assert_eq!(first.tiles_written, first.tiles_total);
    assert!(first.tiles_total >= 3 && first.blobs_written >= 1, "{first:?}");
    let head = alice.head().expect("a save records the head it wrote");

    let mut bob = S3Store::new(config.clone(), pool, document.as_str()).unwrap();
    assert_eq!(load(&bob).unwrap(), original);
    assert_eq!(bob.head().as_deref(), Some(head.as_str()));

    // Unchanged: no object goes up again, the head moves.
    let again = alice_writer.save_to(&original, &mut alice, &SaveOptions::default()).unwrap();
    assert_eq!((again.tiles_written, again.tiles_reused, again.blobs_written), (0, first.tiles_total, 0));
    assert_ne!(alice.head().as_deref(), Some(head.as_str()));

    // Alice saved after Bob opened: Bob's save fails and the head stays Alice's.
    alice_writer.save_to(&edited, &mut alice, &SaveOptions::default()).unwrap();
    let err = PcraftWriter::new().save_to(&other_edit, &mut bob, &SaveOptions::default()).unwrap_err();
    assert!(is_conflict(&err), "{err}");
    let carol = S3Store::new(config.clone(), pool, document.as_str()).unwrap();
    assert_eq!(load(&carol).unwrap(), edited);

    // A store that never read the document can't create it over the existing one.
    let mut dave = S3Store::new(config.clone(), pool, document.as_str()).unwrap();
    assert!(is_conflict(&PcraftWriter::new().save_to(&other_edit, &mut dave, &SaveOptions::default()).unwrap_err()));

    // Bob reopens and saves on top of Alice's version.
    assert_eq!(load(&bob).unwrap(), edited);
    PcraftWriter::new().save_to(&other_edit, &mut bob, &SaveOptions::default()).unwrap();
    assert_eq!(load(&carol).unwrap(), other_edit);

    // A second document with the same content finds every object in the pool.
    let mut copy = S3Store::new(config.clone(), pool, format!("{prefix}copy/")).unwrap();
    let stats = PcraftWriter::new().save_to(&other_edit, &mut copy, &SaveOptions::default()).unwrap();
    assert_eq!((stats.tiles_written, stats.tiles_reused, stats.blobs_written), (0, stats.tiles_total, 0), "{stats:?}");
    assert_eq!(load(&copy).unwrap(), other_edit);
}

use super::*;
use photocraft_doc::{HistoricalXcfSampleOrder as Order, StackMode};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::jobs::historical_xcf_fixture as fixture;

struct SourceFile(PathBuf);

impl SourceFile {
    fn new(bytes: &[u8]) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-smart-xcf-order-{}-{}.xcf", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
        file.write_all(bytes).unwrap();
        Self(path)
    }
}

impl Drop for SourceFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn pixel(order: Order) -> [f32; 4] {
    let rgb = match order {
        Order::BigEndian => [0x1234u16, 0x4567, 0x89ab],
        Order::LittleEndian => [0x3412u16, 0x6745, 0xab89],
    };
    [f32::from(rgb[0]) / 65535., f32::from(rgb[1]) / 65535., f32::from(rgb[2]) / 65535., 1.]
}

fn assert_pixel(actual: &[f32], expected: [f32; 4]) {
    assert_eq!(actual.len(), 4);
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(actual.is_finite());
        assert!((*actual - expected).abs() <= 2. / 65535., "{actual} != {expected}");
    }
}

fn clear_caches() {
    purge_source_cache();
    purge_stack_cache();
}

fn gaussian() -> SmartFilter {
    SmartFilter { command: "filter.blur.gaussianBlur".into(), params: json!({"radius":1}), blend: BlendMode::Normal, opacity: 1., visible: true }
}

fn canvas() -> Document {
    Document::new("Historical smart object", Size::new(4, 4), ColorMode::Rgb, SampleType::U16)
}

fn bits(surface: &Surface) -> Vec<u32> {
    let samples = surface.read_region(Rect::new(0, 0, 4, 4));
    assert!(samples.iter().all(|sample| sample.is_finite()));
    samples.into_iter().map(f32::to_bits).collect()
}

#[test]
fn public_place_retains_historical_order_through_cold_render_and_native_roundtrip() {
    for little in [false, true] {
        let bytes = fixture::historical_pixel(little);
        let file = SourceFile::new(&bytes);
        let (name, order) = if little { ("little-endian", Order::LittleEndian) } else { ("big-endian", Order::BigEndian) };
        for command in ["file.placeEmbedded", "file.placeLinked"] {
            let mut session = Session::new();
            session.execute("file.new", json!({"width":4,"height":4,"depth":16})).unwrap();
            session
                .execute(command, json!({"path":file.0,"historicalXcfSampleOrder":name,"smartObject":true,"fit":false,"scale":100,"center":[0.5,0.5]}))
                .unwrap();
            let mut sm = active_smart(&session);
            assert_eq!(sm.historical_xcf_sample_order, Some(order), "{command}, {name}");
            assert_pixel(&sm.cache.as_ref().unwrap().pixel(0, 0), pixel(Order::BigEndian));
            let doc = session.active().unwrap().doc.as_ref();
            sm.cache = None;
            clear_caches();
            let cold = render(doc, &sm).unwrap().unwrap();
            assert_pixel(&cold.pixel(0, 0), pixel(Order::BigEndian));

            // Rebuild from encoded data after all appearance caches are gone,
            // including transform, smart-filter and stack-mode decode paths.
            sm.transform = Affine::translate(1., 1.);
            sm.stack_mode = Some(StackMode::Mean);
            sm.smart_filters = vec![gaussian()];
            sm.filters_enabled = true;
            let mut stored = doc.clone();
            let id = session.active().unwrap().active_layer.unwrap();
            stored.layer_mut(id).unwrap().content = LayerContent::Smart(sm.clone());
            clear_caches();
            let expected = bits(&render(&stored, &sm).unwrap().unwrap());
            assert!(expected.iter().any(|bits| *bits != 0), "fixture must render nonempty pixels");
            let saved = photocraft_format::save_to_bytes(&stored, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            let LayerContent::Smart(mut restored) = reopened.layer(id).unwrap().content.clone() else { panic!("native smart object") };
            assert_eq!(restored.historical_xcf_sample_order, Some(order));
            assert_eq!(restored.stack_mode, Some(StackMode::Mean));
            assert!(restored.cache.is_none());
            restored.cache = None;
            clear_caches();
            assert_eq!(bits(&render(&reopened, &restored).unwrap().unwrap()), expected, "{command}, {name}");
        }
    }
}

#[test]
fn identical_bytes_with_opposite_orders_do_not_share_decoded_or_stack_sources() {
    let bytes = fixture::historical_pixel(false);
    let fmt = PixelFormat::RGBA16;
    clear_caches();
    // Both interpretations are finite normalized values. A successful wrong
    // interpretation therefore really warms the cache rather than failing first.
    for order in [Order::LittleEndian, Order::BigEndian, Order::LittleEndian, Order::BigEndian] {
        let image = source_image_with_order("identical.xcf", &bytes, fmt, Some(order)).unwrap();
        assert_pixel(&image.surface.pixel(0, 0), pixel(order));
    }
    assert!(source_image_with_order("identical.xcf", &bytes, fmt, None).is_err(), "warm explicit choices must not authorize an unspecified choice");
    for order in [Order::LittleEndian, Order::BigEndian, Order::LittleEndian, Order::BigEndian] {
        let image = stack_image_with_order("identical.xcf", &bytes, fmt, StackMode::Mean, Some(order)).unwrap();
        assert_pixel(&image.surface.pixel(0, 0), pixel(order));
    }
    assert!(stack_image_with_order("identical.xcf", &bytes, fmt, StackMode::Mean, None).is_err());
    clear_caches();
    assert!(source_image_with_order("identical.xcf", &bytes, fmt, None).is_err(), "a cold unspecified choice must still require an explicit answer");
    assert!(stack_image_with_order("identical.xcf", &bytes, fmt, StackMode::Mean, None).is_err());
}

#[test]
fn placed_and_filtered_stages_separate_opposite_choices_and_never_guess_none() {
    let doc = canvas();
    let bytes = fixture::historical_pixel(false);
    for stack_mode in [None, Some(StackMode::Mean)] {
        for filtered in [false, true] {
            let mut right = SmartObject::new(
                SmartSource::Embedded { file_name: "same-source.xcf".into(), bytes: Arc::new(bytes.clone()) },
                Affine::translate(1., 1.),
                None,
            );
            right.historical_xcf_sample_order = Some(Order::BigEndian);
            right.stack_mode = stack_mode;
            if filtered {
                right.smart_filters = vec![gaussian()];
            }
            let mut wrong = right.clone();
            wrong.historical_xcf_sample_order = Some(Order::LittleEndian);
            let mut unspecified = right.clone();
            unspecified.historical_xcf_sample_order = None;
            // Also make the identity contract independent of unrelated tests
            // purging process-wide caches between these tiny renders.
            let right_key = super::super::stack_cache::placed_key(&bytes, doc.pixel_format(), &right, doc.bounds());
            assert_ne!(right_key, super::super::stack_cache::placed_key(&bytes, doc.pixel_format(), &wrong, doc.bounds()));
            assert_ne!(right_key, super::super::stack_cache::placed_key(&bytes, doc.pixel_format(), &unspecified, doc.bounds()));
            clear_caches();
            let correct = bits(&render(&doc, &right).unwrap().unwrap());
            clear_caches();
            let opposite = bits(&render(&doc, &wrong).unwrap().unwrap());
            assert_ne!(correct, opposite, "the two finite interpretations must produce distinct stages");
            assert_eq!(bits(&render(&doc, &right).unwrap().unwrap()), correct, "a warm wrong-order stage cannot replace the right order");
            assert_eq!(bits(&render(&doc, &wrong).unwrap().unwrap()), opposite);
            assert!(render(&doc, &unspecified).is_err(), "a warm Some stage cannot make None succeed");
            clear_caches();
            assert!(render(&doc, &unspecified).is_err(), "None must fail on a real decode miss too");
        }
    }
}

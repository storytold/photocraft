use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const SIDE: u32 = 512;
const HEIGHT: u32 = 1024;

/// One original layer, 128 uncompressed tiles, and no optional metadata or nested source.
fn tiled_source(version: u32, little: bool) -> Vec<u8> {
    fn word(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    fn slot(bytes: &mut Vec<u8>) -> usize {
        let at = bytes.len();
        word(bytes, 0);
        at
    }
    fn patch(bytes: &mut [u8], at: usize, value: usize) {
        bytes[at..at + 4].copy_from_slice(&(value as u32).to_be_bytes());
    }
    let mut bytes = if version == 7 { b"gimp xcf v007\0".to_vec() } else { b"gimp xcf v003\0".to_vec() };
    for value in [SIDE, HEIGHT, 0] {
        word(&mut bytes, value);
    }
    if version == 7 {
        word(&mut bytes, 250);
    }
    word(&mut bytes, 17);
    word(&mut bytes, 1);
    bytes.push(0);
    word(&mut bytes, 0);
    word(&mut bytes, 0);
    let layer = slot(&mut bytes);
    word(&mut bytes, 0);
    word(&mut bytes, 0);
    let at = bytes.len();
    patch(&mut bytes, layer, at);
    for value in [SIDE, HEIGHT, 1, 2] {
        word(&mut bytes, value);
    }
    bytes.extend_from_slice(b"X\0");
    word(&mut bytes, 0);
    word(&mut bytes, 0);
    let hierarchy = slot(&mut bytes);
    word(&mut bytes, 0);
    let at = bytes.len();
    patch(&mut bytes, hierarchy, at);
    for value in [SIDE, HEIGHT, if version == 7 { 8 } else { 4 }] {
        word(&mut bytes, value);
    }
    let level = slot(&mut bytes);
    word(&mut bytes, 0);
    let at = bytes.len();
    patch(&mut bytes, level, at);
    word(&mut bytes, SIDE);
    word(&mut bytes, HEIGHT);
    let tiles: Vec<_> = (0..128).map(|_| slot(&mut bytes)).collect();
    word(&mut bytes, 0);
    let pixel: Vec<u8> = if version == 7 {
        [0x1234u16, 0x5678, 0x9abc, 0xffff].into_iter().flat_map(|sample| if little { sample.to_le_bytes() } else { sample.to_be_bytes() }).collect()
    } else {
        vec![17, 31, 57, 255]
    };
    let tile = pixel.repeat(64 * 64);
    for slot in tiles {
        let at = bytes.len();
        patch(&mut bytes, slot, at);
        bytes.extend_from_slice(&tile);
    }
    // Establish a valid no-cancel oracle before any cancellation assertion, independently
    // of the smart-source wrapper whose forwarding is under test.
    let options = crate::ImportOptions {
        historical_xcf_sample_order: (version == 7).then_some(if little { Order::LittleEndian } else { Order::BigEndian }),
        ..Default::default()
    };
    let decoded = crate::import_with_options_and_interrupt("fixture.xcf", &bytes, &options, &Interrupt::NONE)
        .unwrap_or_else(|error| panic!("invalid generated XCF{version} fixture: {error:?}"));
    assert_eq!(decoded.document.size, Size::new(SIDE, HEIGHT));
    let expected = if version == 7 {
        [0x1234u16, 0x5678, 0x9abc, 0xffff].map(|sample| f32::from(sample) / 65535.)
    } else {
        [17., 31., 57., 255.].map(|sample| sample / 255.)
    };
    assert_eq!(decoded.document.layers[0].surface().unwrap().pixel(511, 1023), expected);
    bytes
}

#[derive(Clone, Copy)]
enum Trigger {
    Layer(f32),
    Poll(usize),
}

/// The reader checks once per 64 tiles. Cancel on the second check after its layer progress:
/// 64 real tiles have decoded, while the second half remains unread.
fn during_decode<R: std::fmt::Debug>(trigger: Trigger, operation: impl FnOnce(Interrupt<'_>) -> R) -> R {
    let polls = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let armed = AtomicBool::new(matches!(trigger, Trigger::Poll(_)));
    let completed = AtomicBool::new(false);
    let cancel = || {
        if cancelled.load(Ordering::Relaxed) {
            return true;
        }
        if !armed.load(Ordering::Relaxed) {
            return false;
        }
        let target = match trigger {
            Trigger::Layer(_) => 2,
            Trigger::Poll(count) => count,
        };
        if polls.fetch_add(1, Ordering::Relaxed) + 1 == target {
            cancelled.store(true, Ordering::Relaxed);
        }
        cancelled.load(Ordering::Relaxed)
    };
    let progress = |value: f32| {
        if matches!(trigger, Trigger::Layer(expected) if value == expected) {
            armed.store(true, Ordering::Relaxed);
        }
        if value == 1. {
            completed.store(true, Ordering::Relaxed);
        }
    };
    let result = operation(Interrupt::new(&cancel, &progress));
    let target = match trigger {
        Trigger::Layer(_) => 2,
        Trigger::Poll(count) => count,
    };
    assert_eq!(polls.load(Ordering::Relaxed), target, "the source decoder must observe cancellation before completion: {result:?}");
    assert!(cancelled.load(Ordering::Relaxed));
    assert!(armed.load(Ordering::Relaxed), "reach the source layer before cancelling");
    assert!(!completed.load(Ordering::Relaxed), "do not finish decoding the source before noticing cancellation");
    result
}

/// The legacy geometry helper intentionally suppresses progress. Calibrate only its fixed
/// pre-layer checks against the existing importer, never against the forwarding under test.
fn before_layer_polls(bytes: &[u8]) -> usize {
    let polls = AtomicUsize::new(0);
    let before = AtomicUsize::new(usize::MAX);
    let cancel = || {
        polls.fetch_add(1, Ordering::Relaxed);
        false
    };
    let progress = |value: f32| {
        if value == 0.05 {
            before.store(polls.load(Ordering::Relaxed), Ordering::Relaxed);
        }
    };
    let imported = crate::import_with("ordinary.xcf", bytes, &Interrupt::new(&cancel, &progress)).unwrap();
    assert_eq!(imported.document.size, Size::new(SIDE, HEIGHT));
    let prefix = before.load(Ordering::Relaxed);
    assert!(prefix >= 2 && prefix < polls.load(Ordering::Relaxed));
    assert!(polls.load(Ordering::Relaxed) >= prefix + 2, "the valid source contains both hierarchy checkpoints");
    prefix
}

#[test]
fn historical_decode_cancels_mid_tiles_while_geometry_never_reads_them() {
    for (order, little) in [(Order::LittleEndian, true), (Order::BigEndian, false)] {
        let bytes = tiled_source(7, little);
        let result = during_decode(Trigger::Layer(0.05), |ctl| crate::smart_xcf_order::decode("source.xcf", &bytes, Some(order), &ctl));
        assert!(matches!(result, Err(crate::IoError::Cancelled)));
        let doc = crate::smart_xcf_order::decode("source.xcf", &bytes, Some(order), &Interrupt::NONE).unwrap();
        assert_eq!(doc.size, Size::new(SIDE, HEIGHT));
        assert_eq!(doc.layers[0].surface().unwrap().pixel(511, 1023), [0x1234u16, 0x5678, 0x9abc, 0xffff].map(|sample| f32::from(sample) / 65535.));
        assert_eq!(crate::smart_xcf_order::geometry("source.xcf", &bytes, Some(order), &Interrupt::NONE).unwrap(), ((512., 1024.), 72.));
    }
}

#[test]
fn geometry_without_order_cancels_in_header_and_conversion_still_cancels_in_pixels() {
    let bytes = tiled_source(3, false);
    let prefix = before_layer_polls(&bytes);
    // Geometry checks its wrapper, then the XCF reader, then the first image property.
    let result = during_decode(Trigger::Poll(3), |ctl| crate::smart_xcf_order::geometry("ordinary.xcf", &bytes, None, &ctl));
    assert!(matches!(result, Err(crate::IoError::Cancelled)));
    assert_eq!(crate::smart_xcf_order::geometry("ordinary.xcf", &bytes, None, &Interrupt::NONE).unwrap(), ((512., 1024.), 72.));
    let doc = document(SmartSource::Embedded { file_name: "ordinary.xcf".into(), bytes: Arc::new(bytes.clone()) }, None);
    // embed_hashed and convert_source contribute three checks before the legacy import.
    during_decode(Trigger::Poll(prefix + 3 + 2), |ctl| {
        let mut ex = exporter_with_interrupt(&doc, ctl);
        assert!(ex.embed_hashed(smart(&doc).contents_id, "ordinary.xcf", &bytes, false, embed_source_hash(&bytes), None).is_err());
        assert!(ex.smart.add.is_empty());
        assert!(ex.smart.converted.is_empty());
    });
}

#[test]
fn cancelled_source_conversion_and_composite_do_not_cache_fallbacks_or_copied_sources() {
    for (order, little) in [(Order::LittleEndian, true), (Order::BigEndian, false)] {
        let bytes = Arc::new(tiled_source(7, little));
        let doc = document(SmartSource::Embedded { file_name: "source.xcf".into(), bytes: bytes.clone() }, Some(order));
        during_decode(Trigger::Layer(0.05), |ctl| {
            let mut ex = exporter_with_interrupt(&doc, ctl);
            assert!(ex.embed_hashed(smart(&doc).contents_id, "source.xcf", &bytes, false, embed_source_hash(&bytes), Some(order)).is_err());
            assert!(ex.smart.add.is_empty(), "a cancelled embedded decode must not fall back to copying its source");
            assert!(ex.smart.docs.is_empty());
            assert!(ex.smart.converted.is_empty(), "cancellation is not a cached unsupported-source result");
        });
        during_decode(Trigger::Layer(0.05), |ctl| {
            let mut ex = exporter_with_interrupt(&doc, ctl);
            ex.smart.add.push(crate::linked::LinkedFile { uuid: "source".into(), file_name: "source.xcf".into(), bytes: bytes.as_ref().clone() });
            assert!(ex.source_composite("source", Some(order)).is_none());
            assert!(ex.smart.composites.is_empty(), "a cancelled decode must not cache None");
            assert!(ex.smart.docs.is_empty());
        });
        let uuid = "known-source";
        let mut known = document(SmartSource::Linked { path: uuid.into() }, Some(order));
        known.metadata.psd_global_blocks.push((
            *b"8BIM",
            *b"lnk2",
            Arc::new(crate::linked::encode_linked_file(&crate::linked::LinkedFile {
                uuid: uuid.into(),
                file_name: "source.xcf".into(),
                bytes: bytes.as_ref().clone(),
            })),
        ));
        // The retained source already has geometry in its header: cancellation occurs while
        // reading image properties, rather than decoding its tile hierarchy.
        during_decode(Trigger::Poll(4), |ctl| {
            let mut ex = exporter_with_interrupt(&known, ctl);
            assert!(ex.smart_source(smart(&known), None).is_err());
            assert!(ex.smart.linked_contents.is_empty(), "cancelled geometry must not proceed through a default-size fallback");
            assert!(ex.smart.linked_copies.is_empty());
        });
        let mut control = exporter(&known);
        assert_eq!(control.smart_source(smart(&known), None).unwrap().size, (512., 1024.));

        let cancelled = AtomicBool::new(false);
        let cancel = || cancelled.load(Ordering::Relaxed);
        let mut warm = exporter_with_interrupt(&doc, Interrupt::cancel_only(&cancel));
        let hash = embed_source_hash(&bytes);
        let source = warm.embed_hashed(smart(&doc).contents_id, "source.xcf", &bytes, false, hash, Some(order)).unwrap();
        assert!(warm.source_composite(&source.uuid, Some(order)).is_some());
        let cached = (warm.smart.converted.len(), warm.smart.composites.len());
        cancelled.store(true, Ordering::Relaxed);
        assert!(warm.embed_hashed(smart(&doc).contents_id, "source.xcf", &bytes, false, hash, Some(order)).is_err());
        assert!(warm.source_composite(&source.uuid, Some(order)).is_none());
        assert_eq!((warm.smart.converted.len(), warm.smart.composites.len()), cached);
    }
}

#[test]
fn public_psd_encoding_cancels_before_a_model_can_be_written_and_controls_are_byte_identical() {
    for (order, little) in [(Order::LittleEndian, true), (Order::BigEndian, false)] {
        let doc = document(SmartSource::Embedded { file_name: "source.xcf".into(), bytes: Arc::new(tiled_source(7, little)) }, Some(order));
        let opts = crate::ExportOptions { image_previews: false, ..Default::default() };
        let mut output = std::io::Cursor::new(Vec::new());
        let result = during_decode(Trigger::Layer(0.05 * crate::PSD_MODEL_DONE), |ctl| crate::encode_with(&doc, "psd", &opts, &ctl));
        match result {
            Err(crate::IoError::Cancelled) => {}
            Ok(exported) => {
                exported.encoded.write_to(&mut output).unwrap();
                panic!("cancelled source encoding returned a writable model");
            }
            other => panic!("expected typed cancellation, got {other:?}"),
        }
        assert!(output.into_inner().is_empty());
        let plain = crate::export(&doc, "psd", &opts).unwrap();
        let uninterrupted = crate::export_with(&doc, "psd", &opts, &Interrupt::NONE).unwrap();
        assert_eq!(uninterrupted.bytes, plain.bytes);
        assert_eq!(uninterrupted.warnings, plain.warnings);
    }
}

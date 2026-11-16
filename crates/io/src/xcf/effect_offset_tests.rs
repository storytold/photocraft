//! Invalid optional effect offsets must not discard mandatory drawable pixels.

use super::tests::{W, hierarchy};
use super::*;
use photocraft_doc::import_conversion::ImportConversion;

/// Independent v26 wire fixture with ordinary pixels in all three drawable owners.
/// raw=0 omits effects; raw=1 points beyond EOF; raw=2 points exactly at EOF.
fn fixture(owner: usize, raw: u64) -> (Vec<u8>, u64) {
    let mut w = W::new(26);
    for value in [1, 1, 0, 150] {
        w.u32(value);
    }
    w.prop(17, &[0]);
    w.end();
    let layer_slot = w.slot();
    w.ptr(0);
    let channel_slot = w.slot();
    w.ptr(0);
    w.ptr(0); // No standalone paths.
    let mut invalid_slot = None;
    let mut effect_list = |w: &mut W, current| {
        if current == owner && raw != 0 {
            invalid_slot = Some(w.slot());
        }
        w.ptr(0);
    };
    let at = w.out.len();
    w.patch(layer_slot, at);
    for value in [1, 1, 1] {
        w.u32(value);
    }
    w.string("Layer pixels");
    w.end();
    let layer_pixels = w.slot();
    let mask_slot = w.slot();
    effect_list(&mut w, 0);
    let at = hierarchy(&mut w, 1, 1, 4, &[17, 31, 57, 255], Compression::None);
    w.patch(layer_pixels, at);
    let at = w.out.len();
    w.patch(mask_slot, at);
    w.u32(1);
    w.u32(1);
    w.string("Layer mask pixels");
    w.end();
    let mask_pixels = w.slot();
    effect_list(&mut w, 1);
    let at = hierarchy(&mut w, 1, 1, 1, &[128], Compression::None);
    w.patch(mask_pixels, at);
    let at = w.out.len();
    w.patch(channel_slot, at);
    w.u32(1);
    w.u32(1);
    w.string("Channel pixels");
    w.end();
    let channel_pixels = w.slot();
    effect_list(&mut w, 2);
    let at = hierarchy(&mut w, 1, 1, 1, &[192], Compression::None);
    w.patch(channel_pixels, at);
    let pointer = match raw {
        1 => w.out.len() as u64 + 1,
        2 => w.out.len() as u64,
        value => value,
    };
    if let Some(slot) = invalid_slot {
        w.out[slot..slot + 8].copy_from_slice(&pointer.to_be_bytes());
    }
    (w.out, pointer)
}

fn assert_pixels(document: &Document) {
    assert_eq!(document.layers.len(), 1);
    let layer = &document.layers[0];
    assert_eq!(layer.surface().unwrap().read_region(Rect::new(0, 0, 1, 1)), [17. / 255., 31. / 255., 57. / 255., 1.]);
    assert_eq!(layer.mask.as_ref().unwrap().surface.read_region(Rect::new(0, 0, 1, 1)), [128. / 255.]);
    assert_eq!(document.channels.len(), 1);
    assert_eq!(document.channels[0].surface.read_region(Rect::new(0, 0, 1, 1)), [192. / 255.]);
}

#[test]
fn invalid_layer_mask_and_channel_effect_offsets_preserve_pixels_and_native_report() {
    for owner in 0..3 {
        for raw in [1, 2, u64::MAX] {
            let (bytes, pointer) = fixture(owner, raw);
            let imported = crate::import("optional-offset.xcf", &bytes).expect("an invalid optional record cannot discard drawable pixels");
            assert_pixels(&imported.document);
            assert!(imported.source_read_only);
            assert!(!imported.preview_only);
            assert!(imported.warnings.iter().any(|warning| warning.contains("effect")));
            let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("typed report") };
            report.validate().unwrap();
            let feature = if owner == 0 { "xcf.layer_effect" } else { "xcf.coverage_effect" };
            let entry = report.entries.iter().find(|entry| entry.feature_code == feature).unwrap();
            assert_eq!(entry.reason, ConversionReason::InvalidOptionalData);
            assert_eq!(entry.source.record_offset, Some(usize::try_from(pointer).unwrap_or(usize::MAX) as u64));
            assert!(entry.native_description.contains("preserved"));
            let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            assert_pixels(&reopened);
            assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
        }
    }
}

#[test]
fn ordinary_drawables_without_effects_and_cancelled_import_remain_unchanged() {
    let (bytes, _) = fixture(0, 0);
    let imported = crate::import("plain-v26.xcf", &bytes).unwrap();
    assert_pixels(&imported.document);
    let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("typed report") };
    assert!(!report.entries.iter().any(|entry| matches!(entry.feature_code.as_str(), "xcf.layer_effect" | "xcf.coverage_effect")));
    let cancelled = Interrupt::cancel_only(&|| true);
    let (invalid, _) = fixture(2, u64::MAX);
    assert!(matches!(crate::import_with("cancelled.xcf", &invalid, &cancelled), Err(IoError::Cancelled)));
}

fn pointer_list(raw: &[u64]) -> Vec<u8> {
    raw.iter().flat_map(|pointer| pointer.to_be_bytes()).collect()
}

#[test]
fn optional_pointer_lists_keep_structural_limits_and_metadata_charges() {
    let decode = |bytes: &[u8], budget: &mut usize| effects::read_pointers(&mut Cursor { b: bytes, pos: 0, wide: true }, budget, &Interrupt::NONE);
    let bytes = pointer_list(&[u64::MAX, u64::MAX - 1, 0]);
    let mut budget = 1024;
    let pointers = decode(&bytes, &mut budget).unwrap();
    assert_eq!(pointers.len(), 2, "distinct raw offsets must not become duplicate sentinels on 32-bit hosts");
    let charge = 4 * std::mem::size_of::<usize>().max(std::mem::size_of::<u64>());
    assert_eq!(budget, 1024 - 2 * charge);
    assert!(decode(&bytes, &mut (charge - 1)).is_err(), "metadata admission remains mandatory");
    assert!(decode(&pointer_list(&[u64::MAX, u64::MAX, 0]), &mut 1024).unwrap_err().to_string().contains("repeated"));
    let missing = pointer_list(&[u64::MAX]);
    for end in 0..=missing.len() {
        assert!(decode(&missing[..end], &mut 1024).is_err(), "a partial or unterminated pointer list is structural failure");
    }
    let mut maximum: Vec<_> = (1..=1024).collect();
    maximum.push(0);
    assert_eq!(decode(&pointer_list(&maximum), &mut (128 << 10)).unwrap().len(), 1024);
    maximum.pop();
    maximum.extend([1025, 0]);
    assert!(decode(&pointer_list(&maximum), &mut (128 << 10)).unwrap_err().to_string().contains("1,024"));
    let cancelled = Interrupt::cancel_only(&|| true);
    let mut budget = 1024;
    assert!(matches!(effects::read_pointers(&mut Cursor { b: &bytes, pos: 0, wide: true }, &mut budget, &cancelled), Err(IoError::Cancelled)));
    assert_eq!(budget, 1024, "cancellation before a pointer must not charge its storage");
}

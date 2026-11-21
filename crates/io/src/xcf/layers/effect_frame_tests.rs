use crate::xcf::tests::{W, hierarchy};
use crate::xcf::*;
use photocraft_doc::import_conversion::{ConversionDisposition, ConversionReason, ImportConversion};

fn source(width: u32, height: u32, x: i32, y: i32) -> Vec<u8> {
    let mut writer = W::new(26);
    for value in [1, 1, 0, 150] {
        writer.u32(value);
    }
    writer.prop(17, &[0]);
    writer.end();
    let group_slot = writer.slot();
    let pixel_slot = writer.slot();
    writer.ptr(0);
    writer.ptr(0);
    writer.ptr(0);
    let group = writer.out.len();
    writer.patch(group_slot, group);
    for value in [width, height, 1] {
        writer.u32(value);
    }
    writer.string("Optional frame");
    writer.prop(29, &[]);
    writer.prop(15, &[x.to_be_bytes(), y.to_be_bytes()].concat());
    writer.end();
    writer.ptr(0);
    writer.ptr(0);
    let effect_slot = writer.slot();
    writer.ptr(0);
    let pixel = writer.out.len();
    writer.patch(pixel_slot, pixel);
    for value in [1, 1, 1] {
        writer.u32(value);
    }
    writer.string("Usable pixels");
    writer.end();
    let hierarchy_slot = writer.slot();
    writer.ptr(0);
    writer.ptr(0);
    let pixels = hierarchy(&mut writer, 1, 1, 4, &[17, 31, 57, 255], Compression::None);
    writer.patch(hierarchy_slot, pixels);
    let effect = writer.out.len();
    writer.patch(effect_slot, effect);
    writer.string("Inert valid effect");
    writer.string("");
    writer.string("gegl:invert-linear");
    writer.string("1");
    writer.end();
    writer.ptr(0);
    writer.out
}

#[test]
fn optional_effect_frame_overflow_preserves_independent_pixels_and_native_report() {
    for (width, height, x, y) in [(u32::MAX, 1, 0, 0), (1, u32::MAX, 0, 0), (2, 1, i32::MAX, 0), (1, 2, 0, i32::MAX)] {
        let bytes = source(width, height, x, y);
        let imported = crate::import("optional-frame.xcf", &bytes).unwrap();
        assert_eq!(imported.document.layers.len(), 2);
        let pixels = imported.document.layers.iter().find(|layer| layer.name == "Usable pixels").unwrap();
        assert_eq!(pixels.surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
        assert!(imported.warnings.iter().any(|warning| warning.contains("effect frame") && warning.contains("pixels preserved")));
        let ImportConversion::Report(report) = imported.document.metadata.import_conversion.as_deref().unwrap() else { panic!("typed report") };
        let frames: Vec<_> = report.entries.iter().filter(|entry| entry.feature_code == "xcf.layer_effect.frame").collect();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].disposition, ConversionDisposition::SkippedNonessential);
        assert_eq!(frames[0].reason, ConversionReason::InvalidOptionalData);
        assert!(!report.entries.iter().any(|entry| entry.feature_code == "xcf.layer_effect"), "invalid frame stages are never decoded");
        let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened.layers, imported.document.layers);
        assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
        let cancel = || true;
        assert!(matches!(crate::xcf::import("cancelled.xcf", &bytes, &Interrupt::cancel_only(&cancel)), Err(crate::IoError::Cancelled)));
    }
}

#[test]
fn representable_optional_effect_frame_keeps_its_inert_effect_inventory() {
    let imported = crate::import("valid-frame.xcf", &source(2, 3, -2, -3)).unwrap();
    let ImportConversion::Report(report) = imported.document.metadata.import_conversion.as_deref().unwrap() else { panic!("typed report") };
    assert!(!report.entries.iter().any(|entry| entry.feature_code == "xcf.layer_effect.frame"));
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.layer_effect" && entry.reason == ConversionReason::MissingMapping));
}

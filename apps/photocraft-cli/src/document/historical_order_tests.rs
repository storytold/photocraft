use crate::test_support::invoke;

// Original one-pixel uncompressed wire fixture, following the owned XCF testkit
// recipe. No decoder/export round trip supplies the sample-order oracle.
pub(crate) fn rgb16(version: u32, little: bool) -> Vec<u8> {
    fn word(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    fn pointer(bytes: &mut Vec<u8>, wide: bool) -> usize {
        let slot = bytes.len();
        bytes.resize(slot + if wide { 8 } else { 4 }, 0);
        slot
    }
    fn patch(bytes: &mut [u8], slot: usize, wide: bool) {
        let offset = bytes.len();
        if wide {
            bytes[slot..slot + 8].copy_from_slice(&(offset as u64).to_be_bytes());
        } else {
            bytes[slot..slot + 4].copy_from_slice(&(offset as u32).to_be_bytes());
        }
    }
    let mut bytes = format!("gimp xcf v{version:03}\0").into_bytes();
    let wide = version >= 11;
    for value in [1, 1, 0, 250, 17, 1] {
        word(&mut bytes, value);
    }
    bytes.push(0); // PROP_COMPRESSION: uncompressed.
    word(&mut bytes, 0);
    word(&mut bytes, 0);
    let layer = pointer(&mut bytes, wide);
    pointer(&mut bytes, wide);
    pointer(&mut bytes, wide); // No channels.
    patch(&mut bytes, layer, wide);
    for value in [1, 1, 1, 6] {
        word(&mut bytes, value);
    }
    bytes.extend_from_slice(b"Pixel\0");
    word(&mut bytes, 0);
    word(&mut bytes, 0);
    let hierarchy = pointer(&mut bytes, wide);
    pointer(&mut bytes, wide); // No mask.
    patch(&mut bytes, hierarchy, wide);
    for value in [1, 1, 8] {
        word(&mut bytes, value);
    }
    let level = pointer(&mut bytes, wide);
    pointer(&mut bytes, wide);
    patch(&mut bytes, level, wide);
    word(&mut bytes, 1);
    word(&mut bytes, 1);
    let tile = pointer(&mut bytes, wide);
    pointer(&mut bytes, wide);
    patch(&mut bytes, tile, wide);
    for value in [0x1234u16, 0x5678, 0x9abc, 0xffff] {
        bytes.extend_from_slice(&if little { value.to_le_bytes() } else { value.to_be_bytes() });
    }
    bytes
}

#[test]
fn convert_requires_historical_choice_and_preserves_exact_native_samples_and_source() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.xcf");
    let output = directory.path().join("converted.pcraft");
    let (input_name, output_name) = (input.to_str().unwrap(), output.to_str().unwrap());
    for version in [7, 11] {
        for (choice, little) in [("little-endian", true), ("big-endian", false)] {
            let original = rgb16(version, little);
            std::fs::write(&input, &original).unwrap();
            let absent = invoke(&["convert", input_name, output_name, "--no-gpu"]);
            assert_eq!(absent.code, 1, "{}", absent.stderr);
            assert!(absent.stderr.contains("sample byte order"), "{}", absent.stderr);
            assert!(!output.exists());
            let converted = invoke(&["convert", input_name, output_name, "--no-gpu", "--historical-xcf-sample-order", choice]);
            assert_eq!(converted.code, 0, "{}", converted.stderr);
            let native = std::fs::read(&output).unwrap();
            let doc = photocraft_format::load_from_bytes(&native).unwrap();
            assert_eq!(doc.size, photocraft_doc::Size::new(1, 1));
            let pixels = doc.layers[0].surface().unwrap().read_region(photocraft_doc::Rect::new(0, 0, 1, 1));
            assert_eq!(pixels, [0x1234u16, 0x5678, 0x9abc, 0xffff].map(|value| f32::from(value) / 65535.));
            let Some(photocraft_doc::import_conversion::ImportConversion::Report(report)) = doc.metadata.import_conversion.as_deref() else {
                panic!("native conversion must retain explicit sample-order evidence");
            };
            assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.historical_sample_order" && entry.native_description.contains(choice)));
            assert_eq!(std::fs::read(&input).unwrap(), original);
            std::fs::remove_file(&output).unwrap();
        }
    }
    // Modern samples are marked big-endian and need no historical choice.
    let modern = rgb16(12, false);
    std::fs::write(&input, &modern).unwrap();
    let converted = invoke(&["convert", input_name, output_name, "--no-gpu"]);
    assert_eq!(converted.code, 0, "{}", converted.stderr);
    assert_eq!(std::fs::read(&input).unwrap(), modern);
}

#[test]
fn convert_rejects_invalid_or_missing_order_values_before_reading_or_writing() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("missing.xcf");
    let output = directory.path().join("absent.pcraft");
    for value in ["native", "little", "BIG-ENDIAN", ""] {
        let rejected = invoke(&["convert", input.to_str().unwrap(), output.to_str().unwrap(), "--historical-xcf-sample-order", value]);
        assert_eq!(rejected.code, 2, "{}", rejected.stderr);
        assert!(rejected.stderr.contains("expected little-endian or big-endian"), "{}", rejected.stderr);
        assert!(!output.exists());
    }
    let rejected = invoke(&["convert", input.to_str().unwrap(), output.to_str().unwrap(), "--historical-xcf-sample-order"]);
    assert_eq!(rejected.code, 2, "{}", rejected.stderr);
    assert!(!output.exists());
}

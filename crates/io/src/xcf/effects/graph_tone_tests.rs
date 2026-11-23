//! Original synthetic inputs qualify ordinary native controls, not GEGL fidelity.
use super::linear_graph_tests::{evidence, fixture};
use photocraft_doc::{LayerContent, SmartSource, import_conversion::ConversionDisposition};

const FULL_TONE_GRAPH: &str = "exposure exposure=1 brightness-contrast brightness=0.25 contrast=1 levels in-low=0 in-high=1 out-low=0.25 out-high=0.75";

fn allow(_: &str, _: &serde_json::Value) -> std::result::Result<(), crate::native_xcf_tones::AuthorizationError> {
    Ok(())
}

#[test]
fn graph_tone_families_keep_native_control_order_pixels_and_native_reopen() {
    let _scope = crate::native_xcf_tones::enter_import(allow);
    for gray in [false, true] {
        // Native Levels is deliberately qualified only for RGB F32. Gray still
        // exercises the shared BC/Exposure graph path without widening that guard.
        let graph = if gray { "exposure exposure=1 brightness-contrast brightness=0.25 contrast=1" } else { FULL_TONE_GRAPH };
        let imported = crate::import("graph-tones.xcf", &fixture(graph, "string", gray)).unwrap();
        let LayerContent::Smart(smart) = &imported.document.layers[0].content else { panic!("qualified native graph tone chain, gray={gray}") };
        let mut expected_commands = vec!["image.adjustments.exposure", "image.adjustments.brightnessContrast", "image.adjustments.brightnessContrast"];
        if !gray {
            expected_commands.push("image.adjustments.levels");
        }
        assert_eq!(smart.smart_filters.iter().map(|filter| filter.command.as_str()).collect::<Vec<_>>(), expected_commands);
        let SmartSource::Embedded { bytes, .. } = &smart.source else { panic!("ordinary native input") };
        let original = photocraft_format::load_from_bytes(bytes).unwrap();
        let input = original.layers[0].surface().unwrap();
        for x in 0..2 {
            let source = input.rgba(x, 0);
            let got = smart.cache.as_ref().unwrap().rgba(x, 0);
            for channel in 0..3 {
                // Native F32 Exposure doubles and legacy brightness adds .25.
                // Qualified RGB float Levels then maps affinely without clipping HDR.
                let brightened = source[channel] * 2. + 0.25;
                let expected = if gray { brightened } else { 0.25 + brightened * 0.5 };
                assert!((got[channel] - expected).abs() < 0.000002, "{gray}: {source:?} -> {got:?}");
            }
            assert_eq!(got[3], source[3]);
        }
        assert!(evidence(&imported.document).entries.iter().all(|entry| entry.disposition == ConversionDisposition::NativeApproximation));
        let bytes = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        let LayerContent::Smart(restored) = &reopened.layers[0].content else { panic!("native controls after reopen") };
        assert_eq!(restored.smart_filters, smart.smart_filters);
        assert_eq!(restored.cache, smart.cache);
        assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
    }
}

#[test]
fn gray_graph_with_unqualified_levels_keeps_the_entire_source_chain_unapplied() {
    let _scope = crate::native_xcf_tones::enter_import(allow);
    let imported = crate::import("gray-levels-graph.xcf", &fixture(FULL_TONE_GRAPH, "string", true)).unwrap();
    let original = imported.document.layers[0].surface().expect("unqualified Gray Levels must not install a BC/Exposure prefix");
    assert_eq!(original.read_region(photocraft_geom::Rect::from_xywh(0, 0, 2, 1)), [0.25, 0.5, 0.75, 1.]);
    assert!(evidence(&imported.document).entries.iter().all(|entry| entry.disposition == ConversionDisposition::RetainedUnapplied));
    assert!(evidence(&imported.document).entries.iter().any(|entry| entry.reason == photocraft_doc::import_conversion::ConversionReason::NativeLimitation));
    let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
    assert_eq!(reopened.layers[0].surface(), Some(original));
    assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
}

#[test]
fn original_over_filtered_original_uses_native_tone_branch_and_preserves_hdr_alpha() {
    let _scope = crate::native_xcf_tones::enter_import(allow);
    for gray in [false, true] {
        let imported = crate::import("graph-tone-branch.xcf", &fixture("id=in over aux=[ref=in exposure exposure=1]", "string", gray)).unwrap();
        let LayerContent::Smart(smart) = &imported.document.layers[0].content else { panic!("ordinary native branch") };
        let SmartSource::Embedded { bytes, .. } = &smart.source else { panic!("compound native source") };
        let contents = photocraft_format::load_from_bytes(bytes).unwrap();
        let input = contents.layers[0].surface().unwrap();
        let LayerContent::Smart(upper) = &contents.layers[1].content else { panic!("filtered upper branch") };
        assert_eq!(upper.smart_filters[0].command, "image.adjustments.exposure");
        assert!(smart.smart_filters.is_empty(), "the completed join has no duplicate tone control");
        for x in 0..2 {
            let source = input.rgba(x, 0);
            let alpha = source[3] * (2. - source[3]);
            let got = smart.cache.as_ref().unwrap().rgba(x, 0);
            for channel in 0..3 {
                let expected = (2. * source[channel] * source[3] + source[channel] * source[3] * (1. - source[3])) / alpha;
                assert!((got[channel] - expected).abs() < 0.000002, "{gray}: {source:?} -> {got:?}");
            }
            assert!((got[3] - alpha).abs() < 0.000001);
        }
    }
}

#[test]
fn unrepresentable_tone_graphs_and_resources_never_prepare_a_partial_owner() {
    let _scope = crate::native_xcf_tones::enter_import(allow);
    for graph in [
        "exposure exposure=1 levels in-low=0.5 in-high=0.5",
        "exposure black-level=0.10000000000000002",
        "invert-linear load path=/unavailable-source",
        "levels in-low=0.1 histogram=anything",
        "brightness-contrast brightness=3 contrast=-5 invert-linear",
        "id=in exposure exposure=1 ref=in levels",
    ] {
        let before = crate::native_xcf_tones::preparation_count();
        let imported = crate::import("unapplied-tone-graph.xcf", &fixture(graph, "string", false)).unwrap();
        assert_eq!(crate::native_xcf_tones::preparation_count(), before, "{graph}");
        assert_eq!(imported.document.layers[0].surface().unwrap().rgba(0, 0), [-0.25, 0.25, 2., 0.5]);
        assert!(evidence(&imported.document).entries.iter().all(|entry| entry.disposition == ConversionDisposition::RetainedUnapplied));
    }
}

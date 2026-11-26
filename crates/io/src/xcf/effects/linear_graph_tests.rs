//! Synthetic public-import controls; the common test writer owns all wire framing.
use crate::xcf::tests::{W, hierarchy};
use photocraft_doc::{
    Document, LayerContent, SmartSource,
    import_conversion::{ConversionDisposition, ImportConversion},
};

fn allow(_: &str, _: &serde_json::Value) -> std::result::Result<(), crate::native_xcf_tones::AuthorizationError> {
    Ok(())
}
pub(super) fn fixture(graph: &str, payload: &str, gray: bool) -> Vec<u8> {
    fixture_envelope(graph, payload, gray, (1., 28, 0, 0))
}
fn fixture_envelope(graph: &str, payload: &str, gray: bool, envelope: (f32, u32, u32, usize)) -> Vec<u8> {
    let mut w = W::new(26);
    for value in [2, 1, u32::from(gray), 600] {
        w.u32(value);
    }
    w.prop(17, &[0]);
    w.end();
    let layer = w.slot();
    for _ in 0..3 {
        w.ptr(0);
    }
    w.patch(layer, w.out.len());
    for value in [2, 1, if gray { 3 } else { 1 }] {
        w.u32(value);
    }
    w.string("Graph owner");
    w.prop_u32(8, 1);
    w.prop_u32(7, 28);
    w.end();
    let pixels = w.slot();
    w.ptr(0);
    let effect = w.slot();
    w.ptr(0);
    let samples = if gray { vec![0.25f32, 0.5, 0.75, 1.] } else { vec![-0.25f32, 0.25, 2., 0.5, 0.75, 1.5, -0.5, 1.] };
    let raw: Vec<_> = samples.into_iter().flat_map(f32::to_be_bytes).collect();
    let at = hierarchy(&mut w, 2, 1, if gray { 8 } else { 16 }, &raw, craft_xcf::Compression::None);
    w.patch(pixels, at);
    w.patch(effect, w.out.len());
    for text in ["Graph stage", "", "gegl:gegl", "0:0"] {
        w.string(text);
    }
    let mut argument = W { out: Vec::new(), wide: true };
    argument.string(payload);
    argument.u32(4);
    argument.string(graph);
    w.prop(45, &argument.out);
    w.prop_u32(7, envelope.1);
    w.prop_u32(8, 1);
    w.prop_u32(35, envelope.2);
    w.prop(33, &envelope.0.to_be_bytes());
    w.end();
    w.ptr(envelope.3);
    w.out
}
pub(super) fn evidence(document: &Document) -> &photocraft_doc::import_conversion::ConversionReport {
    let Some(ImportConversion::Report(report)) = document.metadata.import_conversion.as_deref() else { panic!("conversion evidence") };
    report
}

#[test]
fn authorized_linear_graph_installs_native_invert_pixels_and_survives_native_reopen() {
    let _scope = crate::native_xcf_tones::enter_import(allow);
    for gray in [false, true] {
        for (graph, count) in [("invert-linear", 1), ("invert-linear gegl:invert-linear", 2)] {
            let bytes = fixture(graph, "string", gray);
            let imported = crate::import("linear-graph.xcf", &bytes).unwrap();
            let layer = &imported.document.layers[0];
            let LayerContent::Smart(smart) = &layer.content else { panic!("authorized native graph") };
            assert_eq!(smart.smart_filters.len(), count);
            assert!(smart.smart_filters.iter().all(|filter| filter.command == "image.adjustments.invert" && filter.params == serde_json::json!({})));
            let SmartSource::Embedded { bytes: native_source, .. } = &smart.source else { panic!("ordinary native source") };
            let original = photocraft_format::load_from_bytes(native_source).unwrap();
            let source = original.layers[0].surface().unwrap();
            let cache = smart.cache.as_ref().unwrap();
            for x in 0..2 {
                let input = source.rgba(x, 0);
                let output = cache.rgba(x, 0);
                for channel in 0..3 {
                    let expected = if count == 1 { 1. - input[channel] } else { input[channel] };
                    assert!((output[channel] - expected).abs() < 0.000001, "{gray}/{graph}: {input:?} -> {output:?}");
                }
                assert_eq!(output[3], input[3]);
            }
            let rendered = photocraft_compose::render(&imported.document, imported.document.bounds());
            for (index, pixel) in rendered.px.iter().enumerate() {
                assert_eq!(*pixel, cache.rgba(index as i32, 0));
            }
            assert!(evidence(&imported.document).entries.iter().all(|entry| entry.disposition == ConversionDisposition::NativeApproximation));
            assert_eq!(evidence(&imported.document).attachments.len(), 1);
            let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            let LayerContent::Smart(reopened_smart) = &reopened.layers[0].content else { panic!("native graph after reopen") };
            assert_eq!(reopened_smart.smart_filters, smart.smart_filters);
            assert_eq!(reopened_smart.cache, smart.cache);
            assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
        }
    }
}

#[test]
fn branches_unknown_payloads_and_unscoped_graphs_preserve_original_raster_without_preparation() {
    let no_scope = crate::import("inert-graph.xcf", &fixture("invert-linear", "string", false)).unwrap();
    assert!(matches!(no_scope.document.layers[0].content, LayerContent::Raster(_)));
    let _scope = crate::native_xcf_tones::enter_import(allow);
    for (graph, payload) in [
        ("id=in invert-linear over aux=[ref=in invert-linear]", "string"),
        ("invert-linear unknown-operation", "string"),
        ("invert-linear arbitrary=1", "string"),
        ("id=in invert-linear ref=in invert-linear", "string"),
        ("exposure exposure=0.5rel", "string"),
        ("gegl:color value='cmyk(0.1, 0.2, 0.3, 0.4)'", "string"),
        ("invert-linear", "graph"),
        ("", "string"),
    ] {
        let bytes = fixture(graph, payload, false);
        let before = crate::native_xcf_tones::preparation_count();
        let imported = crate::import("unapplied-graph.xcf", &bytes).unwrap();
        assert_eq!(crate::native_xcf_tones::preparation_count(), before, "{graph}/{payload}");
        let source = imported.document.layers[0].surface().expect("unsupported graph never replaces owner");
        assert_eq!(source.rgba(0, 0), [-0.25, 0.25, 2., 0.5]);
        assert!(evidence(&imported.document).entries.iter().all(|entry| entry.disposition == ConversionDisposition::RetainedUnapplied));
        assert_eq!(evidence(&imported.document).attachments.len(), 1);
    }
}

#[test]
fn graph_filter_authority_is_checked_before_any_native_preparation() {
    fn deny(command: &str, _: &serde_json::Value) -> std::result::Result<(), crate::native_xcf_tones::AuthorizationError> {
        assert_eq!(command, "image.adjustments.invert");
        Err(crate::native_xcf_tones::AuthorizationError::Denied)
    }
    let _scope = crate::native_xcf_tones::enter_import(deny);
    let before = crate::native_xcf_tones::preparation_count();
    assert!(matches!(crate::import("denied-graph.xcf", &fixture("invert-linear", "string", false)), Err(crate::IoError::NativeXcfToneDenied { .. })));
    assert_eq!(crate::native_xcf_tones::preparation_count(), before);
}

#[test]
fn nondefault_graph_envelopes_remain_unapplied_before_native_preparation() {
    let _scope = crate::native_xcf_tones::enter_import(allow);
    for envelope in [(0.5, 28, 0, 0), (1., 62, 0, 0), (1., 28, 1, 0), (1., 28, 0, 8)] {
        let before = crate::native_xcf_tones::preparation_count();
        let imported = crate::import("unapplied-envelope.xcf", &fixture_envelope("invert-linear", "string", false, envelope)).unwrap();
        assert_eq!(crate::native_xcf_tones::preparation_count(), before);
        assert_eq!(imported.document.layers[0].surface().unwrap().rgba(0, 0), [-0.25, 0.25, 2., 0.5]);
        assert!(evidence(&imported.document).entries.iter().all(|entry| entry.disposition == ConversionDisposition::RetainedUnapplied));
    }
}

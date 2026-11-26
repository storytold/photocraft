//! The admitted binary case uses ordinary native source-over, not source math.
use super::linear_graph_tests::{evidence, fixture};
use super::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, SmartSource, import_conversion::ConversionDisposition};
use photocraft_geom::{Rect, Size};

const GRAPH: &str = "id=in over aux=[ref=in invert-linear]";
fn allow(_: &str, _: &serde_json::Value) -> std::result::Result<(), crate::native_xcf_tones::AuthorizationError> {
    Ok(())
}

#[test]
fn binary_over_keeps_two_native_branches_and_matches_independent_alpha_after_reopen() {
    let _scope = crate::native_xcf_tones::enter_import(allow);
    for gray in [false, true] {
        let input_bytes = fixture(GRAPH, "string", gray);
        let imported = crate::import("binary-over.xcf", &input_bytes).unwrap();
        let LayerContent::Smart(smart) = &imported.document.layers[0].content else { panic!("native binary graph") };
        assert!(smart.smart_filters.is_empty(), "upper filters must not run twice on the completed join");
        let SmartSource::Embedded { bytes, .. } = &smart.source else { panic!("ordinary native source") };
        let contents = photocraft_format::load_from_bytes(bytes).unwrap();
        assert_eq!(contents.layers.len(), 2);
        let original = contents.layers[0].surface().unwrap();
        let LayerContent::Smart(upper) = &contents.layers[1].content else { panic!("editable native upper branch") };
        assert_eq!(upper.smart_filters.len(), 1);
        assert_eq!(upper.smart_filters[0].command, "image.adjustments.invert");
        let SmartSource::Embedded { bytes: upper_bytes, .. } = &upper.source else { panic!("native branch source") };
        let upper_contents = photocraft_format::load_from_bytes(upper_bytes).unwrap();
        assert_eq!(upper_contents.layers.len(), 1);
        assert!(matches!(upper_contents.layers[0].content, LayerContent::Raster(_)), "fixed native nesting, no graph model");
        let rendered = photocraft_compose::render(&contents, contents.bounds());
        for x in 0..2 {
            let source = original.rgba(x, 0);
            let alpha = source[3] + source[3] * (1. - source[3]);
            let actual = smart.cache.as_ref().unwrap().rgba(x, 0);
            assert!((actual[3] - alpha).abs() < 0.000001);
            for channel in 0..3 {
                let expected = ((1. - source[channel]) * source[3] + source[channel] * source[3] * (1. - source[3])) / alpha;
                assert!((actual[channel] - expected).abs() < 0.000002, "{gray}: {actual:?} versus {expected}");
                assert!((actual[channel] - rendered.px[x as usize][channel]).abs() < 0.000001);
            }
        }
        assert!(evidence(&imported.document).entries.iter().all(|entry| entry.disposition == ConversionDisposition::NativeApproximation));
        assert_eq!(evidence(&imported.document).attachments.len(), 1);
        let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        let LayerContent::Smart(restored) = &reopened.layers[0].content else { panic!("native owner after reopen") };
        assert_eq!(restored.cache, smart.cache);
        assert_eq!(restored.source, smart.source);
        assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
        assert_eq!(input_bytes, fixture(GRAPH, "string", gray));
    }
}

#[test]
fn other_binary_topologies_and_transfer_overrides_remain_unapplied() {
    let _scope = crate::native_xcf_tones::enter_import(allow);
    for graph in [
        "id=in invert-linear over aux=[ref=in invert-linear]",
        "id=in over aux=[ref=in invert-linear] invert-linear",
        "id=in over aux=[ref=in invert-linear] srgb=true",
        "id=in over aux=[ref=in over aux=[ref=in invert-linear]]",
        "id=in over aux=[ref=in unknown-operation]",
    ] {
        let before = crate::native_xcf_tones::preparation_count();
        let imported = crate::import("unsupported-binary.xcf", &fixture(graph, "string", false)).unwrap();
        assert_eq!(crate::native_xcf_tones::preparation_count(), before);
        assert_eq!(imported.document.layers[0].surface().unwrap().rgba(0, 0), [-0.25, 0.25, 2., 0.5]);
        assert!(evidence(&imported.document).entries.iter().all(|entry| entry.disposition == ConversionDisposition::RetainedUnapplied));
    }
}

#[cfg(feature = "corpus")]
#[test]
fn archived_binary_graph_refuses_unverified_envelope_even_with_private_unit_authority() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/gimp/graph-original-over-inverted.xcf");
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}: run `cargo xtask corpus --gimp`", path.display()));
    let _scope = crate::native_xcf_tones::enter_import(allow);
    let before = crate::native_xcf_tones::preparation_count();
    let imported = crate::import("graph-original-over-inverted.xcf", &bytes).unwrap();
    assert_eq!(crate::native_xcf_tones::preparation_count(), before);
    assert_eq!(imported.document.layers[0].surface().unwrap().rgba(0, 0), [17. / 255., 13. / 255., 29. / 255., 64. / 255.]);
    let records: Vec<_> = evidence(&imported.document).entries.iter().filter(|entry| entry.feature_code == "xcf.layer_effect").collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].disposition, ConversionDisposition::RetainedUnapplied);
    assert_eq!(records[0].reason, photocraft_doc::import_conversion::ConversionReason::NativeLimitation);
}

fn native_fixture() -> (Document, Layer, Rect, Mapping) {
    let document = Document::new("Budgeted binary graph", Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
    let bounds = Rect::new(-3, 4, -1, 5);
    let mut owner = Layer::raster("Original", document.pixel_format());
    owner.surface_mut().unwrap().fill_rect(bounds, &[0.25, 0.5, 2., 0.5]);
    let mut source = super::tests::source("gegl:gegl", vec![craft_xcf::effects::Argument { name: "string", value: ArgumentValue::String(GRAPH) }]);
    source.record.mode = 28;
    let mapping = map_with_budget(&source, &mut craft_xcf::effects::Budget::default(), &Interrupt::NONE).unwrap().unwrap();
    assert!(mapping.over_original);
    (document, owner, bounds, mapping)
}

#[test]
fn binary_over_shared_work_boundary_cancellation_and_late_failure_are_atomic() {
    let (document, owner, bounds, mapping) = native_fixture();
    let initial_retained = 64 << 20;
    let initial_work = 256 << 20;
    let mut retained = initial_retained;
    let mut work = initial_work;
    let prepared = installer::prepare(
        mapping.owner,
        &owner,
        bounds,
        &document,
        std::slice::from_ref(&mapping),
        &mut installer::Budgets { retained: &mut retained, work: &mut work },
        &Interrupt::NONE,
    )
    .unwrap();
    assert_eq!(prepared.smart.transform, photocraft_doc::Affine::translate(-3., 4.));
    let used_work = initial_work - work;
    assert!(used_work > 0 && retained < initial_retained);
    let mut exact_retained = initial_retained;
    let mut exact_work = used_work;
    installer::prepare(
        mapping.owner,
        &owner,
        bounds,
        &document,
        std::slice::from_ref(&mapping),
        &mut installer::Budgets { retained: &mut exact_retained, work: &mut exact_work },
        &Interrupt::NONE,
    )
    .unwrap();
    assert_eq!(exact_work, 0);
    let mut refused_retained = initial_retained;
    let mut refused_work = used_work - 1;
    assert!(matches!(
        installer::prepare(
            mapping.owner,
            &owner,
            bounds,
            &document,
            std::slice::from_ref(&mapping),
            &mut installer::Budgets { retained: &mut refused_retained, work: &mut refused_work },
            &Interrupt::NONE
        ),
        Err(installer::Refusal::Limit)
    ));
    assert_eq!(refused_retained, initial_retained);
    assert_eq!(refused_work, 0);
    let cancel = || true;
    let mut cancelled_work = initial_work;
    assert!(matches!(
        installer::prepare(
            mapping.owner,
            &owner,
            bounds,
            &document,
            std::slice::from_ref(&mapping),
            &mut installer::Budgets { retained: &mut refused_retained, work: &mut cancelled_work },
            &Interrupt::cancel_only(&cancel)
        ),
        Err(installer::Refusal::Cancelled)
    ));
    assert_eq!(cancelled_work, initial_work);
    let mut broken_owner = owner.clone();
    broken_owner.surface_mut().unwrap().write_pixel(bounds.x0, bounds.y0, &[f32::NAN, 0.5, 2., 0.5]);
    let mut failed_work = initial_work;
    assert!(matches!(
        installer::prepare(
            mapping.owner,
            &broken_owner,
            bounds,
            &document,
            std::slice::from_ref(&mapping),
            &mut installer::Budgets { retained: &mut refused_retained, work: &mut failed_work },
            &Interrupt::NONE
        ),
        Err(installer::Refusal::Preparation(_))
    ));
    assert_eq!(refused_retained, initial_retained);
    assert!(failed_work < initial_work, "late refusal never refunds native generation work");
    assert_eq!(owner.surface().unwrap().rgba(bounds.x0, bounds.y0), [0.25, 0.5, 2., 0.5]);
    let (_, _, _, second_mapping) = native_fixture();
    let owner_id = mapping.owner;
    let mut unsupported_work = initial_work;
    assert!(matches!(
        installer::prepare(
            owner_id,
            &owner,
            bounds,
            &document,
            &[mapping, second_mapping],
            &mut installer::Budgets { retained: &mut refused_retained, work: &mut unsupported_work },
            &Interrupt::NONE
        ),
        Err(installer::Refusal::Unsupported(_))
    ));
    assert_eq!(unsupported_work, initial_work, "mixed stacks never prepare a prefix");
    assert_eq!(refused_retained, initial_retained);
}

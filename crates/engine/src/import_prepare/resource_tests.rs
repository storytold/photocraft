use super::*;
use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Layer, SmartFilter, SmartObject};
use photocraft_geom::{Affine, Size};
use std::cell::{Cell, RefCell};

/// Create a fresh native preparation ledger with the production limits and an empty per-call memo.
fn allowance() -> Allowance {
    Allowance { remaining_bytes: MAX_PREPARATION_BYTES, source_bytes: MAX_SOURCE_BYTES, items: MAX_ITEMS, memo: Default::default() }
}

/// Encode a tiny independently colored native raster source for actual preparation-path tests.
fn source(color: Color) -> Arc<Vec<u8>> {
    let doc = Document::with_background("Original tiny native source", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, color);
    Arc::new(smart_cmds::encode_source(&doc).unwrap())
}

/// Wrap immutable native bytes in a new editable Smart identity with no prepared cache.
fn smart(bytes: Arc<Vec<u8>>) -> Layer {
    Layer::new(
        "Native smart",
        LayerContent::Smart(SmartObject::new(SmartSource::Embedded { file_name: "source.pcraft".into(), bytes }, Affine::IDENTITY, None)),
    )
}

/// Place supplied Smart layers into a small native parent without precomputing their appearance.
fn document(layers: Vec<Layer>) -> Document {
    let mut doc = Document::new("Original pending data", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    doc.layers = layers;
    doc
}

#[test]
/// Exercise production source, item and working-byte debits at their exact limit and one unit beyond.
fn actual_source_and_item_charge_limits_accept_exact_boundary_then_refuse_one_more() {
    let mut budget = allowance();
    assert_eq!(budget.source_bytes, 1 << 30);
    budget.source(MAX_SOURCE_BYTES).unwrap();
    assert_eq!(budget.source_bytes, 0);
    assert!(budget.source(1).is_err());
    assert_eq!(budget.source_bytes, 0);
    assert_eq!(budget.items, 65_536);
    for _ in 0..MAX_ITEMS {
        budget.item().unwrap();
    }
    assert_eq!(budget.items, 0);
    assert!(budget.item().is_err());
    assert_eq!(budget.items, 0);
    budget.remaining_bytes = 17;
    budget.bytes(17).unwrap();
    assert!(budget.bytes(1).is_err());
    assert_eq!(budget.remaining_bytes, 0);
}

#[test]
/// Count only missing preparations and reject a short item allowance before changing the pending document.
fn actual_preflight_counts_only_missing_items_and_refuses_before_mutating() {
    let bytes = source(Color::BLACK);
    let original = document(vec![smart(bytes.clone()), smart(bytes)]);
    let mut exact = allowance();
    exact.items = 2;
    assert_eq!(missing_caches(&original, &JobCtx::new(), &mut exact).unwrap().len(), 2);
    assert_eq!(exact.items, 0);
    let mut refused = original.clone();
    let mut short = allowance();
    short.items = 1;
    assert!(prepare_with_allowance(&mut refused, &JobCtx::new(), &mut short).unwrap_err().to_string().contains("item count"));
    assert_eq!(refused, original);
}

#[test]
/// Check sibling source charges share one allowance and a one-byte shortage preserves the original document.
fn source_debit_is_shared_across_siblings_and_failure_is_transactional() {
    let bytes = source(Color::rgb(1.0, 0.0, 0.0));
    let original = document(vec![smart(bytes.clone()), smart(bytes.clone())]);
    let mut exact = original.clone();
    let mut budget = allowance();
    budget.source_bytes = bytes.len() * 2;
    prepare_with_allowance(&mut exact, &JobCtx::new(), &mut budget).unwrap();
    assert_eq!(budget.source_bytes, 0);
    assert!(exact.layers.iter().all(|layer| layer.surface().is_some()));
    let mut refused = original.clone();
    let mut short = allowance();
    short.source_bytes = bytes.len() * 2 - 1;
    assert!(prepare_with_allowance(&mut refused, &JobCtx::new(), &mut short).unwrap_err().to_string().contains("embedded source bytes"));
    assert_eq!(refused, original);
}

#[test]
/// Exercise the real recursive depth boundary, including the next embedded native load, without building huge
/// documents.
fn actual_native_content_depth_guard_accepts_64_and_rejects_65() {
    let mut empty = document(Vec::new());
    assert!(!prepare(&mut empty, &JobCtx::new(), MAX_CONTENT_DEPTH, &mut allowance()).unwrap());
    assert!(prepare(&mut empty, &JobCtx::new(), MAX_CONTENT_DEPTH + 1, &mut allowance()).unwrap_err().to_string().contains("64 nested"));
    // Also exercise the actual recursive +1 call, rather than only its empty guard.
    let original = document(vec![smart(source(Color::BLACK))]);
    let mut accepted = original.clone();
    prepare(&mut accepted, &JobCtx::new(), MAX_CONTENT_DEPTH - 1, &mut allowance()).unwrap();
    assert!(accepted.layers[0].surface().is_some());
    let mut refused = original.clone();
    assert!(prepare(&mut refused, &JobCtx::new(), MAX_CONTENT_DEPTH, &mut allowance()).unwrap_err().to_string().contains("64 nested"));
    assert_eq!(refused, original);
}

thread_local! {
    static CANCEL_CONTEXT: RefCell<Option<JobCtx>> = const { RefCell::new(None) };
    static FIRST_FILTER_CHECKS: Cell<usize> = const { Cell::new(0) };
}

/// Observe the first native command and cancel during the second to test late rollback.
fn cancel_second_filter(command: &str, _: &serde_json::Value) -> Result<()> {
    if command == "image.adjustments.invert" {
        FIRST_FILTER_CHECKS.with(|count| count.set(count.get() + 1));
    }
    if command == "image.adjustments.posterize" {
        CANCEL_CONTEXT.with(|context| {
            if let Some(context) = context.borrow().as_ref() {
                context.cancel();
            }
        });
    }
    Ok(())
}

#[test]
/// Require late cancellation after real first-layer work to leave every pending layer unchanged.
fn cancellation_during_second_native_layer_keeps_the_original_document() {
    let bytes = source(Color::rgb(0.2, 0.4, 0.6));
    let mut original = document(vec![smart(bytes.clone()), smart(bytes)]);
    for (layer, (command, params)) in
        original.layers.iter_mut().zip([("image.adjustments.invert", serde_json::json!({})), ("image.adjustments.posterize", serde_json::json!({"levels":4}))])
    {
        let LayerContent::Smart(object) = &mut layer.content else { panic!("test smart") };
        object.smart_filters.push(SmartFilter { command: command.into(), params, blend: BlendMode::Normal, opacity: 1., visible: true });
    }
    let mut pending = original.clone();
    let context = JobCtx::new();
    CANCEL_CONTEXT.with(|slot| *slot.borrow_mut() = Some(context.clone()));
    FIRST_FILTER_CHECKS.with(|count| count.set(0));
    let _scope = crate::jobs::render_authorization_scope(Some(cancel_second_filter));
    let result = prepare_with_allowance(&mut pending, &context, &mut allowance());
    CANCEL_CONTEXT.with(|slot| *slot.borrow_mut() = None);
    assert!(FIRST_FILTER_CHECKS.with(|count| count.get()) > 0);
    assert!(matches!(result, Err(EngineError::Cancelled)));
    assert_eq!(pending, original);
}

#[test]
/// Verify exact source reuse shares COW payloads but still charges each hit's map metadata and editable identity.
fn repeated_identical_filter_free_sources_share_only_completed_preparation_work() {
    let bytes = source(Color::rgb(0.8, 0.2, 0.4));
    let mut single = document(vec![smart(bytes.clone())]);
    let mut measured = allowance();
    prepare_with_allowance(&mut single, &JobCtx::new(), &mut measured).unwrap();
    let one_unique_work = MAX_PREPARATION_BYTES - measured.remaining_bytes;
    let separate_allocation = Arc::new(bytes.as_ref().clone());
    let mut repeated = document(vec![smart(bytes.clone()), smart(separate_allocation)]);
    if let LayerContent::Smart(object) = &mut repeated.layers[1].content
        && let SmartSource::Embedded { file_name, .. } = &mut object.source
    {
        *file_name = "native-branch.pcraft".into();
    }
    let original_repeated = repeated.clone();
    let hit_metadata = memo::clone_bytes(single.layers[0].surface().unwrap()).unwrap();
    let mut budget = allowance();
    let render_peak = Allowance::render_bytes(&repeated).unwrap();
    budget.remaining_bytes = render_peak + one_unique_work + hit_metadata;
    budget.source_bytes = bytes.len() * 2;
    budget.items = 2;
    prepare_with_allowance(&mut repeated, &JobCtx::new(), &mut budget).unwrap();
    assert_eq!(budget.remaining_bytes, render_peak, "temporary rendering is released after retained memo/cache charges");
    assert_eq!(budget.source_bytes, 0);
    assert_eq!(budget.items, 0);
    assert_eq!(repeated.layers[0].surface(), repeated.layers[1].surface());
    let (LayerContent::Smart(first), LayerContent::Smart(second)) = (&repeated.layers[0].content, &repeated.layers[1].content) else { panic!("native smart") };
    assert_ne!(first.contents_id, second.contents_id, "memo reuse does not merge editable identities");
    let (SmartSource::Embedded { bytes: a, .. }, SmartSource::Embedded { bytes: b, .. }) = (&first.source, &second.source) else { panic!("embedded source") };
    assert!(Arc::ptr_eq(a, b), "successful source reuse shares the original immutable bytes");
    let first_tile = first.cache.as_ref().unwrap().tiles().next().unwrap().1;
    let second_tile = second.cache.as_ref().unwrap().tiles().next().unwrap().1;
    assert!(Arc::ptr_eq(first_tile, second_tile), "successful cache reuse shares native COW tiles");
    let mut insufficient_metadata = original_repeated.clone();
    let mut short_metadata = allowance();
    // A memo hit does not reserve a frame: released render headroom would be
    // available to fund it. This tiny fixture's hit metadata exceeds its32-byte
    // render peak, so the first unique render fits but the second hit does not.
    assert!(hit_metadata > render_peak);
    short_metadata.remaining_bytes = one_unique_work + hit_metadata - 1;
    assert!(prepare_with_allowance(&mut insufficient_metadata, &JobCtx::new(), &mut short_metadata).unwrap_err().to_string().contains("working-byte"));
    assert_eq!(short_metadata.remaining_bytes, hit_metadata - 1, "failed hit preflight does not debit or allocate its clone");
    assert_eq!(short_metadata.source_bytes, MAX_SOURCE_BYTES - bytes.len() * 2, "failure occurs after entering the second Smart item");
    assert_eq!(insufficient_metadata, original_repeated);
    // A distinct original source still needs its own work allowance.
    let original = document(vec![smart(bytes), smart(source(Color::rgb(0.1, 0.7, 0.3)))]);
    let mut different = original.clone();
    let mut short = allowance();
    short.remaining_bytes = one_unique_work;
    assert!(prepare_with_allowance(&mut different, &JobCtx::new(), &mut short).is_err());
    assert_eq!(different, original);
}

#[test]
/// Require the embedded-depth guard even when identical source work is already memoized.
fn repeated_source_memo_does_not_bypass_native_depth_admission() {
    let bytes = source(Color::BLACK);
    let mut budget = allowance();
    let mut first = document(vec![smart(bytes.clone())]);
    prepare_with_allowance(&mut first, &JobCtx::new(), &mut budget).unwrap();
    let original = document(vec![smart(bytes)]);
    let mut at_limit = original.clone();
    assert!(prepare(&mut at_limit, &JobCtx::new(), MAX_CONTENT_DEPTH, &mut budget).unwrap_err().to_string().contains("64 nested"));
    assert_eq!(at_limit, original);
}

#[test]
/// Check memo hits preserve depth/HDR pixels while a later edit detaches tiles without changing the other alias.
fn repeated_native_cache_hits_keep_each_depth_hdr_and_cow_edit_independence() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let color = if depth == SampleType::F32 { Color::rgba(-0.25, 0.5, 1.75, 0.75) } else { Color::rgba(0.2, 0.4, 0.6, 0.75) };
        let source = Document::with_background("Original depth source", Size::new(1, 1), ColorMode::Rgb, depth, color);
        let bytes = Arc::new(smart_cmds::encode_source(&source).unwrap());
        let make = |layers| {
            let mut doc = Document::new("Same native context", source.size, source.mode, source.depth);
            doc.layers = layers;
            doc
        };
        let mut unique = make(vec![smart(bytes.clone())]);
        let mut unique_budget = allowance();
        prepare_with_allowance(&mut unique, &JobCtx::new(), &mut unique_budget).unwrap();
        let expected = unique.layers[0].surface().unwrap();
        let pixel = expected.pixel(0, 0);
        if depth == SampleType::F32 {
            assert!(pixel[0] < 0. && pixel[2] > 1., "ordinary native source retains finite HDR");
        }
        let mut repeated = make(vec![smart(bytes.clone()), smart(Arc::new(bytes.as_ref().clone()))]);
        let mut hit_budget = allowance();
        let unique_work = MAX_PREPARATION_BYTES - unique_budget.remaining_bytes;
        let metadata = memo::clone_bytes(expected).unwrap();
        let render_peak = Allowance::render_bytes(&repeated).unwrap();
        hit_budget.remaining_bytes = render_peak + unique_work + metadata;
        prepare_with_allowance(&mut repeated, &JobCtx::new(), &mut hit_budget).unwrap();
        assert_eq!(hit_budget.remaining_bytes, render_peak, "{depth:?}: cache hits charge metadata while temporary rendering is released");
        assert_eq!(repeated.depth, depth);
        for layer in &repeated.layers {
            let surface = layer.surface().unwrap();
            assert_eq!(surface.format(), repeated.pixel_format());
            assert_eq!(surface, expected, "{depth:?}: repeated pixels exactly match independent unique preparation");
        }
        let a = repeated.layers[0].surface().unwrap().tiles().next().unwrap().1;
        let b = repeated.layers[1].surface().unwrap().tiles().next().unwrap().1;
        assert!(Arc::ptr_eq(a, b), "{depth:?}: hit shares native COW tile payload");
        let LayerContent::Smart(edited) = &mut repeated.layers[1].content else { panic!("native smart cache") };
        edited.cache.as_mut().unwrap().write_pixel(0, 0, &[0.9, 0.1, 0.2, 1.]);
        assert_eq!(repeated.layers[0].surface().unwrap().pixel(0, 0), pixel, "{depth:?}: editing a memo alias cannot modify its sibling");
        assert_ne!(repeated.layers[1].surface().unwrap().pixel(0, 0), pixel);
        let a = repeated.layers[0].surface().unwrap().tiles().next().unwrap().1;
        let b = repeated.layers[1].surface().unwrap().tiles().next().unwrap().1;
        assert!(!Arc::ptr_eq(a, b), "{depth:?}: edited tile detaches");
    }
}

/// Create a sparse native bundle and obtain its real decoded-byte accounting for reservation-boundary tests.
fn sparse_source(depth: SampleType) -> (Document, Arc<Vec<u8>>, u64) {
    let mut source = Document::new("Original sparse source", Size::new(256, 256), ColorMode::Rgb, depth);
    let mut layer = Layer::raster("Sparse pixels", source.pixel_format());
    layer.surface_mut().unwrap().write_pixel(3, 5, &[0.25, 0.5, 0.75, 1.]);
    source.layers.push(layer);
    let bytes = Arc::new(smart_cmds::encode_source(&source).unwrap());
    let (_, decoded) = photocraft_format::load_from_bytes_measured(&bytes, &Default::default()).unwrap();
    (source, bytes, decoded)
}

#[test]
/// Prove same-extent loading consumes its existing render reservation once and preserves ordinary native pixels.
fn native_canvas_admission_reuses_its_already_debited_same_extent_render_reservation() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let (source, bytes, decoded) = sparse_source(depth);
        let mut parent = Document::new("Same extent parent", source.size, source.mode, source.depth);
        parent.layers.push(smart(bytes));
        let mut ordinary = parent.clone();
        prepare_imported_document(&mut ordinary, &JobCtx::new()).unwrap();
        let mut charged = allowance();
        charged.render(&parent).unwrap();
        let render = MAX_PREPARATION_BYTES - charged.remaining_bytes;
        assert!(decoded < render, "small sparse source makes duplicate frame reservation observable");
        let mut exact = allowance();
        let mut retained = allowance();
        retained.cache(ordinary.layers[0].surface().unwrap()).unwrap();
        let cache = MAX_PREPARATION_BYTES - retained.remaining_bytes;
        exact.remaining_bytes = render + decoded + cache;
        prepare_with_allowance(&mut parent, &JobCtx::new(), &mut exact).unwrap();
        assert_eq!(exact.remaining_bytes, render, "same extent releases temporary rendering but retains decoded/cache charges");
        assert_eq!(parent, ordinary, "{depth:?}: admission credit does not change ordinary native pixels or editable data");
    }
}

#[test]
/// Require the extra source extent's real debit and reject one-byte-short work before executing native filters.
fn larger_native_source_requires_checked_extra_render_debit_before_composition() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let (source, bytes, decoded) = sparse_source(depth);
        let mut parent = Document::new("Small parent", Size::new(16, 16), source.mode, source.depth);
        parent.layers.push(smart(bytes));
        let LayerContent::Smart(object) = &mut parent.layers[0].content else { panic!("native smart") };
        object.smart_filters.push(SmartFilter {
            command: "image.adjustments.invert".into(),
            params: serde_json::json!({}),
            blend: BlendMode::Normal,
            opacity: 1.,
            visible: true,
        });
        let mut source_charge = allowance();
        source_charge.render(&source).unwrap();
        let source_render = MAX_PREPARATION_BYTES - source_charge.remaining_bytes;
        let mut ordinary = parent.clone();
        prepare_imported_document(&mut ordinary, &JobCtx::new()).unwrap();
        let mut exact = parent.clone();
        let mut budget = allowance();
        let mut retained = allowance();
        retained.cache(ordinary.layers[0].surface().unwrap()).unwrap();
        let cache = MAX_PREPARATION_BYTES - retained.remaining_bytes;
        budget.remaining_bytes = source_render + decoded + cache;
        prepare_with_allowance(&mut exact, &JobCtx::new(), &mut budget).unwrap();
        assert_eq!(budget.remaining_bytes, source_render, "{depth:?}: the full larger temporary frame is released after preparation");
        assert_eq!(exact, ordinary);
        let mut short = allowance();
        short.remaining_bytes = source_render + decoded - 1;
        let mut refused = parent.clone();
        FIRST_FILTER_CHECKS.with(|count| count.set(0));
        let _scope = crate::jobs::render_authorization_scope(Some(cancel_second_filter));
        assert!(
            prepare_with_allowance(&mut refused, &JobCtx::new(), &mut short).unwrap_err().to_string().contains("working-byte"),
            "source cannot spend a reserved frame twice"
        );
        assert_eq!(FIRST_FILTER_CHECKS.with(|count| count.get()), 0, "extra source extent is debited before even authorizing its render filter");
        assert_eq!(refused, parent, "{depth:?}: failed top-up leaves pending native data unchanged");
    }
}

/// Prepare a sparse native item repeatedly on a larger declared canvas without allocating full-frame fixtures.
fn sparse_items(kind: usize, count: usize) -> Document {
    let mut doc = Document::new("Sequential sparse native work", Size::new(512, 512), ColorMode::Rgb, SampleType::U8);
    for i in 0..count {
        let layer = match kind {
            0 => Layer::new("Sparse native text", LayerContent::Text(photocraft_doc::TextLayer { text: ".".into(), size_pt: 8., ..Default::default() })),
            1 => Layer::new(
                "Sparse native shape",
                LayerContent::Shape(photocraft_doc::ShapeLayer {
                    path: photocraft_doc::Path::new(vec![photocraft_doc::Subpath::polygon(&[(2., 2.), (4., 2.), (4., 4.), (2., 4.)])]),
                    fill: Some(photocraft_doc::Fill::Solid(Color::BLACK)),
                    ..Default::default()
                }),
            ),
            _ => smart(source(Color::rgb(i as f32 / count as f32, 0.3, 0.7))),
        };
        doc.layers.push(layer);
    }
    doc
}

#[test]
/// Many sparse native items require retained cache totals plus one sequential render peak, not one frame per item.
fn sequential_text_shape_and_smart_release_only_temporary_frames() {
    for kind in 0..3 {
        let original = sparse_items(kind, 8);
        let frame = Allowance::render_bytes(&original).unwrap();
        let mut ordinary = original.clone();
        let mut measured = allowance();
        prepare_with_allowance(&mut ordinary, &JobCtx::new(), &mut measured).unwrap();
        let retained = MAX_PREPARATION_BYTES - measured.remaining_bytes;
        assert!(retained > 0, "actual native caches remain charged");
        assert!(retained < frame * 7, "sparse retained data is smaller than seven full render frames");
        let initial = frame + retained;
        let mut exact = allowance();
        exact.remaining_bytes = initial;
        let mut pending = original.clone();
        prepare_with_allowance(&mut pending, &JobCtx::new(), &mut exact).unwrap();
        assert_eq!(exact.remaining_bytes, frame, "{kind}: release frame only, retain cache/source/memo storage");
        assert_eq!(pending, ordinary);
        assert!(pending.layers.iter().all(|layer| layer.surface().is_some()));
        let mut cache_charge = allowance();
        for layer in &pending.layers {
            cache_charge.cache(layer.surface().unwrap()).unwrap();
        }
        assert!(retained >= MAX_PREPARATION_BYTES - cache_charge.remaining_bytes, "each retained payload and map remains charged");
    }
}

#[test]
/// Parent and child Smart preparation coexist: a budget sufficient for one peak cannot admit their nested peak.
fn nested_smart_frames_remain_reserved_until_the_outer_item_finishes() {
    let inner = sparse_items(2, 1);
    let bytes = Arc::new(smart_cmds::encode_source(&inner).unwrap());
    let mut original = Document::new("Nested native peak", inner.size, inner.mode, inner.depth);
    original.layers.push(smart(bytes));
    let frame = Allowance::render_bytes(&original).unwrap();
    let mut ordinary = original.clone();
    let mut measured = allowance();
    prepare_with_allowance(&mut ordinary, &JobCtx::new(), &mut measured).unwrap();
    let retained = MAX_PREPARATION_BYTES - measured.remaining_bytes;
    assert!(retained < frame, "small sparse nested caches isolate concurrent render reservations");
    let mut budget = allowance();
    budget.remaining_bytes = frame + retained;
    let mut refused = original.clone();
    assert!(prepare_with_allowance(&mut refused, &JobCtx::new(), &mut budget).is_err());
    assert_eq!(refused, original, "nested peak failure cannot publish a partially prepared source or cache");
    assert!(budget.remaining_bytes >= frame, "all successfully reserved temporary frames are released on failure");
}

#[test]
/// Failure or cancellation releases exact temporary reservations, including top-ups, while retained debits survive.
fn temporary_scope_does_not_refund_retained_bytes_on_error_or_cancel() {
    let doc = document(Vec::new());
    for cancel in [false, true] {
        let mut budget = allowance();
        budget.remaining_bytes = 100;
        let context = JobCtx::new();
        let result: Result<()> = budget.with_render(&doc, |budget, frame, temporary| {
            assert_eq!(frame, 32);
            budget.extend_render(temporary, 16)?;
            budget.bytes(7)?;
            if cancel {
                context.cancel();
                context.check()
            } else {
                Err(limit("synthetic failure"))
            }
        });
        assert!(result.is_err());
        assert_eq!(budget.remaining_bytes, 93, "only the 48 temporary bytes are refunded");
    }
    let mut short = allowance();
    short.remaining_bytes = 31;
    assert!(short.with_render(&doc, |_, _, _| Ok(())).is_err());
    assert_eq!(short.remaining_bytes, 31, "failed initial reservation is atomic");
}

/// Produce an original flat source so nested preparation exercises a non-native decoder without any external asset.
fn png_smart() -> Layer {
    let pixels = Document::with_background("Original flat pixels", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::rgb(0.2, 0.4, 0.6));
    let bytes = photocraft_io::export(&pixels, "original.png", &Default::default()).unwrap().bytes;
    Layer::new(
        "Flat Smart source",
        LayerContent::Smart(SmartObject::new(SmartSource::Embedded { file_name: "original.png".into(), bytes: Arc::new(bytes) }, Affine::IDENTITY, None)),
    )
}

#[test]
/// Decoded flat tiles are charged before rendering, and an insufficient shared allowance cannot publish a cache.
fn non_native_source_tiles_share_the_preparation_work_allowance() {
    let original = document(vec![png_smart()]);
    let mut refused = original.clone();
    let mut budget = allowance();
    budget.remaining_bytes = 64;
    assert!(prepare_with_allowance(&mut refused, &JobCtx::new(), &mut budget).is_err());
    assert_eq!(refused, original);
    let mut ready = original.clone();
    prepare_with_allowance(&mut ready, &JobCtx::new(), &mut allowance()).unwrap();
    assert!(ready.layers[0].surface().is_some());
    let (LayerContent::Smart(a), LayerContent::Smart(b)) = (&original.layers[0].content, &ready.layers[0].content) else { panic!("Smart sources") };
    assert_eq!(a.source, b.source, "preparation must preserve original non-native contents");
}

#[test]
/// A native parent and flat child consume the same item counter; recursive decoding cannot reset admission.
fn nested_non_native_source_keeps_one_item_ledger_and_cancellation_context() {
    let inner = document(vec![png_smart()]);
    let original = document(vec![smart(Arc::new(smart_cmds::encode_source(&inner).unwrap()))]);
    let mut pending = original.clone();
    let mut budget = allowance();
    budget.items = 1;
    assert!(prepare_with_allowance(&mut pending, &JobCtx::new(), &mut budget).is_err());
    assert_eq!(budget.items, 0);
    assert_eq!(pending, original);
    let context = JobCtx::new();
    context.cancel();
    assert!(matches!(prepare_with_allowance(&mut pending, &context, &mut allowance()), Err(EngineError::Cancelled)));
    assert_eq!(pending, original);
    let mut exact = allowance();
    exact.items = 2;
    prepare_with_allowance(&mut pending, &JobCtx::new(), &mut exact).unwrap();
    assert_eq!(exact.items, 0);
    assert!(pending.layers[0].surface().is_some());
}

#[test]
/// Warm Smart-free native and PNG sources bypass the decoder checkpoint, rather than merely reporting a stable counter.
fn warm_smart_free_sources_reuse_pixels_without_reentering_the_decoder() {
    let layer = png_smart();
    let LayerContent::Smart(object) = layer.content else { panic!("PNG Smart") };
    let SmartSource::Embedded { file_name, bytes } = object.source else { panic!("embedded PNG") };
    let format = document(Vec::new()).pixel_format();
    for (name, bytes) in [(file_name, bytes), ("raster.pcraft".into(), source(Color::rgb(0.71, 0.29, 0.43)))] {
        let first = smart_cmds::source_image(&name, &bytes, format).unwrap();
        crate::allocation::fail_next_for_test();
        let reused = smart_cmds::source_image(&name, &bytes, format);
        let untouched_failpoint = crate::allocation::checkpoint("proving decoder was skipped");
        assert!(reused.is_ok(), "Smart-free warm source must not decode again: {name}");
        assert!(untouched_failpoint.is_err(), "the warm hit must leave the decoder failpoint armed");
        assert_eq!(first.surface.as_ref(), reused.unwrap().surface.as_ref());
    }
}

#[test]
/// A warm Smart-bearing source rechecks its recorded authority without entering the decoder again.
fn warm_smart_source_rechecks_authority_without_redecoding() {
    let mut layer = smart(source(Color::rgb(0.37, 0.19, 0.61)));
    let LayerContent::Smart(object) = &mut layer.content else { panic!("native Smart") };
    object.smart_filters.push(SmartFilter {
        command: "image.adjustments.invert".into(),
        params: serde_json::json!({}),
        blend: BlendMode::Normal,
        opacity: 1.0,
        visible: true,
    });
    let doc = document(vec![layer]);
    let bytes = smart_cmds::encode_source(&doc).unwrap();
    smart_cmds::source_image("action.pcraft", &bytes, doc.pixel_format()).unwrap();
    assert_eq!(smart_cmds::source_decode_count(&bytes, doc.pixel_format()), 1);
    smart_cmds::source_image("action.pcraft", &bytes, doc.pixel_format()).unwrap();
    assert_eq!(smart_cmds::source_decode_count(&bytes, doc.pixel_format()), 1);
    crate::allocation::fail_next_for_test();
    let reused = smart_cmds::source_image("action.pcraft", &bytes, doc.pixel_format());
    let reset = crate::allocation::checkpoint("clear possible test failpoint");
    assert!(reused.is_ok(), "a proved warm source reuses its authorized pixels");
    assert!(reset.is_err(), "the warm hit must leave the decoder failpoint armed");
}

/// Keep one simple vector shape in the actual SVG renderer rather than supplying a native cached substitute.
fn svg_smart(stack: bool) -> Layer {
    let mut object = SmartObject::new(
        SmartSource::Embedded {
            file_name: "original.svg".into(),
            bytes: Arc::new(br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="#cc3366"/></svg>"##.to_vec()),
        },
        Affine::IDENTITY,
        None,
    );
    if stack {
        object.stack_mode = Some(photocraft_doc::StackMode::Mean);
    }
    Layer::new("Original SVG", LayerContent::Smart(object))
}

#[test]
/// Direct SVG spends only its outer preparation item and retained cache; stacked SVG retains shared decoded-source admission.
fn direct_svg_avoids_discarded_native_import_while_stacked_svg_keeps_shared_admission() {
    let original = document(vec![svg_smart(false)]);
    let mut ready = original.clone();
    let mut measured = allowance();
    measured.items = 1;
    prepare_with_allowance(&mut ready, &JobCtx::new(), &mut measured).unwrap();
    assert_eq!(measured.items, 0, "direct vector rendering needs only the outer Smart item");
    let retained = MAX_PREPARATION_BYTES - measured.remaining_bytes;
    let frame = Allowance::render_bytes(&original).unwrap();
    let mut exact = allowance();
    exact.items = 1;
    exact.remaining_bytes = frame + retained;
    let mut direct = original.clone();
    prepare_with_allowance(&mut direct, &JobCtx::new(), &mut exact).unwrap();
    assert_eq!(exact.remaining_bytes, frame);
    assert_eq!(direct, ready);
    let stacked = document(vec![svg_smart(true)]);
    let mut refused = stacked.clone();
    let mut insufficient = allowance();
    insufficient.items = 1;
    insufficient.remaining_bytes = frame + retained;
    assert!(
        prepare_with_allowance(&mut refused, &JobCtx::new(), &mut insufficient).is_err(),
        "stacking must still admit the decoded SVG shape tiles alongside its output"
    );
    assert_eq!(refused, stacked);
    let mut full = allowance();
    full.items = 1;
    prepare_with_allowance(&mut refused, &JobCtx::new(), &mut full).unwrap();
    assert_eq!(full.items, 0, "the tiny imported shape already has a valid native cache");
    let a = ready.layers[0].surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 1, 1));
    let b = refused.layers[0].surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 1, 1));
    assert_eq!(a, b, "one native SVG shape and its Mean stack have the same ordinary pixels");
}

/// Author bounded native pixels and pending automatic filters without precomputing their fixed parameters.
fn automatic_document(layers: usize, filters: usize, uniform: bool) -> (Document, u64) {
    let mut source = Document::new("Original automatic input", Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
    let mut pixels = Layer::raster("Original gradient", source.pixel_format());
    for x in 0..64 {
        let value = if uniform { 0.4 } else { 0.1 + 0.8 * x as f32 / 63.0 };
        pixels.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(x, 0, x + 1, 64), &[value, value, value, 1.0]);
    }
    source.layers.push(pixels);
    let bytes = Arc::new(smart_cmds::encode_source(&source).unwrap());
    let (_, decoded_bytes) = photocraft_format::load_from_bytes_measured(&bytes, &Default::default()).unwrap();
    let mut document = Document::new("Pending sequential analysis", source.size, source.mode, source.depth);
    for _ in 0..layers {
        let mut layer = smart(bytes.clone());
        let LayerContent::Smart(object) = &mut layer.content else { panic!("automatic Smart") };
        object.smart_filters = (0..filters)
            .map(|index| SmartFilter {
                command: if index % 2 == 0 { "image.autoTone" } else { "image.autoContrast" }.into(),
                params: serde_json::json!({}),
                blend: BlendMode::Normal,
                opacity: 1.0,
                visible: true,
            })
            .collect();
        document.layers.push(layer);
    }
    (document, decoded_bytes)
}

/// Permit one-time native analysis but deny publication of its generated fixed curve command.
fn deny_generated_automatic_curve(command: &str, _: &serde_json::Value) -> Result<()> {
    if command == "image.adjustments.curves" { Err(limit("denied fixed automatic curve")) } else { Ok(()) }
}

#[test]
/// Sequential auto filters and layers reuse one live render peak while every decoded tile, cache and fixed parameter stays charged.
fn sequential_automatic_analysis_reuses_prepaid_frames_and_retains_fixed_parameters() {
    for (layers, filters, uniform) in [(1, 3, false), (3, 2, false), (2, 2, true)] {
        let (original, decoded_bytes) = automatic_document(layers, filters, uniform);
        let frame = Allowance::render_bytes(&original).unwrap();
        let mut ready = original.clone();
        let mut measured = allowance();
        prepare_with_allowance(&mut ready, &JobCtx::new(), &mut measured).unwrap();
        let mut caches = allowance();
        for layer in &ready.layers {
            caches.cache(layer.surface().unwrap()).unwrap();
            let LayerContent::Smart(object) = &layer.content else { panic!("prepared Smart") };
            assert!(object.smart_filters.iter().all(|filter| filter.command == if uniform { "image.adjustments.curves" } else { "image.adjustments.levels" }));
        }
        let retained = decoded_bytes * layers as u64 + (MAX_PREPARATION_BYTES - caches.remaining_bytes) + 64 * 1024 * (layers * filters) as u64;
        assert_eq!(
            MAX_PREPARATION_BYTES - measured.remaining_bytes,
            retained,
            "analysis frames are temporary; decoded data, caches and every fixed parameter allowance persist"
        );
        let mut exact = allowance();
        exact.remaining_bytes = frame + retained;
        let mut pending = original.clone();
        prepare_with_allowance(&mut pending, &JobCtx::new(), &mut exact).unwrap();
        assert_eq!(exact.remaining_bytes, frame, "all sequential items release the same render peak");
        assert_eq!(pending, ready);
        let mut short = allowance();
        short.remaining_bytes = frame + retained - 1;
        let mut refused = original.clone();
        assert!(prepare_with_allowance(&mut refused, &JobCtx::new(), &mut short).is_err());
        assert_eq!(refused, original, "a one-byte retained-data shortage cannot publish partial fixed filters");
    }
}

#[test]
/// Analysis credit is validated before work; retained parameter admission survives failure/cancel while temporary credit is released.
fn automatic_analysis_credit_and_retained_parameter_failure_lifetimes_are_distinct() {
    let (original, _) = automatic_document(1, 1, true);
    let original_layer = original.layers[0].clone();
    let LayerContent::Smart(object) = &original_layer.content else { panic!("pending Smart") };
    let SmartSource::Embedded { bytes, .. } = &object.source else { panic!("embedded source") };
    let source = photocraft_format::load_from_bytes(bytes).unwrap();
    let image = smart_cmds::prepared_source_image(&source, original.pixel_format(), None, &JobCtx::new()).unwrap();
    let frame = Allowance::render_bytes(&original).unwrap();
    let mut insufficient = allowance();
    insufficient.remaining_bytes = frame + 64 * 1024;
    let before = insufficient.remaining_bytes;
    let mut layer = original_layer.clone();
    assert!(
        insufficient
            .with_render(&original, |budget, credit, _| crate::import_native_actions::materialize_with_source(
                &original,
                &mut layer,
                &JobCtx::new(),
                credit - 1,
                &mut |bytes| budget.bytes(bytes),
                Some(&image),
            ))
            .is_err()
    );
    assert_eq!(insufficient.remaining_bytes, before, "insufficient prepaid credit must not charge or allocate fixed parameters");
    assert_eq!(layer, original_layer);
    for outcome in 0..3 {
        let context = JobCtx::new();
        let mut layer = original_layer.clone();
        let mut budget = allowance();
        budget.remaining_bytes = frame + 64 * 1024;
        let authority: Option<fn(&str, &serde_json::Value) -> Result<()>> = if outcome == 2 { Some(deny_generated_automatic_curve) } else { None };
        let _scope = crate::jobs::render_authorization_scope(authority);
        let result = budget.with_render(&original, |budget, credit, _| {
            crate::import_native_actions::materialize_with_source(
                &original,
                &mut layer,
                &context,
                credit,
                &mut |bytes| {
                    assert_eq!(bytes, 64 * 1024);
                    budget.bytes(bytes)?;
                    if outcome == 1 {
                        context.cancel();
                        Ok(())
                    } else if outcome == 0 {
                        Err(limit("synthetic fixed-parameter failure"))
                    } else {
                        Ok(())
                    }
                },
                Some(&image),
            )
        });
        if outcome == 1 {
            assert!(matches!(result, Err(EngineError::Cancelled)));
        } else {
            assert!(result.is_err());
        }
        assert_eq!(budget.remaining_bytes, frame, "release only the temporary frame; admitted fixed-parameter bytes stay charged");
        assert_eq!(layer, original_layer, "neither error nor cancellation publishes a partially materialized filter");
    }
}

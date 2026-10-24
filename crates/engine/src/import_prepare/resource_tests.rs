use super::*;
use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Layer, SmartFilter, SmartObject};
use photocraft_geom::{Affine, Size};
use std::cell::{Cell, RefCell};

fn allowance() -> Allowance {
    Allowance { remaining_bytes: MAX_PREPARATION_BYTES, source_bytes: MAX_SOURCE_BYTES, items: MAX_ITEMS, memo: Default::default() }
}

fn source(color: Color) -> Arc<Vec<u8>> {
    let doc = Document::with_background("Original tiny native source", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, color);
    Arc::new(smart_cmds::encode_source(&doc).unwrap())
}

fn smart(bytes: Arc<Vec<u8>>) -> Layer {
    Layer::new(
        "Native smart",
        LayerContent::Smart(SmartObject::new(SmartSource::Embedded { file_name: "source.pcraft".into(), bytes }, Affine::IDENTITY, None)),
    )
}

fn document(layers: Vec<Layer>) -> Document {
    let mut doc = Document::new("Original pending data", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    doc.layers = layers;
    doc
}

#[test]
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
    budget.remaining_bytes = one_unique_work + hit_metadata;
    budget.source_bytes = bytes.len() * 2;
    budget.items = 2;
    prepare_with_allowance(&mut repeated, &JobCtx::new(), &mut budget).unwrap();
    assert_eq!(budget.remaining_bytes, 0);
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
    short_metadata.remaining_bytes = one_unique_work + hit_metadata - 1;
    assert!(prepare_with_allowance(&mut insufficient_metadata, &JobCtx::new(), &mut short_metadata).unwrap_err().to_string().contains("working-byte"));
    assert_eq!(short_metadata.remaining_bytes, hit_metadata - 1, "failed hit preflight does not debit or allocate its clone");
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
        hit_budget.remaining_bytes = unique_work + metadata;
        prepare_with_allowance(&mut repeated, &JobCtx::new(), &mut hit_budget).unwrap();
        assert_eq!(hit_budget.remaining_bytes, 0, "{depth:?}: actual hit charges metadata rather than unique rendering again");
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
        exact.remaining_bytes = render + decoded;
        prepare_with_allowance(&mut parent, &JobCtx::new(), &mut exact).unwrap();
        assert_eq!(exact.remaining_bytes, 0, "same extent reserves its native render exactly once");
        assert_eq!(parent, ordinary, "{depth:?}: admission credit does not change ordinary native pixels or editable data");
    }
}

#[test]
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
        budget.remaining_bytes = source_render + decoded;
        prepare_with_allowance(&mut exact, &JobCtx::new(), &mut budget).unwrap();
        assert_eq!(budget.remaining_bytes, 0, "{depth:?}: larger source's additional extent is actually debited");
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

use super::*;
use craft_psd::{ColorMode, Compression, Version, testgen};

#[test]
fn retained_storage_is_summed_but_workspace_is_reused() {
    let mut budget = ImportBudget::default();
    budget.retain(40).unwrap();
    budget.retain(20).unwrap();
    budget.scratch(30);
    budget.scratch(10);
    assert_eq!(budget.required().unwrap(), 90);
    budget.retain(u64::MAX).unwrap_err();
    budget.workspace = u64::MAX;
    assert!(budget.required().is_err());
}

#[test]
fn tiny_negative_offset_rectangles_pay_for_every_touched_tile() {
    let mut aligned = ImportBudget::default();
    aligned.surface(Rect::from_xywh(0, 0, 2, 2), 4, 2).unwrap();
    let mut offset = ImportBudget::default();
    offset.surface(Rect::from_xywh(-1, -1, 2, 2), 4, 2).unwrap();
    assert_eq!(offset.required().unwrap(), aligned.required().unwrap() * 4);
    offset.surface(Rect::default(), 4, 2).unwrap();
    assert_eq!(offset.required().unwrap(), aligned.required().unwrap() * 4);
}

fn estimate(file: &PsdFile) -> u64 {
    match check(file, 0).unwrap_err() {
        IoError::PsdImportBudget { required, .. } => required,
        error => panic!("unexpected error: {error}"),
    }
}

#[test]
fn individually_fitting_layers_cannot_each_spend_the_import_allowance() {
    let mut file = testgen::small(Version::Psd, Compression::Zip);
    let record = file.layers()[0].clone();
    file.layer_info.as_mut().unwrap().layers = vec![record.clone()];
    let one = estimate(&file);
    assert!(check(&file, one).is_ok());
    file.layer_info.as_mut().unwrap().layers = vec![record; 8];
    assert!(matches!(check(&file, one), Err(IoError::PsdImportBudget { .. })));
}

#[test]
fn large_off_canvas_layer_is_rejected_without_decoding_its_tiny_data() {
    let mut file = testgen::small(Version::Psd, Compression::Zip);
    file.header.width = 1;
    file.header.height = 1;
    file.layers_mut()[0].rect = Rect::from_xywh(-10_000, -10_000, 20_000, 20_000);
    assert!(matches!(check(&file, DEFAULT_PSD_IMPORT_PIXEL_BYTES), Err(IoError::PsdImportBudget { .. })));
    // Both ordinary import routes must fail before reading a deliberately missing merged
    // section. No enormous decoded plane is ever constructed by this test.
    let mut bytes = file.to_bytes().unwrap();
    bytes.truncate(bytes.len() - file.image_data.data.len() - 2);
    crate::memory::with_mock_memory(crate::memory::MemorySnapshot::UNKNOWN, || {
        // Unknown memory keeps a conservative automatic allowance, so this file is refused
        // pending approval (or by the hard platform limit on 32-bit) before any pixel decode.
        assert!(matches!(crate::import("huge.psd", &bytes), Err(IoError::PsdApprovalRequired { .. } | IoError::PsdImportBudget { .. })));
        let source = craft_psd::Source::new(bytes);
        assert!(matches!(
            crate::import_source_for_open_with("huge.psd", source, &photocraft_raster::Interrupt::NONE),
            Err(IoError::PsdApprovalRequired { .. } | IoError::PsdImportBudget { .. })
        ));
    });
}

#[test]
fn user_mask_bounds_count_even_when_the_layer_is_small() {
    let mut file = testgen::small(Version::Psd, Compression::Raw);
    let before = estimate(&file);
    // The generator's first record has a user mask. Modify its parsed mask in place.
    if let craft_psd::layer::MaskData::Mask(mask) = &mut file.layers_mut()[0].mask {
        mask.rect = Rect::from_xywh(0, 0, 20_000, 20_000);
    } else {
        panic!("fixture needs a parsed mask");
    }
    assert!(estimate(&file) > before);
    assert!(matches!(check(&file, before), Err(IoError::PsdImportBudget { .. })));
}

#[test]
fn merged_bitmap_and_multichannel_expansion_are_budgeted() {
    for (mode, depth) in [(ColorMode::Bitmap, 1), (ColorMode::Multichannel, 8), (ColorMode::Rgb, 32)] {
        let mut file = testgen::merged_only(Version::Psb, mode, depth, Compression::Zip, 2, 2);
        let before = estimate(&file);
        assert!(check(&file, before).is_ok());
        file.header.width = 60_000;
        file.header.height = 60_000;
        assert!(matches!(check(&file, DEFAULT_PSD_IMPORT_PIXEL_BYTES), Err(IoError::PsdImportBudget { .. })));
    }
}

#[test]
fn a_custom_limit_is_fatal_for_interactive_and_headless_imports() {
    let file = testgen::small(Version::Psd, Compression::Zip);
    let bytes = file.to_bytes().unwrap();
    let limit = estimate(&file);
    let opts = crate::ImportOptions { psd_max_pixel_bytes: limit, ..Default::default() };
    assert!(crate::import_with_options("small.psd", &bytes, &opts).is_ok());
    let opts = crate::ImportOptions { psd_max_pixel_bytes: limit - 1, ..opts };
    for result in [crate::import_with_options("small.psd", &bytes, &opts), crate::import_for_open("small.psd", &bytes, &opts).map(|(result, _)| result)] {
        assert!(matches!(result, Err(IoError::PsdImportBudget { .. })));
    }
}

#[test]
fn accepted_off_canvas_pixels_and_depths_are_preserved() {
    for depth in [8, 16, 32] {
        for compression in Compression::ALL {
            let file = testgen::layered(Version::Psd, ColorMode::Rgb, depth, compression);
            assert!(check(&file, DEFAULT_PSD_IMPORT_PIXEL_BYTES).is_ok());
            let (doc, _) = crate::psd_to_document(&file);
            assert_eq!(doc.depth, crate::pixels::sample_for_depth(depth));
            let bytes = file.to_bytes().unwrap();
            let opened = crate::import("small.psd", &bytes).unwrap().document;
            let rect = photocraft_geom::Rect::new(-8, -8, 32, 24);
            assert_eq!(photocraft_compose::render(&opened, rect), photocraft_compose::render(&doc, rect));
        }
    }
}

#[test]
fn legacy_conversion_reports_a_rejected_import_instead_of_decoding_it() {
    let mut file = testgen::small(Version::Psd, Compression::Zip);
    file.layers_mut()[0].rect = Rect::from_xywh(0, 0, 20000, 20000);
    // Use fallback snapshot to ensure test does not depend on host available RAM
    crate::memory::with_mock_memory(crate::memory::MemorySnapshot::UNKNOWN, || {
        let (doc, warnings) = crate::psd_to_document(&file);
        assert!(doc.layers.is_empty());
        assert!(warnings.iter().any(|warning| warning.contains("pixel memory")));
    });
}

fn pattern_block(mode: u32, depth: u16, width: u32, height: u32) -> craft_psd::TaggedBlock {
    use craft_psd::patterns::{PsdPattern, mode_channels, write_pattern_block};
    let len = (width as usize * depth as usize).div_ceil(8) * height as usize;
    let p = PsdPattern {
        mode,
        depth,
        width,
        height,
        name: "Budget pattern".into(),
        id: "budget".into(),
        palette: (mode == 2).then(|| vec![0; 768]),
        channels: vec![vec![0; len]; mode_channels(mode)],
        alpha: None,
    };
    craft_psd::TaggedBlock::new(*b"Patt", write_pattern_block(&[p]).unwrap())
}

#[test]
fn pattern_tiles_and_expansion_share_the_import_allowance() {
    for (mode, depth) in [(1, 1), (2, 8), (3, 8), (4, 16), (9, 32)] {
        let mut file = testgen::small(Version::Psd, Compression::Raw);
        let baseline = estimate(&file);
        let block = pattern_block(mode, depth, 9, 3);
        file.global_blocks.push(block.clone());
        let one = estimate(&file);
        assert!(one > baseline);
        assert!(check(&file, one).is_ok());
        assert!(matches!(check(&file, one - 1), Err(IoError::PsdImportBudget { .. })));
        let bytes = file.to_bytes().unwrap();
        let opts = crate::ImportOptions { psd_max_pixel_bytes: one - 1, ..Default::default() };
        assert!(matches!(crate::import_with_options("pattern.psd", &bytes, &opts), Err(IoError::PsdImportBudget { .. })));
        assert_eq!(crate::import("pattern.psd", &bytes).unwrap().document.patterns.len(), 1);
        file.global_blocks.push(block);
        assert!(estimate(&file) > one, "blocks cannot each reuse the allowance");
    }
}

#[test]
fn malformed_later_pattern_still_counts_earlier_decodes() {
    let mut file = testgen::small(Version::Psd, Compression::Raw);
    let baseline = estimate(&file);
    let mut block = pattern_block(1, 1, 9, 3);
    // An invalid second pattern makes the real parser discard the block, after decoding
    // its first record. The preflight must still account for that transient allocation.
    block.data.extend_from_slice(&4u32.to_be_bytes());
    block.data.extend_from_slice(&99u32.to_be_bytes());
    file.global_blocks.push(block);
    assert!(estimate(&file) > baseline);
}

/// An unknown colour mode imports as RGB (`import_format`), layers included, so it must pay
/// for them: a layered file can't dodge the allowance by carrying mode 5.
#[test]
fn an_unknown_color_mode_pays_for_the_layers_it_imports_as_rgb() {
    let rgb = testgen::small(Version::Psd, Compression::Raw);
    assert!(!rgb.layers().is_empty() && !super::super::psd_import::merged_needed(&rgb), "the fixture must import its layers alone");
    let mut unknown = rgb.clone();
    unknown.header.color_mode = craft_psd::ColorMode::Unknown(5);
    assert!(unknown.header.validate().is_ok());
    let required = estimate(&unknown);
    assert!(required >= estimate(&rgb), "the layers import as RGB and are counted");
    let bytes = unknown.to_bytes().unwrap();
    let opts = crate::ImportOptions { psd_max_pixel_bytes: required - 1, ..Default::default() };
    assert!(matches!(crate::import_with_options("mode5.psd", &bytes, &opts), Err(IoError::PsdImportBudget { .. })));
    let source = craft_psd::Source::new(bytes);
    let refused = crate::import_source_with_limit("mode5.psd", source, &photocraft_raster::Interrupt::NONE, required - 1);
    assert!(matches!(refused, Err(IoError::PsdImportBudget { .. })), "{:?}", refused.map(|(r, _)| r.warnings));
}

#[test]
fn adaptive_budget_requires_explicit_approval_in_confirmation_range() {
    let file = testgen::small(Version::Psd, Compression::Zip);
    let bytes = file.to_bytes().unwrap();
    let est = estimate(&file);

    // Set mock RAM such that est is strictly between S and H.
    // E.g., if A = est * 2 + 1 GiB:
    // H = min(75% A, A - 1 GiB) = min(1.5 est + 768 MiB, 2 est) = 1.5 est + 768 MiB > est.
    // S = min(50% A, 8 GiB, H) = min(est + 512 MiB, 8 GiB, H).
    // Let us set A precisely so S < est <= H:
    // If we want S < est, 50% A < est => A < 2 est.
    // If A = (est * 16) / 10 (i.e. 1.6 est):
    // S = 0.8 est < est.
    // H = min(1.2 est, 1.6 est - 1 GiB).
    // To ensure saturating A - 1 GiB is not 0, let A = 10 * (1 << 30) (10 GiB):
    // S = 5 GiB, H = 7.5 GiB.
    // Target estimate est in (5 GiB, 7.5 GiB].
    // With mock memory, we can scale mock RAM:
    let gb = 1u64 << 30;
    // Set RAM such that soft = est - 1, hard = est + 1000:
    // With 12 GiB RAM: S = 6 GiB, H = 9 GiB.
    let snap = crate::memory::MemorySnapshot { available_ram: Some(12 * gb), swap_free: None, commit_headroom: None };
    crate::memory::with_mock_memory(snap, || {
        // Without caller limit, small file is << 6 GiB -> automatically admitted
        assert!(crate::import_with_options("small.psd", &bytes, &crate::ImportOptions::default()).is_ok());

        // Now set caller limit to est + 1:
        // Then hard = est + 1, soft = est + 1.
        let opts = crate::ImportOptions { psd_max_pixel_bytes: est + 1, ..Default::default() };
        assert!(crate::import_with_options("small.psd", &bytes, &opts).is_ok());

        // Now set caller limit to est - 1:
        // Then hard = est - 1 -> refused
        let opts = crate::ImportOptions { psd_max_pixel_bytes: est - 1, ..Default::default() };
        assert!(matches!(crate::import_with_options("small.psd", &bytes, &opts), Err(IoError::PsdImportBudget { .. })));
    });
}

#[test]
fn explicit_approval_allows_confirmation_range_but_never_bypasses_hard_boundary() {
    if cfg!(any(target_arch = "wasm32", target_pointer_width = "32")) {
        return; // the 6 GiB hard limit is above the 32-bit aggregate cap
    }
    let gb = 1u64 << 30;

    // Scale RAM so that est falls between S and H:
    // S = 50% A, H = 75% A.
    // We want S < est <= H.
    // So 0.50 A < est <= 0.75 A => (4/3) est <= A < 2 est.
    // If A = 1.6 * est, then S = 0.8 est, H_pct = 1.2 est.
    // To satisfy saturating A - 1 GiB: we test pure policy decision:
    let snap = crate::memory::MemorySnapshot { available_ram: Some(8 * gb), swap_free: None, commit_headroom: None };
    let budget = crate::memory::resolve_budget(snap, None);
    assert_eq!(budget.soft_limit, 4 * gb);
    assert_eq!(budget.hard_limit, 6 * gb);

    // In confirmation range:
    let mid = 5 * gb;
    assert_eq!(budget.decide(mid), crate::memory::BudgetDecision::RequiresApproval);

    // Over hard limit:
    let over = 7 * gb;
    assert_eq!(budget.decide(over), crate::memory::BudgetDecision::Refused);
}

#[test]
fn zero_available_ram_refuses_admission() {
    if cfg!(target_arch = "wasm32") {
        return; // the web policy ignores a host snapshot; it has its own tests
    }
    let file = testgen::small(Version::Psd, Compression::Zip);
    let bytes = file.to_bytes().unwrap();

    let snap = crate::memory::MemorySnapshot { available_ram: Some(0), swap_free: None, commit_headroom: None };
    crate::memory::with_mock_memory(snap, || {
        let opts = crate::ImportOptions::default();
        let res = crate::import_with_options("small.psd", &bytes, &opts);
        assert!(matches!(res, Err(IoError::PsdImportBudget { .. })));
    });
}

/// A small file opens with 900 MiB free (it used to be refused outright below 1 GiB); with no
/// free memory at all it is refused and the refusal says why.
#[test]
fn small_file_opens_with_little_free_memory_and_a_refusal_names_it() {
    if cfg!(target_arch = "wasm32") {
        return; // the web policy ignores a host snapshot; it has its own tests
    }
    let file = testgen::small(Version::Psd, Compression::Zip);
    let bytes = file.to_bytes().unwrap();

    let low = crate::memory::MemorySnapshot { available_ram: Some(900 << 20), swap_free: None, commit_headroom: None };
    crate::memory::with_mock_memory(low, || {
        let res = crate::import_with_options("small.psd", &bytes, &crate::ImportOptions::default());
        assert!(res.is_ok(), "{:?}", res.map(|_| ()));
    });

    let none = crate::memory::MemorySnapshot { available_ram: Some(0), swap_free: None, commit_headroom: None };
    crate::memory::with_mock_memory(none, || {
        let res = crate::import_with_options("small.psd", &bytes, &crate::ImportOptions::default());
        assert!(matches!(res, Err(IoError::PsdImportBudget { limit: 0, free_memory: Some(0), .. })));
    });
}

#[test]
fn preflight_rejection_and_approval_files_release_gate() {
    if cfg!(any(target_arch = "wasm32", target_pointer_width = "32")) {
        return;
    }
    let mut file = testgen::small(Version::Psd, Compression::Zip);
    file.header.width = 1;
    file.header.height = 1;
    file.layers_mut()[0].rect = Rect::from_xywh(0, 0, 30_000, 30_000);
    let bytes = file.to_bytes().unwrap();

    let gb = 1u64 << 30;
    // Set RAM such that this large layer exceeds hard limit
    let snap = crate::memory::MemorySnapshot { available_ram: Some(4 * gb), swap_free: None, commit_headroom: None };
    crate::memory::with_mock_memory(snap, || {
        assert!(!crate::psd_gate::is_holding_admission());
        let res = crate::import_with_options("huge.psd", &bytes, &crate::ImportOptions::default());
        assert!(matches!(res, Err(IoError::PsdImportBudget { .. })));
        // Gate must not be held after rejection
        assert!(!crate::psd_gate::is_holding_admission());
    });

    // Set RAM such that file falls in confirmation range
    let est = estimate(&file);
    let snap_mid = crate::memory::MemorySnapshot { available_ram: Some(est * 2), swap_free: None, commit_headroom: None };
    crate::memory::with_mock_memory(snap_mid, || {
        assert!(!crate::psd_gate::is_holding_admission());
        let res = crate::import_with_options("mid.psd", &bytes, &crate::ImportOptions::default());
        assert!(matches!(res, Err(IoError::PsdApprovalRequired { .. })));
        // Gate must not be held when requiring approval
        assert!(!crate::psd_gate::is_holding_admission());
    });
}

#[test]
fn retry_estimate_cap_rejects_altered_or_enlarged_file() {
    if cfg!(any(target_arch = "wasm32", target_pointer_width = "32")) {
        return;
    }
    let gb = 1u64 << 30;
    let snap = crate::memory::MemorySnapshot { available_ram: Some(12 * gb), swap_free: None, commit_headroom: None };
    // S = 6 GiB, H = 9 GiB

    // 1. Within approved_estimate: passes
    let res = crate::memory::evaluate_psd_admission(7 * gb, 0, 0, true, Some(7 * gb), snap);
    assert!(res.is_ok());

    // 2. Fresh requirement exceeds approved_estimate: requires fresh approval
    let res = crate::memory::evaluate_psd_admission(8 * gb, 0, 0, true, Some(7 * gb), snap);
    assert!(matches!(res, Err(IoError::PsdApprovalRequired { required, .. }) if required == 8 * gb));

    // 3. Exceeds hard limit: refused completely regardless of approval
    let res = crate::memory::evaluate_psd_admission(10 * gb, 0, 0, true, Some(10 * gb), snap);
    assert!(matches!(res, Err(IoError::PsdImportBudget { required, limit, .. }) if required == 10 * gb && limit == 9 * gb));
}

#[test]
fn caller_zero_and_lower_caps_cannot_bypass_hard_boundary() {
    if cfg!(any(target_arch = "wasm32", target_pointer_width = "32")) {
        return;
    }
    let gb = 1u64 << 30;
    let snap = crate::memory::MemorySnapshot { available_ram: Some(8 * gb), swap_free: None, commit_headroom: None };
    // S = 4 GiB, H = 6 GiB

    // 0 uses adaptive budget
    let b_zero = crate::memory::resolve_budget(snap, Some(0));
    assert_eq!(b_zero.hard_limit, 6 * gb);
    assert_eq!(b_zero.soft_limit, 4 * gb);

    // Lower cap lowers both hard and soft
    let b_lower = crate::memory::resolve_budget(snap, Some(3 * gb));
    assert_eq!(b_lower.hard_limit, 3 * gb);
    assert_eq!(b_lower.soft_limit, 3 * gb);

    // Higher cap cannot raise above hard limit
    let b_higher = crate::memory::resolve_budget(snap, Some(20 * gb));
    assert_eq!(b_higher.hard_limit, 6 * gb);
    assert_eq!(b_higher.soft_limit, 4 * gb);
}

#[test]
fn test_memory_scope_propagates_to_worker_thread() {
    let gb = 1u64 << 30;
    let mock = crate::memory::MemorySnapshot { available_ram: Some(4 * gb), swap_free: None, commit_headroom: None };
    crate::memory::with_mock_memory(mock, || {
        let scope = crate::memory::TestMemoryScope::capture();
        let handle = std::thread::spawn(move || {
            scope.run(|| {
                assert_eq!(crate::memory::MemorySnapshot::current().available_ram, Some(4 * gb));
            });
        });
        handle.join().unwrap();
    });
}

#[test]
fn test_memory_scope_propagates_the_estimate_charge_to_a_worker() {
    let gb = 1u64 << 30;
    let file = std::sync::Arc::new(testgen::small(Version::Psd, Compression::Zip));
    let bare = estimate(&file);
    with_test_estimate_charge(2 * gb, || {
        assert_eq!(estimate(&file), bare + 2 * gb, "the charge is part of the estimate on this thread");
        let scope = crate::memory::TestMemoryScope::capture();
        let worker_file = std::sync::Arc::clone(&file);
        std::thread::spawn(move || {
            scope.run(|| assert_eq!(estimate(&worker_file), bare + 2 * gb, "a captured scope carries the charge to its worker"));
        })
        .join()
        .unwrap();
    });
    assert_eq!(estimate(&file), bare, "the charge is restored on this thread");
    let detached = std::sync::Arc::clone(&file);
    assert_eq!(std::thread::spawn(move || estimate(&detached)).join().unwrap(), bare, "a worker outside the scope never sees it");
}

#[test]
fn nested_imports_consume_from_shared_allowance_and_do_not_reset_parent() {
    let gb = 1u64 << 30;
    let ctl = photocraft_raster::Interrupt::NONE;
    let _guard = crate::psd_gate::acquire_decode_admission(&ctl).unwrap();
    // Simulate top-level attempt authorized total of 5 GB with 2 GB remaining allowance
    crate::psd_gate::set_active_attempt_budget(2 * gb);

    // First nested consume fits within 2 GB
    assert!(crate::psd_gate::consume_attempt_allowance(gb).is_ok());

    // Second nested consume exceeds remaining 1 GB
    let err = crate::psd_gate::consume_attempt_allowance(2 * gb).unwrap_err();
    assert!(matches!(err, IoError::PsdImportBudget { required, limit, .. } if required == 2 * gb && limit == gb));

    // Allowance is still 1 GB (failed attempt does not reset or corrupt parent)
    assert!(crate::psd_gate::consume_attempt_allowance(gb).is_ok());
    assert!(crate::psd_gate::consume_attempt_allowance(1).is_err());
}

#[test]
fn nested_tiff_layer_import_consumes_composite_and_estimate_without_resetting_parent() {
    let gb = 1u64 << 30;
    let ctl = photocraft_raster::Interrupt::NONE;
    let _guard = crate::psd_gate::acquire_decode_admission(&ctl).unwrap();
    crate::psd_gate::set_active_attempt_budget(3 * gb);

    assert!(crate::psd_gate::is_holding_admission());
    assert!(crate::psd_gate::consume_attempt_allowance(gb).is_ok());
    assert!(crate::psd_gate::consume_attempt_allowance(2 * gb).is_ok());
    assert!(crate::psd_gate::consume_attempt_allowance(1).is_err());
}

/// A real import carries the approval flag and its estimate bound into admission: the fixture is
/// tiny, and a thread-local test charge puts it in the confirmation band without allocating
/// gigabytes.
#[test]
fn real_import_propagates_high_memory_approval_and_its_estimate_bound() {
    if cfg!(any(target_arch = "wasm32", target_pointer_width = "32")) {
        return;
    }
    let gb = 1u64 << 30;
    let file = testgen::small(Version::Psd, Compression::Zip);
    let bytes = file.to_bytes().unwrap();
    let snapshot = crate::memory::MemorySnapshot { available_ram: Some(12 * gb), swap_free: None, commit_headroom: None };
    crate::memory::with_mock_memory(snapshot, || {
        let automatic_layers =
            crate::import_with_options("approval.psd", &bytes, &crate::ImportOptions::default()).expect("small file is automatic").document.layers.len();
        with_test_estimate_charge(7 * gb, || {
            let required = match crate::import_with_options("approval.psd", &bytes, &crate::ImportOptions::default()) {
                Err(IoError::PsdApprovalRequired { required, soft_limit, hard_limit, available_ram }) => {
                    assert_eq!((soft_limit, hard_limit), (6 * gb, 9 * gb));
                    assert_eq!(available_ram, Some(12 * gb));
                    required
                }
                other => panic!("expected an approval request, got {other:?}"),
            };
            assert!((7 * gb..=9 * gb).contains(&required), "the charge lands in the approval band: {required}");

            let approved = crate::ImportOptions { psd_high_memory_approved: true, ..Default::default() };
            assert_eq!(
                crate::import_with_options("approval.psd", &bytes, &approved).expect("approved").document.layers.len(),
                automatic_layers,
                "an approved import decodes the same document as an automatic one"
            );
            let bounded = crate::ImportOptions { psd_high_memory_approved: true, psd_approved_estimate: Some(required), ..Default::default() };
            assert!(crate::import_with_options("approval.psd", &bytes, &bounded).is_ok());
            let stale = crate::ImportOptions { psd_high_memory_approved: true, psd_approved_estimate: Some(required - 1), ..Default::default() };
            assert!(matches!(crate::import_with_options("approval.psd", &bytes, &stale), Err(IoError::PsdApprovalRequired { .. })));
            let lowered = crate::ImportOptions { psd_high_memory_approved: true, psd_max_pixel_bytes: required - 1, ..Default::default() };
            assert!(matches!(
                crate::import_with_options("approval.psd", &bytes, &lowered),
                Err(IoError::PsdImportBudget { required: r, limit: l, .. }) if r == required && l == required - 1
            ));
        });
    });
}

/// Available memory can drop while an import waits for the decode gate. The post-gate recheck
/// must then refuse before anything is decoded. A per-attempt provider, shared with this test
/// only, changes the measurement while the worker is blocked; nothing near a gigabyte is
/// allocated, and the real document is never opened.
#[test]
fn memory_drop_while_waiting_for_the_gate_refuses_before_decoding() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    if cfg!(any(target_arch = "wasm32", target_pointer_width = "32")) {
        return;
    }
    let gb = 1u64 << 30;
    let file = testgen::small(Version::Psd, Compression::Zip);
    let bytes = file.to_bytes().unwrap();

    // Hold the gate so the worker stops after its first admission check.
    let _guard = crate::psd_gate::acquire_decode_admission(&photocraft_raster::Interrupt::NONE).expect("active decode");

    let available = Arc::new(AtomicU64::new(32 * gb));
    let prechecked = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = {
        let available = Arc::clone(&available);
        let prechecked = Arc::clone(&prechecked);
        std::thread::spawn(move || {
            let provider = std::rc::Rc::new(move || {
                prechecked.store(true, Ordering::SeqCst);
                crate::memory::MemorySnapshot { available_ram: Some(available.load(Ordering::SeqCst)), swap_free: None, commit_headroom: None }
            });
            let result =
                crate::memory::with_mock_memory_provider(provider, || crate::import_with_options("drop.psd", &bytes, &crate::ImportOptions::default()));
            tx.send(result.map(|r| r.document.layers.len())).ok();
        })
    };

    // The provider's first call is the pre-gate admission check.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !prechecked.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "the import never reached its admission check");
        std::thread::yield_now();
    }
    // The precheck admitted it; now the machine has nothing left. Release the gate afterwards.
    available.store(0, Ordering::SeqCst);
    drop(_guard);

    let result = rx.recv_timeout(Duration::from_secs(10)).expect("the post-gate recheck must refuse instead of decoding");
    match result {
        Err(IoError::PsdImportBudget { required, limit: 0, .. }) => assert!(required > 0),
        other => panic!("expected the post-gate recheck to refuse with a zero budget, got {other:?}"),
    }
    handle.join().expect("worker thread");
}

/// A real import counts its tile pixels as spillable: with a spilling scratch disk they are
/// admitted against RAM plus its room, while non-spillable working memory still needs RAM, and
/// with spilling off the RAM-only policy applies unchanged. Nothing near a gigabyte is allocated.
#[test]
fn real_import_admits_tile_pixels_against_scratch_room_only_while_spilling() {
    if cfg!(any(target_arch = "wasm32", target_pointer_width = "32")) {
        return;
    }
    let gb = 1u64 << 30;
    let file = testgen::small(Version::Psd, Compression::Zip);
    let bytes = file.to_bytes().unwrap();
    let est = estimate(&file);
    let snapshot = crate::memory::MemorySnapshot { available_ram: Some(12 * gb), swap_free: None, commit_headroom: None };
    let import = |opts: &crate::ImportOptions| crate::import_with_options("scratch.psd", &bytes, opts);
    crate::memory::with_mock_memory(snapshot, || {
        assert!(super::estimate(&file).unwrap().tile_bytes > 0, "layer surfaces are tile pixels");
        with_test_tile_charge(18 * gb, || {
            let charged = super::estimate(&file).unwrap();
            assert_eq!(charged.total_bytes, est + 18 * gb);
            // Spilling off (or paused): 18 GiB of tiles is over H = 9 GiB.
            crate::memory::with_mock_scratch(None, || {
                assert!(matches!(import(&Default::default()), Err(IoError::PsdImportBudget { limit, .. }) if limit == 9 * gb));
            });
            // Spilling with room for every tile: automatic, no approval.
            crate::memory::with_mock_scratch(Some(40 * gb), || {
                assert!(import(&Default::default()).is_ok());
            });
            // Room for only part of the tiles: the rest must fit RAM.
            crate::memory::with_mock_scratch(Some(4 * gb), || {
                assert!(matches!(import(&Default::default()), Err(IoError::PsdImportBudget { .. })));
            });
        });
        // Non-spillable memory gets no scratch credit: 7 GiB still asks for approval.
        with_test_estimate_charge(7 * gb, || {
            crate::memory::with_mock_scratch(Some(40 * gb), || match import(&Default::default()) {
                Err(IoError::PsdApprovalRequired { required, soft_limit, .. }) => {
                    let tiles = super::estimate(&file).unwrap().tile_bytes;
                    assert_eq!(soft_limit, 6 * gb + tiles);
                    assert!(required > soft_limit);
                }
                other => panic!("expected an approval request, got {other:?}"),
            });
        });
    });
}

/// A shape record without pixels whose path is just the initial-fill record: an inverted shape,
/// which covers the whole canvas.
fn inverted_shape_record(fill: photocraft_doc::Fill) -> craft_psd::LayerRecord {
    use craft_psd::path::{PathData, VectorMaskBlock};
    let vsms = VectorMaskBlock { version: 3, flags: 0, path: PathData::from_subpaths(&[], true) }.to_bytes();
    let (key, data) = crate::blocks::write_fill(&fill);
    craft_psd::LayerRecord {
        name: b"shape".to_vec(),
        blocks: vec![craft_psd::TaggedBlock::new(*b"vsms", vsms), craft_psd::TaggedBlock::new(key, data), craft_psd::TaggedBlock::unicode_name("shape")],
        ..Default::default()
    }
}

/// A shape the import renders from its path is charged before any pixel is decoded: an
/// inverted one covers the whole canvas, which a record of a few bytes can make 300 000 px wide.
/// A small one still opens, rendered (an inverted shape really covers the canvas).
#[test]
fn shapes_rendered_from_their_path_are_charged_before_decoding() {
    let blue = photocraft_color::Color::rgb(0.0, 0.0, 1.0);
    let mut file = testgen::small(Version::Psb, Compression::Raw);
    let before = estimate(&file);
    file.layers_mut().push(inverted_shape_record(photocraft_doc::Fill::Solid(blue)));
    // One more RGBA tile at least: the 4x3 canvas the shape covers.
    assert!(estimate(&file) >= before + 256 * 256 * 4, "{} vs {before}", estimate(&file));

    let bytes = file.to_bytes().unwrap();
    let doc = crate::import("small.psb", &bytes).unwrap().document;
    let shape = doc.layers.iter().find(|l| l.name == "shape").expect("the shape layer");
    let photocraft_doc::LayerContent::Shape(sh) = &shape.content else { panic!("{:?}", shape.content) };
    let cache = sh.cache.as_ref().expect("rendered");
    assert_eq!(cache.content_bounds(), photocraft_geom::Rect::new(0, 0, 4, 3));
    assert_eq!(cache.pixel(3, 2), vec![0.0, 0.0, 1.0, 1.0]);

    file.header.width = 300_000;
    file.header.height = 4096;
    assert!(matches!(check(&file, DEFAULT_PSD_IMPORT_PIXEL_BYTES), Err(IoError::PsdImportBudget { .. })));
    // Without the merged image, which the refusal must come before.
    let mut bytes = file.to_bytes().unwrap();
    bytes.truncate(bytes.len() - file.image_data.data.len() - 2);
    crate::memory::with_mock_memory(crate::memory::MemorySnapshot::UNKNOWN, || {
        assert!(matches!(crate::import("huge.psb", &bytes), Err(IoError::PsdApprovalRequired { .. } | IoError::PsdImportBudget { .. })));
    });
}

/// A shape that keeps its stored pixels (hard, with pixels) or renders only its own path costs
/// no canvas: a small path on a huge canvas is charged for its tiles alone.
#[test]
fn a_small_rendered_shape_is_charged_for_its_own_area() {
    use craft_psd::path::{PathData, PsdKnot, PsdSubpath, VectorMaskBlock};
    let mut file = testgen::small(Version::Psb, Compression::Raw);
    file.header.width = 300_000;
    file.header.height = 4096;
    let without = estimate(&file);
    // A 10 px square at (0, 0): path coordinates are fractions of the canvas.
    let (w, h) = (300_000.0, 4096.0);
    let knot = |x: f64, y: f64| PsdKnot { linked: false, pre: (x / w, y / h), anchor: (x / w, y / h), post: (x / w, y / h) };
    let square = PsdSubpath { closed: true, operation: 1, knots: vec![knot(0.0, 0.0), knot(10.0, 0.0), knot(10.0, 10.0), knot(0.0, 10.0)] };
    let vsms = VectorMaskBlock { version: 3, flags: 0, path: PathData::from_subpaths(&[square], false) }.to_bytes();
    let mut rec = inverted_shape_record(photocraft_doc::Fill::Solid(photocraft_color::Color::BLACK));
    rec.blocks[0] = craft_psd::TaggedBlock::new(*b"vsms", vsms);
    file.layers_mut().push(rec);
    let with = estimate(&file);
    assert!(with > without, "the square's tile is charged");
    assert!(with - without < 64 << 20, "{} more", with - without);
}

/// A hard shape with stored pixels: testgen's 4x3 layer given a blue fill inverted around a
/// rectangle (a leading subtract) with a 1 px stroke along it, under a clipped layer, on a
/// `w`x`h` canvas.
fn stored_stroked_shape(w: u32, h: u32) -> PsdFile {
    use craft_psd::path::{PathData, PsdKnot, PsdSubpath, VectorMaskBlock};
    let mut file = testgen::small(Version::Psd, Compression::Raw);
    file.header.width = w;
    file.header.height = h;
    let (fw, fh) = (f64::from(w), f64::from(h));
    let knot = |x: f64, y: f64| PsdKnot { linked: false, pre: (x / fw, y / fh), anchor: (x / fw, y / fh), post: (x / fw, y / fh) };
    let rect = PsdSubpath { closed: true, operation: 2, knots: vec![knot(0.5, 0.5), knot(6.5, 0.5), knot(6.5, 5.5), knot(0.5, 5.5)] };
    let vsms = VectorMaskBlock { version: 3, flags: 0, path: PathData::from_subpaths(&[rect], false) }.to_bytes();
    let (key, fill) = crate::blocks::write_fill(&photocraft_doc::Fill::Solid(photocraft_color::Color::rgb(0.0, 0.0, 1.0)));
    let stroke = photocraft_doc::ShapeStroke { width: 1.0, ..Default::default() };
    let mut shape = file.layers()[0].clone();
    shape.blocks.extend([
        craft_psd::TaggedBlock::new(*b"vsms", vsms),
        craft_psd::TaggedBlock::new(key, fill),
        craft_psd::TaggedBlock::new(*b"vstk", crate::vector_map::vstk_bytes(Some(&stroke), true, 72.0)),
    ]);
    let mut clipped = file.layers()[2].clone();
    clipped.clipping = 1;
    file.layers_mut().clear();
    file.layers_mut().extend([shape, clipped]);
    file
}

/// The document without the shape's stroke: no split, its stored pixels under the clipped layer.
fn unsplit(doc: &photocraft_doc::Document) -> photocraft_doc::Document {
    let mut doc = doc.clone();
    if let photocraft_doc::LayerContent::Shape(sh) = &mut doc.layers[0].content {
        sh.stroke = None;
    }
    doc
}

/// A hard shape keeps its stored pixels at import, so it costs no render there. Composited
/// under a visible clipped layer, its fill and stroke are split over the canvas to stack the
/// stroke above that layer; the split is sized before it is drawn, so an inverted fill over a
/// huge canvas isn't split, even for the first, small composite: the stored pixels composite as
/// they are. A small canvas still splits.
#[test]
fn a_stored_shape_is_not_split_over_a_huge_canvas_when_composited() {
    let viewport = photocraft_geom::Rect::new(0, 0, 8, 8);
    let (small, _) = crate::psd_to_document(&stored_stroked_shape(64, 48));
    let photocraft_doc::LayerContent::Shape(sh) = &small.layers[0].content else { panic!("{:?}", small.layers[0].content) };
    assert!(small.layers[1].clipped && sh.stroke.is_some() && sh.path.inverted && !sh.path.subpaths.is_empty());
    assert!(
        photocraft_compose::render(&small, viewport) != photocraft_compose::render(&unsplit(&small), viewport),
        "the stroke stacks above the clipped layer"
    );

    // 30000x30000: the inverted fill alone would draw 3.6 GB, past the split's draw limit.
    let huge = stored_stroked_shape(30_000, 30_000);
    assert!(check(&huge, DEFAULT_PSD_IMPORT_PIXEL_BYTES).is_ok(), "the stored pixels are all the import keeps");
    let (doc, warnings) = crate::psd_to_document(&huge);
    assert_eq!(doc.layers.len(), 2, "{warnings:?}");
    // Composited cold: nothing has measured this shape yet.
    let first = photocraft_compose::render(&doc, viewport);
    assert!(first == photocraft_compose::render(&unsplit(&doc), viewport), "the stored pixels, unsplit");
}

/// A folder is imported as a group, never as a shape: a Photoshop Frame's stroke (`vstk`) along
/// its vector mask is drawn by the compositor, not rendered at import, so a frame around a huge
/// canvas costs the import nothing and the file opens.
#[test]
fn a_folder_with_a_frame_stroke_is_not_charged_as_a_shape() {
    use craft_psd::path::{PathData, PsdKnot, PsdSubpath, VectorMaskBlock};
    let mut file = testgen::small(Version::Psb, Compression::Raw);
    let before = estimate(&file);
    file.header.width = 300_000;
    file.header.height = 4096;
    let corner = |x: f64, y: f64| PsdKnot { linked: false, pre: (x, y), anchor: (x, y), post: (x, y) };
    let frame = PsdSubpath { closed: true, operation: 1, knots: vec![corner(0.0, 0.0), corner(1.0, 0.0), corner(1.0, 1.0), corner(0.0, 1.0)] };
    let vsms = VectorMaskBlock { version: 3, flags: 0, path: PathData::from_subpaths(&[frame], false) }.to_bytes();
    let stroke = photocraft_doc::ShapeStroke { width: 8.0, ..Default::default() };
    let folder = &mut file.layers_mut()[3];
    assert!(folder.section_type().is_folder());
    folder.blocks.push(craft_psd::TaggedBlock::new(*b"vsms", vsms));
    folder.blocks.push(craft_psd::TaggedBlock::new(*b"vstk", crate::vector_map::vstk_bytes(Some(&stroke), true, 72.0)));
    let est = check(&file, DEFAULT_PSD_IMPORT_PIXEL_BYTES).expect("the frame costs the import nothing");
    assert_eq!(est.total_bytes, before, "only the layers' own pixels");
    let (doc, warnings) = crate::psd_to_document(&file);
    let group = doc.layers.iter().find(|l| l.is_group()).unwrap_or_else(|| panic!("{warnings:?}"));
    assert!(group.vector_mask.is_some());
    let photocraft_doc::LayerContent::Group(g) = &group.content else { panic!("{:?}", group.content) };
    assert!(g.frame_stroke.is_some(), "the frame keeps its stroke");
}

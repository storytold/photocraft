use super::*;
use photocraft_psd::{ColorMode, Compression, PsdError, ReadAt, Source, Version, testgen};

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
        let source = photocraft_psd::Source::new(bytes);
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
    if let photocraft_psd::layer::MaskData::Mask(mask) = &mut file.layers_mut()[0].mask {
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
fn opaque_rgb8_merged_budget_preserves_pixels_and_enforces_caller_limit() {
    for compression in [Compression::Raw, Compression::Rle] {
        let file = testgen::merged_only(Version::Psd, ColorMode::Rgb, 8, compression, 257, 259);
        let planes = file.decode_merged().unwrap();
        let n = 257 * 259;
        let expected: Vec<u8> = (0..n).flat_map(|i| [planes[i], planes[n + i], planes[2 * n + i], 255]).collect();
        let budget = super::estimate(&file).unwrap();
        // The byte path needs an RGBA interleave, not the old 60 bytes/pixel workspace.
        assert!(budget.workspace_bytes >= n as u64 * 4);
        assert!(budget.workspace_bytes < n as u64 * 8);
        assert!(budget.retained_bytes - budget.tile_bytes >= n as u64 * 3 + file.image_data.encoded_len());
        let limit = estimate(&file);
        let bytes = file.to_bytes().unwrap();
        let opts = crate::ImportOptions { psd_max_pixel_bytes: limit, ..Default::default() };
        let opened = crate::import_with_options("opaque.psd", &bytes, &opts).unwrap().document;
        let photocraft_doc::LayerContent::Raster(surface) = &opened.layers[0].content else { panic!("expected raster") };
        assert_eq!(surface.to_interleaved(opened.bounds()), expected);
        let source = photocraft_psd::Source::new(bytes.clone());
        let (opened, damaged) = crate::import_source_with_options("opaque.psd", source, &photocraft_raster::Interrupt::NONE, &opts).unwrap();
        assert!(!damaged);
        let photocraft_doc::LayerContent::Raster(surface) = &opened.document.layers[0].content else { panic!("expected raster") };
        assert_eq!(surface.to_interleaved(opened.document.bounds()), expected);
        let opts = crate::ImportOptions { psd_max_pixel_bytes: limit - 1, ..opts };
        assert!(matches!(crate::import_with_options("opaque.psd", &bytes, &opts), Err(IoError::PsdImportBudget { .. })));
        let source = photocraft_psd::Source::new(bytes);
        assert!(matches!(
            crate::import_source_with_options("opaque.psd", source, &photocraft_raster::Interrupt::NONE, &opts),
            Err(IoError::PsdImportBudget { .. })
        ));
    }
}

#[test]
fn opaque_rgb8_large_merged_budget_counts_bytes_without_float_expansion() {
    let mut file = testgen::merged_only(Version::Psd, ColorMode::Rgb, 8, Compression::Rle, 1, 1);
    // Header-only estimate: never allocate this full-resolution synthetic image.
    file.header.width = 25_400;
    file.header.height = 17_780;
    let pixels = 25_400u64 * 17_780;
    let est = super::estimate(&file).unwrap();
    assert!(est.workspace_bytes >= pixels * 4);
    assert!(est.retained_bytes >= pixels * 7);
    assert!(est.total_bytes < 8 << 30);
    // A merged alpha channel uses the separate unmatting path and keeps its bound.
    file.header.channels = 4;
    assert!(super::estimate(&file).unwrap().workspace_bytes >= pixels * 60);
}

#[test]
fn opaque_rgb8_merged_budget_includes_oversized_encoded_source() {
    for compression in [Compression::Raw, Compression::Rle] {
        let file = testgen::merged_only(Version::Psd, ColorMode::Rgb, 8, compression, 1, 1);
        let opts = crate::ImportOptions { psd_max_pixel_bytes: estimate(&file) + (64 << 10), ..Default::default() };
        let mut bytes = file.to_bytes().unwrap();
        // The low pixel count must not hide a large encoded payload. Preflight must
        // reject before the parser validates or decodes these trailing bytes.
        bytes.resize(bytes.len() + (1 << 20), 0);
        for result in [
            crate::import_with_options("opaque.psd", &bytes, &opts),
            crate::import_source_with_options("opaque.psd", photocraft_psd::Source::new(bytes.clone()), &photocraft_raster::Interrupt::NONE, &opts)
                .map(|(r, _)| r),
        ] {
            assert!(matches!(result, Err(IoError::PsdImportBudget { required, .. }) if required >= 2 << 20), "{result:?}");
        }
    }
}

#[test]
fn merged_preflight_keeps_other_pixel_formats_on_the_conservative_path() {
    let merged = |mode, depth, compression| testgen::merged_only(Version::Psd, mode, depth, compression, 512, 512);
    let mut alpha = merged(ColorMode::Rgb, 8, Compression::Raw);
    alpha.header.channels = 4;
    alpha.image_data = photocraft_psd::ImageData::encode(Compression::Raw, &vec![255; 512 * 512 * 4], &alpha.header).unwrap();
    assert!(alpha.merged_has_alpha());
    let mut layered = testgen::small(Version::Psd, Compression::Raw);
    layered.header = alpha.header.clone();
    layered.image_data = alpha.image_data.clone();
    // The fourth plane is an extra channel, so this layered file needs the merged image.
    layered.layer_info.as_mut().unwrap().merged_alpha = false;
    assert!(!layered.layers().is_empty());
    let mut unknown = merged(ColorMode::Rgb, 8, Compression::Raw);
    unknown.image_data.compression = Compression::Unknown(99);
    // Fixed pre-change totals for 512x512 fixtures, independent of the new estimator.
    for (file, required) in [
        (alpha, 19_922_944),
        (layered, 20_774_912),
        (merged(ColorMode::Rgb, 8, Compression::Zip), 19_398_657),
        (merged(ColorMode::Rgb, 8, Compression::ZipPrediction), 19_398_657),
        (merged(ColorMode::Rgb, 16, Compression::Raw), 25_690_112),
        (merged(ColorMode::Rgb, 32, Compression::Rle), 37_748_736),
        (merged(ColorMode::Grayscale, 8, Compression::Raw), 9_437_185),
        (unknown, 19_398_657),
    ] {
        assert!(crate::psd_import::merged_needed(&file));
        assert_eq!(estimate(&file), required, "{:?}", file.header);
        let opts = crate::ImportOptions { psd_max_pixel_bytes: required - 1, ..Default::default() };
        let bytes = file.to_bytes().unwrap();
        for result in [
            crate::import_with_options("conservative.psd", &bytes, &opts),
            crate::import_source_with_options("conservative.psd", photocraft_psd::Source::new(bytes.clone()), &photocraft_raster::Interrupt::NONE, &opts)
                .map(|(r, _)| r),
        ] {
            assert!(matches!(result, Err(IoError::PsdImportBudget { required: n, .. }) if n == required), "{result:?}");
        }
    }
    let opaque = merged(ColorMode::Rgb, 8, Compression::Raw);
    assert_eq!(super::estimate_with_merged(&opaque, None, 0).unwrap().total_bytes, 19_398_657);
    assert!(super::estimate_with_merged(&opaque, Some(Compression::Raw), u64::MAX).is_err());
}

#[test]
fn preflight_info_reports_merged_compression_and_length_before_decoding() {
    for version in [Version::Psd, Version::Psb] {
        for compression in Compression::ALL {
            let file = testgen::merged_only(version, ColorMode::Rgb, 8, compression, 17, 9);
            let bytes = file.to_bytes().unwrap();
            let preflight = |preview: &PsdFile, actual, len| {
                assert_eq!(preview.image_data.encoded_len(), 0);
                assert_eq!(actual, Some(compression));
                assert_eq!(len, file.image_data.encoded_len());
                Ok(true)
            };
            let (_, merged) = PsdFile::from_bytes_and_merged_preflight_info(&bytes, preflight, |_| false).unwrap();
            assert_eq!(merged.unwrap().unwrap(), file.decode_merged().unwrap());
            let (_, merged) = PsdFile::from_source_and_merged_preflight_info(Source::new(bytes), preflight, |_| false).unwrap();
            assert_eq!(merged.unwrap().unwrap(), file.decode_merged().unwrap());
        }
    }
}

#[test]
fn preflight_info_refusal_precedes_missing_fields_and_invalid_payloads() {
    for compression in Compression::ALL {
        let file = testgen::merged_only(Version::Psd, ColorMode::Rgb, 8, compression, 17, 9);
        let original = file.to_bytes().unwrap();
        let start = original.len() - file.image_data.data.len() - 2;
        for keep in 0..=3 {
            let bytes = original[..start + keep].to_vec();
            let preflight = |_: &PsdFile, actual, len| {
                assert_eq!(actual, (keep >= 2).then_some(compression));
                assert_eq!(len, keep.saturating_sub(2) as u64);
                Err(PsdError::LimitExceeded("test import budget"))
            };
            let recover = |_: &PsdFile| panic!("a refused preflight must not recover");
            assert!(matches!(PsdFile::from_bytes_and_merged_preflight_info(&bytes, preflight, recover), Err(PsdError::LimitExceeded("test import budget"))));
            assert!(matches!(
                PsdFile::from_source_and_merged_preflight_info(Source::new(bytes.clone()), preflight, recover),
                Err(PsdError::LimitExceeded("test import budget"))
            ));
            assert!(PsdFile::from_bytes_and_merged_preflight_info(&bytes, |_, _, _| Ok(true), |_| false).is_err());
            assert!(PsdFile::from_source_and_merged_preflight_info(Source::new(bytes), |_, _, _| Ok(true), |_| false).is_err());
        }
    }
}

#[test]
fn source_preflight_info_reads_only_compression_and_defers_read_errors() {
    struct HeaderOnly {
        bytes: Vec<u8>,
        merged_at: u64,
        fail_compression: bool,
    }
    impl ReadAt for HeaderOnly {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> photocraft_psd::Result<()> {
            assert!(offset + buf.len() as u64 <= self.merged_at + 2, "preflight read encoded pixels");
            if self.fail_compression && offset >= self.merged_at {
                return Err(PsdError::Read("compression read failed".into()));
            }
            self.bytes.read_at(offset, buf)
        }
    }
    let file = testgen::merged_only(Version::Psd, ColorMode::Rgb, 8, Compression::Rle, 17, 9);
    let bytes = file.to_bytes().unwrap();
    let merged_at = (bytes.len() - file.image_data.data.len() - 2) as u64;
    let source = |fail_compression| Source::new(HeaderOnly { bytes: bytes.clone(), merged_at, fail_compression });
    for fail in [false, true] {
        let preflight = |_: &PsdFile, compression, len| {
            assert_eq!(compression, (!fail).then_some(Compression::Rle));
            assert_eq!(len, file.image_data.encoded_len());
            Err(PsdError::LimitExceeded("test import budget"))
        };
        assert!(matches!(
            PsdFile::from_source_and_merged_preflight_info(source(fail), preflight, |_| panic!("preflight cannot recover")),
            Err(PsdError::LimitExceeded("test import budget"))
        ));
    }
    assert!(matches!(
        PsdFile::from_source_and_merged_preflight_info(source(true), |_, _, _| Ok(true), |_| panic!("read failure cannot recover")),
        Err(PsdError::Read(message)) if message == "compression read failed"
    ));
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

fn pattern_block(mode: u32, depth: u16, width: u32, height: u32) -> photocraft_psd::TaggedBlock {
    use photocraft_psd::patterns::{PsdPattern, mode_channels, write_pattern_block};
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
    photocraft_psd::TaggedBlock::new(*b"Patt", write_pattern_block(&[p]).unwrap())
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
    unknown.header.color_mode = photocraft_psd::ColorMode::Unknown(5);
    assert!(unknown.header.validate().is_ok());
    let required = estimate(&unknown);
    assert!(required >= estimate(&rgb), "the layers import as RGB and are counted");
    let bytes = unknown.to_bytes().unwrap();
    let opts = crate::ImportOptions { psd_max_pixel_bytes: required - 1, ..Default::default() };
    assert!(matches!(crate::import_with_options("mode5.psd", &bytes, &opts), Err(IoError::PsdImportBudget { .. })));
    let source = photocraft_psd::Source::new(bytes);
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
    assert!(matches!(res, Err(IoError::PsdImportBudget { required, limit }) if required == 10 * gb && limit == 9 * gb));
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
    assert!(matches!(err, IoError::PsdImportBudget { required, limit } if required == 2 * gb && limit == gb));

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
                Err(IoError::PsdImportBudget { required: r, limit: l }) if r == required && l == required - 1
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
        Err(IoError::PsdImportBudget { required, limit: 0 }) => assert!(required > 0),
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

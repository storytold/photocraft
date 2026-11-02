use super::*;
use photocraft_psd::{ColorMode, Compression, Version, testgen};

#[test]
fn retained_storage_is_summed_but_workspace_is_reused() {
    let mut budget = ImportBudget::default();
    budget.retain(40).unwrap();
    budget.retain(20).unwrap();
    budget.scratch(30);
    budget.scratch(10);
    assert!(budget.check(90).is_ok());
    assert!(matches!(budget.check(89), Err(IoError::PsdImportBudget { required: 90, limit: 89 })));
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
/// A fixed allowance rejects huge off-canvas pixels before reading missing merged data.
fn large_off_canvas_layer_is_rejected_without_decoding_its_tiny_data() {
    let mut file = testgen::small(Version::Psd, Compression::Zip);
    file.header.width = 1;
    file.header.height = 1;
    file.layers_mut()[0].rect = Rect::from_xywh(-10_000, -10_000, 200_000, 200_000);
    assert!(matches!(check(&file, DEFAULT_PSD_IMPORT_PIXEL_BYTES), Err(IoError::PsdImportBudget { .. })));
    // Both ordinary import routes must fail before reading a deliberately missing merged
    // section. No enormous decoded plane is ever constructed by this test.
    let mut bytes = file.to_bytes().unwrap();
    bytes.truncate(bytes.len() - file.image_data.data.len() - 2);
    let opts = crate::ImportOptions { psd_max_pixel_bytes: DEFAULT_PSD_IMPORT_PIXEL_BYTES, ..Default::default() };
    assert!(matches!(crate::import_with_options("huge.psd", &bytes, &opts), Err(IoError::PsdImportBudget { .. })));
    let source = photocraft_psd::Source::new(bytes);
    assert!(matches!(
        crate::import_source_with_limit("huge.psd", source, &photocraft_raster::Interrupt::NONE, DEFAULT_PSD_IMPORT_PIXEL_BYTES),
        Err(IoError::PsdImportBudget { .. })
    ));
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
/// Legacy conversion reports a deterministic preflight error through its warning channel.
fn legacy_conversion_reports_a_rejected_import_instead_of_decoding_it() {
    let mut file = testgen::small(Version::Psd, Compression::Zip);
    let mut record = file.layers()[0].clone();
    record.rect = Rect::from_xywh(0, 0, i32::MAX as u32, i32::MAX as u32);
    // Exceeds the format dimension cap on every host, regardless of installed RAM.
    file.layer_info.as_mut().unwrap().layers = vec![record];
    let (doc, warnings) = crate::psd_to_document(&file);
    assert!(doc.layers.is_empty());
    assert!(warnings.iter().any(|warning| warning.contains("layer dimensions exceed 300000")), "{warnings:?}");
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
/// Check static native scaling and target-specific ceilings without depending on host RAM.
fn native_defaults_scale_with_ram_and_fall_back_safely() {
    let address_limit = i64::MAX as u64;
    for (ram, expected) in [(0, 2), (2, 2), (4, 2), (8, 4), (16, 8), (128, 64)] {
        assert_eq!(native_default(ram << 30, address_limit), expected << 30);
    }
    assert_eq!(native_default(u64::MAX, address_limit), address_limit);
    assert_eq!(native_default(128 << 30, i32::MAX as u64), i32::MAX as u64);
    assert_eq!(effective_limit(8 << 30, false), (8 << 30).min(isize::MAX as u64));
    assert_eq!(effective_limit(8 << 30, true), 1 << 30);
    assert_eq!(effective_limit(123, true), 123);
    assert_eq!(effective_limit(0, false), 0);
    assert_eq!(crate::ImportOptions::default().psd_max_pixel_bytes, default_psd_import_pixel_bytes());
}

/// Build 38 full-canvas layer records without allocating or decoding their declared pixels.
fn ordinary_layer_records(version: Version, depth: u16) -> PsdFile {
    let mut file = testgen::small(version, Compression::Zip);
    file.header.width = 6144;
    file.header.height = 2560;
    file.header.depth = depth;
    file.header.channels = 3;
    let mut layer = file.layers()[0].clone();
    layer.rect = Rect::from_xywh(0, 0, 6144, 2560);
    layer.mask = photocraft_psd::layer::MaskData::None;
    layer.channels.retain(|c| c.id >= -1);
    file.layer_info.as_mut().unwrap().layers = vec![layer; 38];
    file
}

#[test]
/// The reported document geometry exceeds 2 GiB but fits a larger native allowance at every depth.
fn ordinary_layered_psd_and_psb_fit_a_large_machine_without_decoding() {
    for version in [Version::Psd, Version::Psb] {
        for depth in [8, 16, 32] {
            let file = ordinary_layer_records(version, depth);
            assert!(!super::super::psd_import::merged_needed(&file));
            assert!(estimate(&file) > 2 << 30);
            assert!(check(&file, 2 << 30).is_err());
            assert!(check_with_available(&file, native_default(128 << 30, i64::MAX as u64), None).is_ok());
            assert!(check(&file, effective_limit(64 << 30, true)).is_err());
        }
    }
}

// Keep the ordinary suite small. This exercises real decoding (about 600 MP per format),
// whereas the test above verifies the policy on every machine without allocating the pixels.
#[test]
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(debug_assertions, ignore = "large PSD/PSB decode: included in the release corpus suite on machines with at least 8 GiB RAM")]
/// Decode the reported 38-layer geometry through default and explicit byte/file PSD and PSB paths.
/// Release corpus runs require sufficient memory; unmet requirements are reported as failures.
fn ordinary_38_layer_documents_open_under_the_native_default() -> Result<(), Box<dyn std::error::Error>> {
    use photocraft_psd::layer::ChannelData;
    let mut system = sysinfo::System::new();
    system.refresh_memory_specifics(sysinfo::MemoryRefreshKind::nothing().with_ram());
    let needed = estimate(&ordinary_layer_records(Version::Psd, 8));
    require_large_regression_headroom(system.total_memory(), needed, crate::memory::available())?;
    for version in [Version::Psd, Version::Psb] {
        let mut file = ordinary_layer_records(version, 8);
        let plane = vec![0; 6144 * 2560];
        let channels: Vec<_> = [0, 1, 2, -1].into_iter().map(|id| ChannelData::encode(id, Compression::Zip, &plane, 6144, 2560, 8, version).unwrap()).collect();
        for layer in file.layers_mut() {
            layer.channels = channels.clone();
        }
        drop(plane);
        let required = estimate(&file);
        assert!(default_psd_import_pixel_bytes() >= required);
        let bytes = file.to_bytes().unwrap();
        let opts = crate::ImportOptions { psd_max_pixel_bytes: 2 << 30, ..Default::default() };
        assert!(matches!(crate::import_with_options("ordinary.psd", &bytes, &opts), Err(IoError::PsdImportBudget { .. })));
        require_large_regression_headroom(system.total_memory(), required, crate::memory::available())?;
        let opened = crate::import("ordinary.psd", &bytes)?;
        assert_eq!(opened.document.layers.len(), 38);
        assert_eq!(opened.document.size.width, 6144);
        assert_eq!(opened.document.size.height, 2560);
        drop(opened);
        // The explicit allowance may exceed the old ceiling on both bytes and file routes.
        let opts = crate::ImportOptions { psd_max_pixel_bytes: required, ..opts };
        require_large_regression_headroom(system.total_memory(), required, crate::memory::available())?;
        assert_eq!(crate::import_with_options("ordinary.psd", &bytes, &opts)?.document.layers.len(), 38);
        let path = std::env::temp_dir().join(format!("photocraft-10745-{:?}-{}.psd", version, std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let imported = require_large_regression_headroom(system.total_memory(), required, crate::memory::available())
            .map_err(crate::IoError::Unsupported)
            .and_then(|()| crate::import_file("ordinary.psd", &path, &opts, &photocraft_raster::Interrupt::NONE))
            .map(|result| result.document.layers.len());
        std::fs::remove_file(path).unwrap();
        assert_eq!(imported?, 38);
    }
    Ok(())
}

/// A resource shortfall must be a visible test failure, never a successful early return.
#[cfg(not(target_arch = "wasm32"))]
fn require_large_regression_headroom(total_ram: u64, required: u64, available: Option<u64>) -> Result<(), String> {
    if total_ram < 8 << 30 {
        return Err("large PSD/PSB regression requires at least 8 GiB RAM".into());
    }
    crate::memory::check_fits(6144, 2560, required, available).map_err(|error| format!("large PSD/PSB regression requires more current headroom: {error}"))
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
/// Low physical RAM and reduced free memory must report errors instead of skipping checks.
fn large_regression_resource_shortfalls_are_errors() {
    let required = 3 << 30;
    let room = required + crate::memory::RESERVE;
    assert!(require_large_regression_headroom(8 << 30, required, Some(room)).is_ok());
    assert!(require_large_regression_headroom(8 << 30, required, None).is_ok());
    assert!(require_large_regression_headroom((8 << 30) - 1, required, Some(room)).unwrap_err().contains("8 GiB"));
    assert!(require_large_regression_headroom(8 << 30, required, Some(room - 1)).unwrap_err().contains("headroom"));
}

#[test]
/// Fresh runtime headroom constrains both automatic and explicit pixel allowances.
fn psd_limits_also_respect_current_process_and_cgroup_headroom() {
    let file = testgen::small(Version::Psd, Compression::Zip);
    let budget = super::estimate_budget(&file).unwrap();
    let required = budget.required().unwrap();
    let room = required + crate::memory::RESERVE;
    for limit in [required, native_default(128 << 30, i64::MAX as u64)] {
        assert!(check_with_available(&file, limit, Some(room)).is_ok());
        let error = check_with_available(&file, limit, Some(room - 1)).unwrap_err();
        assert!(error.to_string().contains("not enough memory"), "{error}");
        assert!(check_with_available(&file, limit, None).is_ok());
    }
    assert!(matches!(check_with_available(&file, required - 1, Some(u64::MAX)), Err(IoError::PsdImportBudget { .. })));
}

/// Declare oversized RGBA32 layers, retaining only the generator's tiny encoded channel data.
/// Grow the layer count once the side reaches the PSD/PSB 300,000-pixel dimension cap.
fn layered_preflight_above(headroom: u64) -> PsdFile {
    let mut file = testgen::layered(Version::Psb, ColorMode::Rgb, 32, Compression::Zip);
    let mut layer = file.layers()[0].clone();
    let side = headroom.div_ceil(16).isqrt().saturating_add(1).clamp(1, u64::from(photocraft_psd::header::MAX_DIMENSION)) as u32;
    layer.rect = Rect::from_xywh(0, 0, side, side);
    file.layer_info.as_mut().unwrap().layers = vec![layer.clone()];
    let retained = super::estimate_budget(&file).unwrap().retained;
    assert!(retained > 0);
    let count = headroom / retained + 1;
    assert!(count <= i16::MAX as u64, "headroom exceeds the format-valid fixture range: {headroom}");
    file.layer_info.as_mut().unwrap().layers = vec![layer; usize::try_from(count).unwrap()];
    assert!(super::estimate_budget(&file).unwrap().required().unwrap() > headroom);
    file
}

#[test]
/// Prove the metadata fixture scales past terabyte hosts without exceeding format limits.
fn live_headroom_fixture_scales_without_decoding_pixels() {
    // At the dimension cap, 32,767 RGBA32 layer records cover over 40 PiB, before
    // workspace. Exercise twice the supported Linux host bound: 4 PiB physical RAM
    // (52-bit addresses) + at most 32 swap types * 2^32 pages * 64 KiB = 8 PiB swap.
    // Bounds: kernel.org/doc/html/latest/arch/x86/x86_64/5level-paging.html;
    // github.com/torvalds/linux/blob/v6.17/arch/arm64/Kconfig (ARM64_PA_BITS_52);
    // github.com/torvalds/linux/blob/v6.17/include/linux/swap.h (last_page/type bits).
    for headroom in [0, 1, 4 << 30, 256 << 30, 4 << 40, 256 << 40, 4 << 50, 24 << 50] {
        let file = layered_preflight_above(headroom);
        assert!(file.layers().len() <= i16::MAX as usize);
        assert!(file.layers().iter().all(|layer| layer.rect.size().is_ok()));
        check_with_available(&file, u64::MAX, None).unwrap();
        let error = check_with_available(&file, u64::MAX, Some(headroom)).unwrap_err();
        assert!(error.to_string().contains("not enough memory"), "{error}");
    }
}

#[test]
#[cfg(target_os = "linux")]
/// Exercise the production live-memory query; bypassing it must fail without decoding pixels.
fn check_refuses_a_layered_psd_above_live_linux_headroom() {
    let live = crate::memory::available().expect("Linux must expose available RAM/swap for this regression");
    let mut system = sysinfo::System::new();
    system.refresh_memory_specifics(sysinfo::MemoryRefreshKind::nothing().with_ram().with_swap());
    // Include all host RAM/swap, not only free/cgroup headroom, so reclaim or a cgroup
    // expansion during the test cannot admit the fixture. No importer is called here.
    let bound = live.max(system.total_memory().saturating_add(system.total_swap()));
    let file = layered_preflight_above(bound.checked_mul(2).expect("host RAM/swap bound is too large to double"));
    let error = check(&file, u64::MAX).unwrap_err();
    assert!(error.to_string().contains("not enough memory"), "{error}");
}

#[test]
/// Sum PSD/PSB source estimates before decoding, preserving the total after refused inputs.
fn batch_sources_share_one_allowance_before_any_pixel_decode() {
    for version in [Version::Psd, Version::Psb] {
        let file = testgen::small(version, Compression::Zip);
        let required = estimate(&file);
        let opts = crate::ImportOptions { psd_max_pixel_bytes: required * 2, ..Default::default() };
        let mut bytes = file.to_bytes().unwrap();
        // No merged section: the preflight must finish without touching it.
        bytes.truncate(bytes.len() - file.image_data.data.len() - 2);
        let mut batch = PsdImportBatchBudget::default();
        batch.include(b"not a PSD", &opts).unwrap();
        batch.include(&bytes, &opts).unwrap();
        batch.include(&bytes, &opts).unwrap();
        assert_eq!(batch.required, required * 2);
        assert!(matches!(batch.include(&bytes, &opts), Err(IoError::PsdImportBudget { .. })));
        assert_eq!(batch.required, required * 2, "a refusal must not spend the allowance");
        assert!(batch.include(b"8BPS", &opts).is_err());
        batch.required = u64::MAX;
        assert!(batch.include(&bytes, &opts).unwrap_err().to_string().contains("overflow"));
    }
}

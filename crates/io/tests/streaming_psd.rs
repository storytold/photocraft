#![cfg(not(target_arch = "wasm32"))]
use photocraft_psd::{ColorMode, Compression, Version};
use photocraft_raster::Interrupt;

// Native imports and exports reserve the same process-wide working-memory allowance.
// Keep fidelity fixtures from rejecting each other solely because the test runner is parallel.
static WORKING_MEMORY_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn seekable_import_matches_byte_import_pixels_masks_and_metadata() {
    let _test = WORKING_MEMORY_TESTS.lock().unwrap();
    let path = std::env::temp_dir().join(format!("photocraft-stream-parity-{}.psb", std::process::id()));
    for version in [Version::Psd, Version::Psb] {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            for depth in [8, 16, 32] {
                for compression in Compression::ALL {
                    let mut file = photocraft_psd::testgen::layered(version, mode, depth, compression);
                    // The streaming layer path need not consume the stored merged image.
                    file.header.channels = mode.color_channels().unwrap() + u16::from(file.merged_has_alpha());
                    let bytes = file.to_bytes().unwrap();
                    std::fs::write(&path, &bytes).unwrap();
                    let a = photocraft_io::import("d", &bytes).unwrap();
                    let b = photocraft_io::import_path_with("d", &path, &Interrupt::NONE).unwrap();
                    let authorized = photocraft_io::import_seekable_with("d", &|| std::fs::File::open(&path), &Interrupt::NONE).unwrap();
                    assert_eq!(
                        photocraft_compose::render(&authorized.document, authorized.document.bounds()).px,
                        photocraft_compose::render(&b.document, b.document.bounds()).px
                    );
                    assert_eq!(a.warnings, b.warnings, "{version:?}/{mode:?}/{depth}/{compression:?}");
                    let aa = photocraft_compose::render(&a.document, a.document.bounds());
                    let bb = photocraft_compose::render(&b.document, b.document.bounds());
                    assert_eq!(aa.px, bb.px, "{version:?}/{mode:?}/{depth}/{compression:?}");
                    let options = photocraft_io::PsdExportOptions::default();
                    let (a, _) = photocraft_io::document_to_psd_with(&a.document, &options);
                    let (b, _) = photocraft_io::document_to_psd_with(&b.document, &options);
                    assert_eq!(a, b, "layer, mask and unknown-block round trip: {version:?}/{mode:?}/{depth}/{compression:?}");
                }
            }
        }
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn cancelled_and_truncated_imports_publish_no_document() {
    let _test = WORKING_MEMORY_TESTS.lock().unwrap();
    let path = std::env::temp_dir().join(format!("photocraft-stream-cancel-{}.psb", std::process::id()));
    let mut file = photocraft_psd::testgen::layered(Version::Psb, ColorMode::Rgb, 8, Compression::Zip);
    file.header.channels = 3 + u16::from(file.merged_has_alpha());
    let bytes = file.to_bytes().unwrap();
    std::fs::write(&path, &bytes).unwrap();
    assert!(matches!(photocraft_io::import_path_with("d", &path, &Interrupt::cancel_only(&|| true)), Err(photocraft_io::IoError::Cancelled)));
    std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
    assert!(photocraft_io::import_path_with("d", &path, &Interrupt::NONE).is_err());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn seekable_export_preserves_merged_layers_masks_and_unknown_metadata() {
    let _test = WORKING_MEMORY_TESTS.lock().unwrap();
    let path = std::env::temp_dir().join(format!("photocraft-export-parity-{}.psb", std::process::id()));
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [8, 16, 32] {
            let input = photocraft_psd::testgen::layered(Version::Psb, mode, depth, Compression::ZipPrediction);
            let doc = photocraft_io::import("input", &input.to_bytes().unwrap()).unwrap().document;
            let (reference, _) = photocraft_io::document_to_psd_with(&doc, &photocraft_io::PsdExportOptions { force_psb: true, ..Default::default() });
            photocraft_io::export_path_with(&doc, &path, &Default::default(), &Interrupt::NONE).unwrap();
            let output = photocraft_psd::PsdFile::from_bytes(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(output.header, reference.header, "{mode:?}/{depth}");
            assert_eq!(output.resources, reference.resources);
            let normalized_reference = photocraft_psd::PsdFile::from_bytes(&reference.to_bytes().unwrap()).unwrap();
            assert_eq!(output.global_blocks, normalized_reference.global_blocks);
            assert_eq!(output.image_data.decode(&output.header).unwrap(), reference.image_data.decode(&reference.header).unwrap(), "merged {mode:?}/{depth}");
            let loaded = photocraft_io::import_path_with("output", &path, &Interrupt::NONE).unwrap();
            let (loaded, _) = photocraft_io::document_to_psd_with(&loaded.document, &photocraft_io::PsdExportOptions { force_psb: true, ..Default::default() });
            assert_eq!(loaded.to_bytes().unwrap(), reference.to_bytes().unwrap(), "layer/mask/metadata {mode:?}/{depth}");
        }
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn cancelled_stream_save_keeps_destination_and_removes_temporary_output() {
    let _test = WORKING_MEMORY_TESTS.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("photocraft-export-cancel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("kept.psb");
    std::fs::write(&path, b"precious original").unwrap();
    let input = photocraft_psd::testgen::layered(Version::Psb, ColorMode::Rgb, 8, Compression::Raw);
    let doc = photocraft_io::import("input", &input.to_bytes().unwrap()).unwrap().document;
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 5;
    assert!(matches!(
        photocraft_io::export_path_with(&doc, &path, &Default::default(), &Interrupt::cancel_only(&cancel)),
        Err(photocraft_io::IoError::Cancelled)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"precious original");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "corpus")]
#[test]
fn native_path_import_matches_byte_import_across_real_psd_corpus() {
    let _test = WORKING_MEMORY_TESTS.lock().unwrap();
    fn collect(p: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(p).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                collect(&p, out);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("psd") || x.eq_ignore_ascii_case("psb")) {
                out.push(p);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/psd");
    assert!(root.is_dir(), "run cargo xtask corpus --all");
    let mut files = Vec::new();
    collect(&root, &mut files);
    let mut compared = 0;
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        let Ok(a) = photocraft_io::import("d", &bytes) else { continue };
        let b = photocraft_io::import_path_with("d", &path, &Interrupt::NONE).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(a.warnings, b.warnings, "{}", path.display());
        assert_eq!(
            photocraft_compose::render(&a.document, a.document.bounds()).px,
            photocraft_compose::render(&b.document, b.document.bounds()).px,
            "{}",
            path.display()
        );
        compared += 1;
    }
    assert!(compared >= 20, "{compared} corpus files compared");
    eprintln!("seekable corpus parity: {compared} files");
}

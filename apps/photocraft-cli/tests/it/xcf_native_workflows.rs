//! Process-level XCF transport, native editability and safe export controls.
//! Fixtures are original public-spec source bytes and independently stored wrappers.
use std::path::Path;
use std::process::{Command, Output};

fn cli() -> Command {
    let mut command = crate::command();
    for name in ["PHOTOCRAFT_CONTROL_PORT", "PHOTOCRAFT_CONTROL_TOKEN", "PHOTOCRAFT_CONTROL_TOKEN_FILE", "PHOTOCRAFT_GPU"] {
        command.env_remove(name);
    }
    command
}
fn success(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "status {:?}: {} {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn assert_native(path: &Path, opacity: f32) {
    let bytes = std::fs::read(path).unwrap();
    let document = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(document.layers.len(), 1);
    assert_eq!(document.layers[0].name, "Pixel");
    assert_eq!(document.layers[0].opacity, opacity, "opacity remains a native editable property");
    assert_eq!(document.layers[0].surface().unwrap().read_region(document.bounds()), [128. / 255., 64. / 255., 32. / 255., 1.]);
}

/// One ordinary RGBA layer, with uncompressed tile bytes and no private corpus input.
fn layered_xcf() -> Vec<u8> {
    /// Append a big-endian v3 fixture field without relying on the importer or a corpus file.
    fn word(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_be_bytes());
    }
    /// Resolve a reserved 32-bit fixture pointer to its independently written section.
    fn patch(out: &mut [u8], at: usize, value: usize) {
        out[at..at + 4].copy_from_slice(&(value as u32).to_be_bytes());
    }
    let mut out = b"gimp xcf v003\0".to_vec();
    for value in [1, 1, 0, 17, 1] {
        word(&mut out, value);
    }
    out.push(0); // Image compression property: raw tiles.
    word(&mut out, 0);
    word(&mut out, 0);
    let layer_pointer = out.len();
    for _ in 0..3 {
        word(&mut out, 0); // Layer pointer, end of layers, end of channels.
    }
    let layer = out.len();
    for value in [1, 1, 1, 6] {
        word(&mut out, value);
    }
    out.extend_from_slice(b"Pixel\0");
    word(&mut out, 0);
    word(&mut out, 0);
    let hierarchy_pointer = out.len();
    word(&mut out, 0);
    word(&mut out, 0);
    let hierarchy = out.len();
    for value in [1, 1, 4] {
        word(&mut out, value);
    }
    let level_pointer = out.len();
    word(&mut out, 0);
    word(&mut out, 0);
    let level = out.len();
    word(&mut out, 1);
    word(&mut out, 1);
    let tile_pointer = out.len();
    word(&mut out, 0);
    word(&mut out, 0);
    let tile = out.len();
    out.extend_from_slice(&[128, 64, 32, 255]);
    patch(&mut out, layer_pointer, layer);
    patch(&mut out, hierarchy_pointer, hierarchy);
    patch(&mut out, level_pointer, level);
    patch(&mut out, tile_pointer, tile);
    out
}

/// Compute the standard wrapper checksum over original fixture bytes for decoder-independent transport tests.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

/// Wrap a small original fixture in one stored DEFLATE block with independent gzip length and CRC fields.
fn stored_gzip(bytes: &[u8]) -> Vec<u8> {
    assert!(bytes.len() <= u16::MAX as usize);
    let len = bytes.len() as u16;
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255, 1];
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(bytes);
    out.extend_from_slice(&crc32(bytes).to_le_bytes());
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out
}

/// Single uncompressed LZMA2 chunk and CRC32; independent of decoder encoder features.
fn stored_xz(bytes: &[u8]) -> Vec<u8> {
    /// Encode an XZ index integer in base-128 form for the original stored-chunk wrapper.
    fn vli(mut value: usize, out: &mut Vec<u8>) {
        loop {
            let byte = (value & 127) as u8;
            value >>= 7;
            out.push(byte | if value != 0 { 128 } else { 0 });
            if value == 0 {
                break;
            }
        }
    }
    assert!(!bytes.is_empty() && bytes.len() <= 65536);
    let mut out = b"\xfd7zXZ\0\0\x01".to_vec();
    out.extend_from_slice(&crc32(&[0, 1]).to_le_bytes());
    let mut header = vec![2, 0, 0x21, 1, 0, 0, 0, 0];
    header.extend_from_slice(&crc32(&header).to_le_bytes());
    out.extend_from_slice(&header);
    let mut data = vec![1];
    data.extend_from_slice(&((bytes.len() - 1) as u16).to_be_bytes());
    data.extend_from_slice(bytes);
    data.push(0);
    out.extend_from_slice(&data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    out.extend_from_slice(&crc32(bytes).to_le_bytes());
    let mut index = vec![0, 1];
    vli(header.len() + data.len() + 4, &mut index);
    vli(bytes.len(), &mut index);
    while !index.len().is_multiple_of(4) {
        index.push(0);
    }
    index.extend_from_slice(&crc32(&index).to_le_bytes());
    out.extend_from_slice(&index);
    let mut footer = ((index.len() / 4 - 1) as u32).to_le_bytes().to_vec();
    footer.extend_from_slice(&[0, 1]);
    out.extend_from_slice(&crc32(&footer).to_le_bytes());
    out.extend_from_slice(&footer);
    out.extend_from_slice(b"YZ");
    out
}

/// Return identical original XCF pixels in plain, gzip, bzip2 and XZ transports.
fn wrapped_sources() -> Vec<(&'static str, Vec<u8>)> {
    let source = layered_xcf();
    // Python standard-library bz2.compress(layered_xcf()) output, solely original synthetic
    // bytes defined above. A literal avoids adding an encoder dependency to the CLI crate.
    let bzip = vec![
        66, 90, 104, 57, 49, 65, 89, 38, 83, 89, 246, 62, 85, 39, 0, 0, 48, 127, 192, 237, 192, 32, 0, 64, 0, 72, 128, 64, 0, 64, 2, 11, 166, 97, 64, 96, 0, 0,
        0, 160, 0, 84, 68, 104, 153, 26, 24, 153, 48, 98, 12, 42, 52, 26, 1, 144, 104, 17, 245, 233, 72, 102, 185, 131, 201, 176, 17, 65, 24, 166, 137, 32, 99,
        131, 72, 144, 183, 164, 187, 152, 151, 179, 102, 24, 66, 209, 26, 124, 128, 193, 1, 216, 187, 146, 41, 194, 132, 135, 177, 242, 169, 56,
    ];
    vec![("plain", source.clone()), ("gzip", stored_gzip(&source)), ("bzip2", bzip), ("xz", stored_xz(&source))]
}

#[test]
fn convert_plain_gzip_bzip2_and_xz_into_equivalent_native_layers() {
    let directory = tempfile::tempdir().unwrap();
    for (kind, bytes) in wrapped_sources() {
        // Deliberately generic names exercise wrapper magic without changing
        // the longstanding positional convert arguments or --format override.
        let input = directory.path().join(format!("{kind}.data"));
        let output = directory.path().join(format!("{kind}.pcraft"));
        std::fs::write(&input, &bytes).unwrap();
        success(cli().arg("convert").arg(&input).arg(&output).args(["--format", "pcraft"]));
        assert_native(&output, 1.);
        assert_eq!(std::fs::read(input).unwrap(), bytes, "conversion never rewrites its source");
    }
}

#[test]
fn batch_explicit_native_and_flat_formats_keep_wrapped_source_pixels_and_editability() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input");
    std::fs::create_dir(&input).unwrap();
    let names = ["plain.XCF", "gzip.xcf.gz", "bzip.xcf.bz2", "xz.xcf.xz"];
    let sources = wrapped_sources();
    for ((_, bytes), name) in sources.iter().zip(names) {
        std::fs::write(input.join(name), bytes).unwrap();
    }
    let actions = directory.path().join("actions.json");
    // The original pair form remains accepted alongside current action objects.
    std::fs::write(&actions, br#"[["layer.setProps",{"opacity":0.5}]]"#).unwrap();
    for (route, format) in [("native", ".pcraft"), ("flat", ".png")] {
        let output = directory.path().join(route);
        let result = success(cli().args(["batch", "--actions"]).arg(&actions).arg("--in").arg(&input).arg("--out").arg(&output).args(["--format", format]));
        assert!(String::from_utf8_lossy(&result.stdout).contains("4 succeeded, 0 failed"));
        let files = std::fs::read_dir(&output).unwrap().map(|entry| entry.unwrap().path()).collect::<Vec<_>>();
        assert_eq!(files.len(), 4);
        for path in files {
            if route == "native" {
                assert_native(&path, 0.5);
            } else {
                assert_eq!(path.extension().unwrap(), "png");
                let decoded = photocraft_io::import("export.png", &std::fs::read(path).unwrap()).unwrap();
                let samples = decoded.document.layers[0].surface().unwrap().read_region(decoded.document.bounds());
                for (sample, expected) in samples.iter().zip([128. / 255., 64. / 255., 32. / 255., 0.5]) {
                    assert!((sample - expected).abs() <= 1. / 255.);
                }
            }
        }
        for ((_, bytes), name) in sources.iter().zip(names) {
            assert_eq!(std::fs::read(input.join(name)).unwrap(), *bytes);
        }
    }
}

#[test]
fn convert_without_in_place_rejects_original_and_hardlink_alias_outputs() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.xcf");
    let alias = directory.path().join("alias.pcraft");
    let bytes = layered_xcf();
    std::fs::write(&source, &bytes).unwrap();
    std::fs::hard_link(&source, &alias).unwrap();
    for output in [&source, &alias] {
        let result = cli().arg("convert").arg(&source).arg(output).args(["--format", "pcraft"]).output().unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("input file"));
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        assert_eq!(std::fs::read(&alias).unwrap(), bytes);
    }
}

#[test]
fn convert_in_place_cannot_replace_an_import_only_source_or_its_hardlink() {
    let directory = tempfile::tempdir().unwrap();
    for (kind, bytes) in wrapped_sources() {
        let source = directory.path().join(format!("{kind}.data"));
        let alias = directory.path().join(format!("{kind}-alias.pcraft"));
        std::fs::write(&source, &bytes).unwrap();
        std::fs::hard_link(&source, &alias).unwrap();
        for output in [&source, &alias] {
            let result = cli().arg("convert").arg(&source).arg(output).args(["--format", "pcraft", "--in-place"]).output().unwrap();
            assert!(!result.status.success());
            assert!(String::from_utf8_lossy(&result.stderr).contains("protected import-only source"), "{}", String::from_utf8_lossy(&result.stderr));
            assert_eq!(std::fs::read(&source).unwrap(), bytes);
            assert_eq!(std::fs::read(&alias).unwrap(), bytes);
        }
    }
}

#[test]
fn convert_in_place_still_rewrites_an_ordinary_writable_png() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.xcf");
    let png = directory.path().join("writable.png");
    std::fs::write(&source, layered_xcf()).unwrap();
    success(cli().arg("convert").arg(&source).arg(&png));
    let before = photocraft_io::import("writable.png", &std::fs::read(&png).unwrap()).unwrap().document;
    success(cli().arg("convert").arg(&png).arg(&png).arg("--in-place"));
    let after = photocraft_io::import("writable.png", &std::fs::read(&png).unwrap()).unwrap().document;
    assert_eq!(after.layers[0].surface().unwrap().read_region(after.bounds()), before.layers[0].surface().unwrap().read_region(before.bounds()));
    assert_eq!(std::fs::read(source).unwrap(), layered_xcf());
}

#[path = "../support/historical_xcf.rs"]
mod historical_fixture;

#[test]
fn info_and_run_require_explicit_historical_order_and_preserve_each_choice() {
    let folder = tempfile::tempdir().unwrap();
    let input = folder.path().join("historical.xcf");
    let output = folder.path().join("run.pcraft");
    let bytes = historical_fixture::rgb16(10, false);
    std::fs::write(&input, &bytes).unwrap();
    for command in ["info", "run"] {
        let mut invocation = cli();
        invocation.arg(command).arg(&input);
        if command == "run" {
            invocation.arg("--out").arg(&output);
        }
        let result = invocation.output().unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("choose little-endian or big-endian"));
        assert!(!output.exists());
    }
    for (order, expected) in [("little-endian", [0x3412u16, 0x7856, 0xbc9a]), ("big-endian", [0x1234u16, 0x5678, 0x9abc])] {
        let information = success(cli().arg("info").arg(&input).args(["--compact", "--detail", "--historical-xcf-sample-order", order]));
        let information: serde_json::Value = serde_json::from_slice(&information.stdout).unwrap();
        assert_eq!(information["width"], 1);
        assert_eq!(information["height"], 1);
        assert!(information["warnings"].is_array());
        success(
            cli()
                .arg("run")
                .arg(&input)
                .args(["--cmd", "layer.setProps", "--params", r#"{"opacity":0.5}"#, "--historical-xcf-sample-order", order, "--out"])
                .arg(&output),
        );
        let doc = photocraft_format::load_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(doc.layers[0].opacity, 0.5);
        assert_eq!(
            doc.layers[0].surface().unwrap().pixel(0, 0),
            [f32::from(expected[0]) / 65535., f32::from(expected[1]) / 65535., f32::from(expected[2]) / 65535., 1.]
        );
        assert_eq!(std::fs::read(&input).unwrap(), bytes);
    }
}

#[test]
fn info_and_run_keep_modern_inputs_compatible_without_the_new_order_option() {
    let folder = tempfile::tempdir().unwrap();
    let input = folder.path().join("modern.xcf");
    let output = folder.path().join("native.pcraft");
    let original = historical_fixture::rgb16(12, false);
    std::fs::write(&input, &original).unwrap();
    let information = success(cli().arg("info").arg(&input).arg("--compact"));
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&information.stdout).unwrap()["width"], 1);
    success(cli().arg("run").arg(&input).arg("--out").arg(&output));
    let document = photocraft_format::load_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(document.layers[0].surface().unwrap().pixel(0, 0), [0x1234u16, 0x5678, 0x9abc, 0xffff].map(|value| f32::from(value) / 65535.));
    for command in ["info", "run"] {
        let mut invocation = cli();
        invocation.arg(command).arg("does-not-exist.xcf").args(["--historical-xcf-sample-order", "native"]);
        if command == "run" {
            invocation.arg("--out").arg(&output);
        }
        let result = invocation.output().unwrap();
        assert_eq!(result.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&result.stderr).contains("--historical-xcf-sample-order"));
    }
    let rejected =
        cli().args(["run", "--new", r#"{"width":1,"height":1}"#, "--historical-xcf-sample-order", "little-endian", "--out"]).arg(&output).output().unwrap();
    assert_eq!(rejected.status.code(), Some(2), "source-specific order must not pretend to govern --new");
    assert_eq!(std::fs::read(input).unwrap(), original);
}

#[test]
fn run_in_place_rejects_import_only_original_and_alias_before_any_command() {
    let folder = tempfile::tempdir().unwrap();
    for (kind, bytes) in wrapped_sources() {
        let input = folder.path().join(format!("{kind}.data"));
        let alias = folder.path().join(format!("{kind}-alias.pcraft"));
        std::fs::write(&input, &bytes).unwrap();
        std::fs::hard_link(&input, &alias).unwrap();
        for output in [&input, &alias] {
            let result = cli()
                .arg("run")
                .arg(&input)
                .args(["--cmd", "this-command-must-not-run", "--in-place", "--format", "pcraft", "--out"])
                .arg(output)
                .output()
                .unwrap();
            assert!(!result.status.success());
            assert!(String::from_utf8_lossy(&result.stderr).contains("protected import-only source"), "{}", String::from_utf8_lossy(&result.stderr));
            assert!(result.stdout.is_empty(), "no command result is emitted before the refusal");
            assert_eq!(std::fs::read(&input).unwrap(), bytes);
            assert_eq!(std::fs::read(&alias).unwrap(), bytes);
        }
    }
}

#[test]
fn run_in_place_still_edits_and_exports_an_ordinary_writable_png() {
    let folder = tempfile::tempdir().unwrap();
    let input = folder.path().join("source.xcf");
    let png = folder.path().join("writable.png");
    std::fs::write(&input, layered_xcf()).unwrap();
    success(cli().arg("convert").arg(&input).arg(&png));
    success(cli().arg("run").arg(&png).args(["--cmd", "layer.setProps", "--params", r#"{"opacity":0.5}"#, "--in-place", "--out"]).arg(&png));
    let doc = photocraft_io::import("writable.png", &std::fs::read(png).unwrap()).unwrap().document;
    let samples = doc.layers[0].surface().unwrap().read_region(doc.bounds());
    for (sample, expected) in samples.iter().zip([128. / 255., 64. / 255., 32. / 255., 0.5]) {
        assert!((sample - expected).abs() <= 1. / 255.);
    }
    assert_eq!(std::fs::read(input).unwrap(), layered_xcf());
}

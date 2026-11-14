use crate::test_support::invoke;

#[test]
fn batch_wrapped_historical_inputs_exclude_unrelated_archives_in_both_listing_modes() {
    let folder = tempfile::tempdir().unwrap();
    let input = folder.path().join("in");
    let actions = folder.path().join("actions.json");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(&actions, "[]").unwrap();
    let sources = [("gzip.xcf.gz", GZIP), ("bzip.xcf.bz2", BZIP), ("xz.XCF.XZ", XZ), ("alias.xcfgz", GZIP)];
    for (name, bytes) in sources {
        std::fs::write(input.join(name), bytes).unwrap();
        assert!(super::is_input(&input.join(name)), "{name}");
    }
    for name in ["archive.tar.gz", "notes.gz", "notes.bz2", "notes.xz", "pixel.xcf.gz.backup"] {
        std::fs::write(input.join(name), b"not an image").unwrap();
        assert!(!super::is_input(&input.join(name)), "{name}");
    }
    for recursive in [false, true] {
        let output = folder.path().join(format!("out-{recursive}"));
        let mut args = vec![
            "batch",
            "--actions",
            actions.to_str().unwrap(),
            "--in",
            input.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
            "--format",
            "pcraft",
            "--no-gpu",
            "--historical-xcf-sample-order",
            "little-endian",
        ];
        if recursive {
            args.push("--recursive");
        }
        let result = invoke(&args);
        assert_eq!(result.code, 0, "{args:?}: {}{}", result.stdout, result.stderr);
        assert!(result.stdout.contains("4 succeeded, 0 failed"), "{}", result.stdout);
        assert!(!result.stderr.contains("FAIL"), "{}", result.stderr);
        assert_eq!(std::fs::read_dir(&output).unwrap().count(), 4);
        for (name, bytes) in sources {
            let stem = std::path::Path::new(name).file_stem().unwrap().to_string_lossy();
            let target = output.join(format!("{stem}.pcraft"));
            let native = std::fs::read(&target).unwrap();
            let doc = photocraft_format::load_from_bytes(&native).unwrap();
            assert_eq!(doc.layers.len(), 1);
            let pixels = doc.layers[0].surface().unwrap().read_region(photocraft_doc::Rect::new(0, 0, 1, 1));
            assert_eq!(pixels.len(), 4);
            for (actual, expected) in pixels.iter().zip([0x1234u16, 0x5678, 0x9abc, 0xffff]) {
                assert!((*actual - f32::from(expected) / 65535.).abs() <= 1. / 65535., "{name}: {pixels:?}");
            }
            assert_eq!(std::fs::read(input.join(name)).unwrap(), bytes, "batch must preserve wrapped source bytes");
        }
    }
}

/// Independently authored one-pixel RGB16 XCF, with versioned pointer widths and
/// explicitly encoded sample bytes rather than a blank high-depth header.
fn historical_rgb16(version: u32, little: bool) -> Vec<u8> {
    struct Wire {
        bytes: Vec<u8>,
        wide: bool,
    }
    impl Wire {
        fn word(&mut self, value: u32) {
            self.bytes.extend_from_slice(&value.to_be_bytes());
        }
        fn pointer(&mut self, value: usize) {
            if self.wide {
                self.bytes.extend_from_slice(&(value as u64).to_be_bytes());
            } else {
                self.word(value as u32);
            }
        }
        fn slot(&mut self) -> usize {
            let slot = self.bytes.len();
            self.pointer(0);
            slot
        }
        fn patch(&mut self, slot: usize) {
            let offset = self.bytes.len();
            if self.wide {
                self.bytes[slot..slot + 8].copy_from_slice(&(offset as u64).to_be_bytes());
            } else {
                self.bytes[slot..slot + 4].copy_from_slice(&(offset as u32).to_be_bytes());
            }
        }
    }
    let mut wire = Wire { bytes: format!("gimp xcf v{version:03}\0").into_bytes(), wide: version >= 11 };
    for value in [1, 1, 0, 250, 17, 1] {
        wire.word(value);
    }
    wire.bytes.push(0); // Uncompressed tile samples.
    wire.word(0);
    wire.word(0);
    let layer = wire.slot();
    wire.pointer(0);
    wire.pointer(0); // No saved channels.
    wire.patch(layer);
    for value in [1, 1, 1, 6] {
        wire.word(value);
    }
    wire.bytes.extend_from_slice(b"Pixel\0");
    wire.word(0);
    wire.word(0);
    let hierarchy = wire.slot();
    wire.pointer(0); // No layer mask.
    wire.patch(hierarchy);
    for value in [1, 1, 8] {
        wire.word(value);
    }
    let level = wire.slot();
    wire.pointer(0);
    wire.patch(level);
    wire.word(1);
    wire.word(1);
    let tile = wire.slot();
    wire.pointer(0);
    wire.patch(tile);
    for sample in [0x1234u16, 0x5678, 0x9abc, 0xffff] {
        wire.bytes.extend_from_slice(&if little { sample.to_le_bytes() } else { sample.to_be_bytes() });
    }
    wire.bytes
}

#[test]
fn batch_historical_order_reaches_every_multibyte_version_and_preserves_sources() {
    let folder = tempfile::tempdir().unwrap();
    let actions = folder.path().join("actions.json");
    std::fs::write(&actions, "[]").unwrap();
    for (order, little) in [("little-endian", true), ("big-endian", false)] {
        let input = folder.path().join(order);
        std::fs::create_dir(&input).unwrap();
        for version in 7..=11 {
            std::fs::write(input.join(format!("v{version}.xcf")), historical_rgb16(version, little)).unwrap();
        }
        for recursive in [false, true] {
            let output = folder.path().join(format!("{order}-{recursive}"));
            let mut args = vec![
                "batch",
                "--actions",
                actions.to_str().unwrap(),
                "--in",
                input.to_str().unwrap(),
                "--out",
                output.to_str().unwrap(),
                "--format",
                "pcraft",
                "--no-gpu",
                "--historical-xcf-sample-order",
                order,
            ];
            if recursive {
                args.push("--recursive");
            }
            let result = invoke(&args);
            assert_eq!(result.code, 0, "{args:?}: {}{}", result.stdout, result.stderr);
            assert!(result.stdout.contains("5 succeeded, 0 failed"), "{}", result.stdout);
            for version in 7..=11 {
                let bytes = std::fs::read(output.join(format!("v{version}.pcraft"))).unwrap();
                let doc = photocraft_format::load_from_bytes(&bytes).unwrap();
                let pixels = doc.layers[0].surface().unwrap().read_region(photocraft_doc::Rect::new(0, 0, 1, 1));
                assert_eq!(pixels.len(), 4);
                for (actual, expected) in pixels.iter().zip([0x1234u16, 0x5678, 0x9abc, 0xffff]) {
                    assert!((*actual - f32::from(expected) / 65535.).abs() <= 1. / 65535., "v{version}, {order}: {pixels:?}");
                }
                assert_eq!(std::fs::read(input.join(format!("v{version}.xcf"))).unwrap(), historical_rgb16(version, little));
            }
        }
    }
}

#[test]
fn batch_historical_order_default_refuses_ambiguity_and_invalid_flag_is_usage_error() {
    let folder = tempfile::tempdir().unwrap();
    let input = folder.path().join("in");
    let output = folder.path().join("out");
    let actions = folder.path().join("actions.json");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(&actions, "[]").unwrap();
    std::fs::write(input.join("ambiguous.xcf"), historical_rgb16(7, true)).unwrap();
    let args =
        ["batch", "--actions", actions.to_str().unwrap(), "--in", input.to_str().unwrap(), "--out", output.to_str().unwrap(), "--format", "pcraft", "--no-gpu"];
    let result = invoke(&args);
    assert_eq!(result.code, 1, "{}{}", result.stdout, result.stderr);
    assert!(result.stdout.contains("0 succeeded, 1 failed"), "{}", result.stdout);
    assert!(result.stderr.contains("little-endian") && result.stderr.contains("big-endian"), "{}", result.stderr);
    assert!(!output.join("ambiguous.pcraft").exists());
    let mut invalid = args.to_vec();
    invalid.extend(["--historical-xcf-sample-order", "native"]);
    let result = invoke(&invalid);
    assert_eq!(result.code, 2);
    assert!(result.stderr.contains("possible values"), "{}", result.stderr);
    let help = invoke(&["batch", "--help"]);
    assert_eq!(help.code, 0);
    assert!(help.stdout.contains("--historical-xcf-sample-order"));
}

// Original historical_rgb16(7, true) bytes, wrapped by Python standard-library
// gzip/bz2/lzma compressors; decoded equality is checked below. No external assets.
const GZIP: &[u8] = &[
    31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 75, 207, 204, 45, 80, 168, 72, 78, 83, 40, 51, 48, 48, 103, 0, 2, 70, 40, 6, 129, 95, 64, 44, 136, 196, 7, 3, 107, 36,
    54, 35, 18, 102, 11, 200, 172, 72, 205, 65, 86, 153, 136, 166, 138, 3, 136, 75, 209, 196, 90, 65, 28, 19, 161, 138, 176, 61, 179, 254, 255, 7, 0, 2, 57,
    242, 181, 141, 0, 0, 0,
];
const BZIP: &[u8] = &[
    66, 90, 104, 57, 49, 65, 89, 38, 83, 89, 58, 241, 49, 207, 0, 0, 50, 123, 232, 233, 192, 48, 0, 64, 0, 68, 136, 65, 0, 43, 166, 67, 64, 2, 0, 0, 16, 0, 4,
    0, 16, 160, 0, 84, 80, 76, 153, 49, 52, 194, 105, 134, 147, 208, 105, 149, 30, 160, 50, 0, 6, 129, 108, 150, 28, 189, 231, 119, 80, 72, 2, 196, 146, 61,
    49, 153, 48, 12, 34, 96, 8, 13, 142, 221, 75, 71, 53, 228, 7, 49, 44, 57, 190, 67, 77, 0, 55, 8, 31, 232, 209, 216, 187, 146, 41, 194, 132, 129, 215, 137,
    142, 120,
];
const XZ: &[u8] = &[
    253, 55, 122, 88, 90, 0, 0, 4, 230, 214, 180, 70, 2, 0, 33, 1, 22, 0, 0, 0, 116, 47, 229, 163, 224, 0, 140, 0, 62, 93, 0, 51, 154, 73, 251, 227, 151, 228,
    69, 84, 21, 244, 125, 131, 124, 36, 16, 31, 233, 164, 69, 72, 104, 44, 242, 68, 79, 5, 220, 44, 184, 116, 82, 78, 237, 134, 197, 98, 124, 12, 140, 50, 52,
    222, 242, 70, 252, 56, 178, 33, 137, 90, 82, 218, 28, 74, 165, 70, 224, 120, 16, 4, 0, 0, 0, 0, 126, 106, 230, 248, 49, 127, 163, 141, 0, 1, 90, 141, 1, 0,
    0, 0, 10, 236, 138, 159, 177, 196, 103, 251, 2, 0, 0, 0, 0, 4, 89, 90,
];

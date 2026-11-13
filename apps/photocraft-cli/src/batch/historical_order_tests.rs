use crate::test_support::invoke;

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

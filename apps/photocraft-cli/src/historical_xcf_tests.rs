use super::*;
use photocraft_automation::{AuthorizedWorkspace, Headless};
use serde_json::json;

fn source() -> Vec<u8> {
    struct Wire(Vec<u8>);
    impl Wire {
        fn word(&mut self, n: u32) {
            self.0.extend(n.to_be_bytes());
        }
        fn pointer(&mut self, n: usize) {
            self.0.extend((n as u64).to_be_bytes());
        }
        fn slot(&mut self) -> usize {
            let at = self.0.len();
            self.pointer(0);
            at
        }
        fn patch(&mut self, at: usize) {
            let end = self.0.len() as u64;
            self.0[at..at + 8].copy_from_slice(&end.to_be_bytes());
        }
    }
    // Independent structural BE records, with explicitly LE U16 RGBA samples.
    let mut w = Wire(b"gimp xcf v011\0".to_vec());
    for n in [1, 1, 0, 250, 17, 1] {
        w.word(n);
    }
    w.0.push(0);
    w.word(0);
    w.word(0);
    let layer = w.slot();
    w.pointer(0);
    w.pointer(0);
    w.patch(layer);
    for n in [1, 1, 1, 6] {
        w.word(n);
    }
    w.0.extend(b"Pixel\0");
    w.word(0);
    w.word(0);
    let hierarchy = w.slot();
    w.pointer(0);
    w.patch(hierarchy);
    for n in [1, 1, 8] {
        w.word(n);
    }
    let level = w.slot();
    w.pointer(0);
    w.patch(level);
    w.word(1);
    w.word(1);
    let tile = w.slot();
    w.pointer(0);
    w.patch(tile);
    for n in [0x1234u16, 0x3456, 0x789a, 0x4000] {
        w.0.extend(n.to_le_bytes());
    }
    w.0
}

fn invoke(args: &[&str]) -> (i32, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run(args.iter().copied(), &mut out, &mut err);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

#[test]
fn historical_choice_uses_upstream_clap_validation_for_all_import_commands() {
    use crate::args::{Command, parse};
    let cases = [
        vec!["convert", "source.xcf", "native.pcraft"],
        vec!["info", "source.xcf"],
        vec!["run", "source.xcf", "--out", "native.pcraft"],
        vec!["batch", "--actions", "actions.json", "--in", "inputs", "--out", "outputs"],
    ];
    for base in cases {
        for (name, expected) in
            [("little-endian", photocraft_io::HistoricalXcfSampleOrder::LittleEndian), ("big-endian", photocraft_io::HistoricalXcfSampleOrder::BigEndian)]
        {
            let mut argv: Vec<OsString> = base.iter().map(OsString::from).collect();
            argv.extend(["--historical-xcf-sample-order".into(), name.into()]);
            let parsed = parse(&argv).unwrap();
            let choice = match parsed.command {
                Command::Convert(a) => a.import.historical_xcf_sample_order,
                Command::Info(a) => a.import.historical_xcf_sample_order,
                Command::Run(a) => a.import.historical_xcf_sample_order,
                Command::Batch(a) => a.import.historical_xcf_sample_order,
                _ => panic!("unexpected command"),
            };
            assert_eq!(choice, Some(expected));
            argv.extend(["--historical-xcf-sample-order".into(), name.into()]);
            assert_eq!(parse(&argv).err().unwrap().exit_code(), 2);
        }
        let mut argv: Vec<OsString> = base.iter().map(OsString::from).collect();
        argv.extend(["--historical-xcf-sample-order".into(), "guess".into()]);
        assert_eq!(parse(&argv).err().unwrap().exit_code(), 2);
        argv.pop();
        assert_eq!(parse(&argv).err().unwrap().exit_code(), 2);
    }
}

#[test]
fn historical_choice_reaches_cli_conversion_and_capability_scoped_rpc() {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("photocraft-xcf-order-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir(&dir).unwrap();
    let input = dir.join("source.xcf");
    let output = dir.join("native.pcraft");
    let bytes = source();
    std::fs::write(&input, &bytes).unwrap();
    let source_path = input.to_str().unwrap();
    let dest = output.to_str().unwrap();
    let (code, _, error) = invoke(&["convert", source_path, dest]);
    assert_eq!(code, 1, "{error}");
    assert!(error.contains("byte order"), "{error}");
    assert!(!output.exists());
    let (code, _, error) = invoke(&["convert", source_path, dest, "--historical-xcf-sample-order", "little-endian"]);
    assert_eq!(code, 0, "{error}");
    let reopened = photocraft_format::load_path(&output).unwrap();
    let pixels = reopened.layers[0].surface().unwrap().read_region(reopened.bounds());
    for (actual, expected) in pixels.iter().zip([0x1234u16, 0x3456, 0x789a, 0x4000]) {
        assert!((*actual - f32::from(expected) / 65535.).abs() < 0.000001);
    }
    assert_eq!(std::fs::read(&input).unwrap(), bytes);
    let (code, _, error) = invoke(&["info", source_path, "--historical-xcf-sample-order", "guess"]);
    assert_eq!(code, 2);
    assert!(error.contains("little-endian"), "{error}");
    let mut h = Headless::with_workspace(AuthorizedWorkspace::new(Some(&dir), None).unwrap());
    h.handle("doc.new", json!({"width": 2, "height": 2})).unwrap();
    let previous = h.session.active().unwrap().doc.id;
    assert!(h.handle("doc.open", json!({"path": "source.xcf"})).is_err());
    assert_eq!(h.session.active().unwrap().doc.id, previous);
    for invalid in [json!("guess"), json!(7), json!({})] {
        assert!(h.handle("doc.open", json!({"path": "source.xcf", "historicalXcfSampleOrder": invalid})).is_err());
        assert_eq!(h.session.active().unwrap().doc.id, previous);
    }
    h.handle("doc.open", json!({"path": "source.xcf", "historicalXcfSampleOrder": "little-endian"})).unwrap();
    let doc = &h.session.active().unwrap().doc;
    assert_eq!(doc.layers[0].surface().unwrap().read_region(doc.bounds()), pixels);
    assert!(h.handle("doc.open", json!({"path": "../source.xcf", "historicalXcfSampleOrder": "little-endian"})).is_err());
    drop(h);
    assert_eq!(dir.parent(), Some(std::env::temp_dir().as_path()));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn batch_selects_only_explicit_xcf_transport_suffixes() {
    let dir = std::env::temp_dir().join(format!("photocraft-batch-xcf-suffixes-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    for name in ["plain.xcf", "upper.XCF", "gzip.XCF.GZ", "bzip.xcf.bz2", "xz.xcf.xz", "gzip.xcfgz", "bzip.XCFBZ2", "xz.xcfxz", "ordinary.png"] {
        let path = dir.join(name);
        std::fs::write(&path, []).unwrap();
        assert!(crate::batch::is_input(&path), "supported input suffix: {name}");
    }
    for name in ["archive.gz", "photo.png.gz", "notxcf.bz2", "source.xcf.zip", "source.xcf.gz.bak", "source.xcf.gz.gz", "source.xcf.tar.xz"] {
        let path = dir.join(name);
        std::fs::write(&path, []).unwrap();
        assert!(!crate::batch::is_input(&path), "unrelated compressed suffix: {name}");
    }
    let folder = dir.join("folder.xcf");
    std::fs::create_dir(&folder).unwrap();
    assert!(!crate::batch::is_input(&folder));
    assert!(!crate::batch::is_input(&dir.join("missing.xcf")));
    assert_eq!(dir.parent(), Some(std::env::temp_dir().as_path()));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn batch_imports_historical_xcf_and_wrappers_instead_of_silently_skipping_them() {
    // Independently encoded gzip/bzip2/XZ containers around source()'s exact 177-byte wire
    // fixture. Generated once with Python's gzip.compress(mtime=0), bz2.compress and
    // lzma.compress; no corpus files, runtime compressors or extra dependencies are needed.
    const GZIP: &[u8] = &[
        31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 75, 207, 204, 45, 80, 168, 72, 78, 83, 40, 51, 48, 52, 100, 0, 2, 70, 40, 6, 129, 95, 64, 44, 136, 196, 135, 3, 119,
        6, 76, 192, 136, 132, 217, 2, 50, 43, 82, 115, 208, 85, 148, 98, 81, 205, 1, 229, 79, 196, 34, 7, 2, 43, 97, 130, 38, 66, 97, 38, 179, 42, 24, 28, 0,
        142, 140, 208, 64, 177, 0, 0, 0,
    ];
    const BZIP2: &[u8] = &[
        66, 90, 104, 57, 49, 65, 89, 38, 83, 89, 234, 41, 132, 65, 0, 0, 53, 127, 176, 237, 196, 176, 0, 64, 0, 100, 0, 64, 128, 65, 0, 11, 166, 67, 64, 32,
        16, 0, 32, 0, 16, 32, 0, 84, 80, 17, 166, 16, 100, 192, 0, 24, 162, 0, 26, 104, 1, 161, 145, 122, 165, 162, 81, 241, 176, 209, 129, 213, 48, 160, 18,
        9, 31, 144, 36, 9, 34, 228, 244, 196, 9, 214, 17, 147, 227, 253, 15, 8, 71, 176, 86, 151, 181, 241, 148, 103, 66, 46, 158, 32, 108, 93, 201, 20, 225,
        66, 67, 168, 166, 17, 4,
    ];
    const XZ: &[u8] = &[
        253, 55, 122, 88, 90, 0, 0, 4, 230, 214, 180, 70, 2, 0, 33, 1, 22, 0, 0, 0, 116, 47, 229, 163, 224, 0, 176, 0, 65, 93, 0, 51, 154, 73, 251, 227, 151,
        228, 69, 84, 21, 244, 125, 36, 209, 74, 168, 213, 229, 51, 182, 129, 197, 192, 173, 97, 61, 171, 225, 238, 156, 50, 248, 8, 85, 220, 16, 48, 11, 98,
        215, 230, 136, 121, 151, 110, 220, 148, 132, 146, 66, 209, 236, 182, 75, 200, 42, 181, 197, 127, 37, 144, 251, 7, 179, 0, 0, 0, 0, 0, 162, 245, 94, 17,
        242, 139, 37, 180, 0, 1, 93, 177, 1, 0, 0, 0, 53, 137, 94, 230, 177, 196, 103, 251, 2, 0, 0, 0, 0, 4, 89, 90,
    ];
    let inputs = [("plain.xcf", source()), ("gzip.XCF.GZ", GZIP.to_vec()), ("bzip.xcf.bz2", BZIP2.to_vec()), ("xz.xcf.xz", XZ.to_vec())];
    let dir = std::env::temp_dir().join(format!("photocraft-batch-historical-xcf-{}", std::process::id()));
    let input_dir = dir.join("in");
    std::fs::create_dir_all(&input_dir).unwrap();
    for (name, bytes) in &inputs {
        std::fs::write(input_dir.join(name), bytes).unwrap();
    }
    // This must remain excluded: arbitrary .gz is not an XCF batch candidate.
    std::fs::write(input_dir.join("unrelated.gz"), b"not an image").unwrap();
    let actions = dir.join("actions.json");
    std::fs::write(&actions, b"[]").unwrap();
    let args = |output: &std::path::Path| {
        vec![
            "batch".to_string(),
            "--actions".into(),
            actions.to_str().unwrap().into(),
            "--in".into(),
            input_dir.to_str().unwrap().into(),
            "--out".into(),
            output.to_str().unwrap().into(),
            "--format".into(),
            "pcraft".into(),
        ]
    };
    let run_args = |args: &[String]| invoke(&args.iter().map(String::as_str).collect::<Vec<_>>());

    let refused = dir.join("missing-order");
    let (code, stdout, stderr) = run_args(&args(&refused));
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(stdout.contains("0 succeeded, 4 failed"), "inputs must reach import, not become an empty successful batch: {stdout}");
    assert_eq!(stderr.matches("FAIL  ").count(), 4, "{stderr}");
    assert!(stderr.contains("byte order"), "{stderr}");
    assert_eq!(std::fs::read_dir(&refused).unwrap().count(), 0);

    let invalid = dir.join("invalid-order");
    let mut invalid_args = args(&invalid);
    invalid_args.extend(["--historical-xcf-sample-order".into(), "guess".into()]);
    let (code, _, stderr) = run_args(&invalid_args);
    assert_eq!(code, 2, "invalid order must use clap's usage failure: {stderr}");
    assert!(stderr.contains("little-endian"), "{stderr}");
    assert!(!invalid.exists(), "invalid CLI params must not start the batch");

    let output = dir.join("native");
    let mut valid_args = args(&output);
    valid_args.extend(["--historical-xcf-sample-order".into(), "little-endian".into()]);
    let (code, stdout, stderr) = run_args(&valid_args);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("4 succeeded, 0 failed"), "{stdout}");
    assert_eq!(stdout.matches("ok    ").count(), 4, "{stdout}");
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 4);
    for (name, original) in inputs {
        let source_path = input_dir.join(name);
        let stem = source_path.file_stem().unwrap().to_str().unwrap();
        let doc = photocraft_format::load_path(&output.join(format!("{stem}.pcraft"))).unwrap();
        let pixels = doc.layers[0].surface().unwrap().read_region(doc.bounds());
        assert_eq!(pixels.len(), 4, "{name}: one native RGBA pixel");
        for (actual, expected) in pixels.iter().zip([0x1234u16, 0x3456, 0x789a, 0x4000]) {
            assert_eq!((*actual * 65535.).round() as u16, expected, "{name}: exact source U16 sample code");
        }
        assert_eq!(std::fs::read(&source_path).unwrap(), original, "{name}: source must remain untouched");
    }
    assert_eq!(std::fs::read(input_dir.join("unrelated.gz")).unwrap(), b"not an image");
    assert_eq!(dir.parent(), Some(std::env::temp_dir().as_path()));
    std::fs::remove_dir_all(dir).unwrap();
}

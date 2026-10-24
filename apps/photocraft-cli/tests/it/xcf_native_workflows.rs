//! Direct CLI conversion must prepare native data even though it does not install a Session.
use photocraft_engine::Session;
use serde_json::json;

#[test]
fn convert_cannot_replace_an_import_only_source_even_with_in_place() {
    let mut bytes = b"gimp xcf v003\0".to_vec();
    for value in [1u32, 1, 0, 0, 0, 0, 0] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("pc-cli-source-protection-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let source = dir.join("source.xcf");
    let alias = dir.join("alias.pcraft");
    std::fs::write(&source, &bytes).unwrap();
    std::fs::hard_link(&source, &alias).unwrap();
    for destination in [&source, &alias] {
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let code = photocraft_cli::run(
            &[
                "convert".into(),
                source.to_string_lossy().into_owned(),
                destination.to_string_lossy().into_owned(),
                "--format".into(),
                "pcraft".into(),
                "--in-place".into(),
            ],
            &mut stdout,
            &mut stderr,
        );
        assert_ne!(code, 0);
        assert!(String::from_utf8_lossy(&stderr).contains("import-only source"), "{}", String::from_utf8_lossy(&stderr));
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        assert_eq!(std::fs::read(&alias).unwrap(), bytes);
    }
    std::fs::remove_file(alias).unwrap();
    std::fs::remove_file(source).unwrap();
    std::fs::remove_dir(dir).unwrap();
}

#[test]
fn direct_convert_renders_missing_native_smart_cache_before_writing() {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 1, "height": 1, "background": "white"})).unwrap();
    session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let bytes = photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap();
    let zip = photocraft_format::zip::ZipReader::new(&bytes).unwrap();
    let mut writer = photocraft_format::zip::ZipWriter::new();
    for entry in &zip.entries {
        let mut data = zip.read(entry, usize::MAX).unwrap();
        if entry.name == "manifest.json" {
            let mut value: serde_json::Value = serde_json::from_slice(&data).unwrap();
            value["document"]["layers"][0]["content"]["cache"] = serde_json::Value::Null;
            data = serde_json::to_vec(&value).unwrap();
        }
        writer.add(&entry.name, &data).unwrap();
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("pc-cli-native-preparation-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let source = dir.join("source.pcraft");
    let output = dir.join("output.png");
    std::fs::write(&source, writer.finish().unwrap()).unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = photocraft_cli::run(&["convert".into(), source.to_string_lossy().into_owned(), output.to_string_lossy().into_owned()], &mut stdout, &mut stderr);
    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&stderr));
    let doc = photocraft_io::import("output.png", &std::fs::read(&output).unwrap()).unwrap().document;
    let pixels = doc.layers[0].surface().unwrap().read_region(doc.bounds());
    assert!(pixels.len() >= 3 && pixels.iter().take(3).all(|&value| value == 1.0), "native white pixels expected, got {pixels:?}");
    std::fs::remove_file(source).unwrap();
    std::fs::remove_file(output).unwrap();
    std::fs::remove_dir(dir).unwrap();
}

#[test]
fn info_inherits_conversion_summary_for_xcf_and_native_reopen() {
    let mut bytes = b"gimp xcf v003\0".to_vec();
    for value in [1u32, 1, 0, 0, 0, 0, 0] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    let imported = photocraft_io::import("source.bin", &bytes).unwrap();
    let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("pc-cli-import-info-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let mut source_summary = None;
    for (name, data) in [("source.bin", bytes), ("converted.pcraft", native)] {
        let path = dir.join(name);
        std::fs::write(&path, data).unwrap();
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let code = photocraft_cli::run(&["info".into(), path.to_string_lossy().into_owned()], &mut stdout, &mut stderr);
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&stderr));
        let result: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(result["importConversion"]["sourceFormat"], "xcf");
        assert!(result["importConversion"]["entryCount"].as_u64().is_some_and(|count| count > 0));
        if let Some(summary) = &source_summary {
            assert_eq!(&result["importConversion"], summary, "native reopen must preserve every source outcome");
        } else {
            source_summary = Some(result["importConversion"].clone());
        }
        assert!(result["warnings"].is_array());
        std::fs::remove_file(path).unwrap();
    }
    std::fs::remove_dir(dir).unwrap();
}

/// One ordinary RGBA layer, with uncompressed tile bytes and no private corpus input.
fn layered_xcf() -> Vec<u8> {
    fn word(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_be_bytes());
    }
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

struct RouteFiles(std::path::PathBuf);
impl RouteFiles {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("pc-cli-xcf-routes-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}
impl Drop for RouteFiles {
    fn drop(&mut self) {
        if let Ok(entries) = std::fs::read_dir(&self.0) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn call(args: Vec<String>) -> (i32, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = photocraft_cli::run(&args, &mut out, &mut err);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

#[test]
fn all_xcf_transports_support_info_convert_native_edit_and_source_independent_reopen() {
    let mut expected_summary = None;
    for (transport, bytes) in wrapped_sources() {
        // Content must win even when a wrapper has a suffix normally forcing native loading.
        for suffix in ["xcf", "pcraft", "bin"] {
            let files = RouteFiles::new();
            let source = files.path(&format!("{transport}.{suffix}"));
            let native = files.path("converted.pcraft");
            let edited = files.path("edited.pcraft");
            let flat = files.path("exported.png");
            std::fs::write(&source, &bytes).unwrap();
            let (code, info, err) = call(vec!["info".into(), source.clone(), "--compact".into()]);
            assert_eq!(code, 0, "{transport}/{suffix}: {err}");
            let info: serde_json::Value = serde_json::from_str(&info).unwrap();
            assert_eq!(info["importConversion"]["sourceFormat"], "xcf");
            assert!(info["warnings"].is_array());
            if let Some(expected) = &expected_summary {
                assert_eq!(&info["importConversion"], expected);
            } else {
                expected_summary = Some(info["importConversion"].clone());
            }
            let (code, _, err) = call(vec!["convert".into(), source.clone(), native.clone()]);
            assert_eq!(code, 0, "{transport}/{suffix}: {err}");
            let (code, output, err) = call(vec![
                "run".into(),
                source.clone(),
                "--cmd".into(),
                "layer.setProps".into(),
                "--params".into(),
                "{\"opacity\":0.5}".into(),
                "--cmd".into(),
                "document.importReport".into(),
                "--out".into(),
                edited.clone(),
            ]);
            assert_eq!(code, 0, "{transport}/{suffix}: {err}");
            let responses: Vec<serde_json::Value> = output.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
            assert_eq!(responses[1]["command"], "document.importReport");
            assert_eq!(std::fs::read(&source).unwrap(), bytes);
            std::fs::remove_file(&source).unwrap();
            let (code, reopened, err) = call(vec!["info".into(), edited.clone(), "--compact".into()]);
            assert_eq!(code, 0, "native reopen without source: {err}");
            let reopened: serde_json::Value = serde_json::from_str(&reopened).unwrap();
            assert_eq!(reopened["importConversion"], info["importConversion"]);
            let doc = photocraft_io::import("edited.pcraft", &std::fs::read(&edited).unwrap()).unwrap().document;
            assert_eq!(doc.layers.len(), 1);
            assert_eq!(doc.layers[0].opacity, 0.5);
            assert_eq!(doc.layers[0].surface().unwrap().read_region(doc.bounds()), vec![128.0 / 255.0, 64.0 / 255.0, 32.0 / 255.0, 1.0]);
            let (code, _, err) = call(vec![
                "run".into(),
                edited,
                "--cmd".into(),
                "layer.setProps".into(),
                "--params".into(),
                "{\"opacity\":0.75}".into(),
                "--out".into(),
                flat.clone(),
            ]);
            assert_eq!(code, 0, "native edit/export: {err}");
            let exported = photocraft_io::import("exported.png", &std::fs::read(flat).unwrap()).unwrap().document;
            let pixel = exported.layers[0].surface().unwrap().read_region(exported.bounds());
            for (actual, expected) in pixel.iter().zip([128.0 / 255.0, 64.0 / 255.0, 32.0 / 255.0, 0.75]) {
                assert!((actual - expected).abs() <= 1.0 / 255.0, "native opacity/export: {pixel:?}");
            }
        }
    }
}

#[test]
fn compressed_and_misnamed_sources_reject_convert_and_run_to_hard_link_aliases() {
    for (transport, bytes) in wrapped_sources() {
        let files = RouteFiles::new();
        let source = files.path(&format!("{transport}.pcraft"));
        let alias = files.path("alias.pcraft");
        std::fs::write(&source, &bytes).unwrap();
        std::fs::hard_link(&source, &alias).unwrap();
        for destination in [&source, &alias] {
            for args in [
                vec!["convert".into(), source.clone(), destination.clone(), "--in-place".into()],
                vec![
                    "run".into(),
                    source.clone(),
                    "--cmd".into(),
                    "layer.setProps".into(),
                    "--params".into(),
                    "{\"opacity\":0.25}".into(),
                    "--out".into(),
                    destination.clone(),
                ],
                vec![
                    "run".into(),
                    source.clone(),
                    "--cmd".into(),
                    "layer.setProps".into(),
                    "--params".into(),
                    "{\"opacity\":0.25}".into(),
                    "--out".into(),
                    destination.clone(),
                    "--in-place".into(),
                ],
            ] {
                let in_place = args.iter().any(|arg| arg == "--in-place");
                let (code, _, err) = call(args);
                assert_ne!(code, 0, "{transport} overwrote source");
                // Upstream rejects aliases before decoding unless explicitly in-place;
                // opting in must still reach the import-only protection.
                let expected = if in_place { "import-only source" } else { "input file" };
                assert!(err.contains(expected), "{transport}: {err}");
                assert_eq!(std::fs::read(&source).unwrap(), bytes);
                assert_eq!(std::fs::read(&alias).unwrap(), bytes);
            }
        }
    }
}

#[test]
fn corrupt_sources_fail_all_cli_routes_without_output_or_source_changes() {
    for (transport, mut bytes) in wrapped_sources() {
        bytes.truncate(bytes.len() - 3);
        let files = RouteFiles::new();
        let source = files.path(&format!("{transport}.xcf"));
        let output = files.path("output.pcraft");
        std::fs::write(&source, &bytes).unwrap();
        for args in [
            vec!["info".into(), source.clone()],
            vec!["convert".into(), source.clone(), output.clone()],
            vec![
                "run".into(),
                source.clone(),
                "--cmd".into(),
                "layer.setProps".into(),
                "--params".into(),
                "{\"opacity\":0.25}".into(),
                "--out".into(),
                output.clone(),
            ],
        ] {
            let (code, _, err) = call(args);
            assert_ne!(code, 0, "{transport} accepted truncated stream");
            assert!(!err.is_empty());
            assert!(!std::path::Path::new(&output).exists());
            assert_eq!(std::fs::read(&source).unwrap(), bytes);
        }
    }
}

/// Independently authored v26 framing, original pixels, and one typed stored
/// adjustment. No native document bytes or external corpus define this input.
fn stored_native_effect_xcf(operation: &str) -> Vec<u8> {
    struct W(Vec<u8>);
    impl W {
        fn word(&mut self, v: u32) {
            self.0.extend(v.to_be_bytes());
        }
        fn ptr(&mut self, v: usize) {
            self.0.extend((v as u64).to_be_bytes());
        }
        fn slot(&mut self) -> usize {
            let at = self.0.len();
            self.ptr(0);
            at
        }
        fn patch(&mut self, at: usize) {
            let value = self.0.len() as u64;
            self.0[at..at + 8].copy_from_slice(&value.to_be_bytes());
        }
        fn text(&mut self, v: &str) {
            self.word(v.len() as u32 + 1);
            self.0.extend(v.as_bytes());
            self.0.push(0);
        }
        fn prop(&mut self, kind: u32, data: &[u8]) {
            self.word(kind);
            self.word(data.len() as u32);
            self.0.extend(data);
        }
        fn end(&mut self) {
            self.word(0);
            self.word(0);
        }
    }
    let mut w = W(b"gimp xcf v026\0".to_vec());
    for value in [4, 3, 0, 150] {
        w.word(value);
    }
    w.prop(17, &[0]);
    w.prop(9001, &[1, 3, 5, 7]);
    w.end();
    let layer = w.slot();
    w.ptr(0);
    w.ptr(0);
    w.ptr(0);
    w.patch(layer);
    for value in [4, 3, 1] {
        w.word(value);
    }
    w.text("Stored filter owner");
    w.end();
    let hierarchy = w.slot();
    w.ptr(0);
    let effect = w.slot();
    w.ptr(0);
    w.patch(hierarchy);
    for value in [4, 3, 4] {
        w.word(value);
    }
    let level = w.slot();
    w.ptr(0);
    w.patch(level);
    w.word(4);
    w.word(3);
    let tile = w.slot();
    w.ptr(0);
    w.patch(tile);
    w.0.extend([64, 128, 192, 128].repeat(12));
    w.patch(effect);
    w.text("Original stored adjustment");
    w.text("");
    w.text(operation);
    w.text(if operation == "gegl:exposure" { "1:0" } else { "0:0" });
    if operation == "gegl:exposure" {
        let mut arg = W(Vec::new());
        arg.text("exposure");
        arg.word(3);
        arg.0.extend(1f32.to_be_bytes());
        w.prop(45, &arg.0);
    }
    w.end();
    w.ptr(0);
    w.0
}
/// A fresh ordinary native command chain on the original pixels is the oracle;
/// import output, report data and source-operation dispatch are never consulted.
fn independently_native_stored_filter_samples(operation: &str) -> Vec<f32> {
    let mut native = Session::new();
    native.execute("file.new", json!({"width":4,"height":3,"background":"transparent"})).unwrap();
    native.execute("edit.assignProfile", json!({"profile":"srgb"})).unwrap();
    let doc = std::sync::Arc::make_mut(&mut native.active_mut().unwrap().doc);
    let bounds = doc.bounds();
    doc.layers[0].surface_mut().unwrap().write_region(bounds, &[64u8, 128, 192, 128].repeat(12).iter().map(|v| f32::from(*v) / 255.).collect::<Vec<_>>());
    native.execute("filter.convertForSmartFilters", json!({})).unwrap();
    if operation == "gegl:exposure" {
        native.execute("image.adjustments.exposure", json!({"exposure":1.,"offset":0.,"gamma":1.})).unwrap();
    } else {
        native.execute("image.adjustments.invert", json!({})).unwrap();
    }
    let doc = &native.active().unwrap().doc;
    assert!(doc.metadata.import_conversion.is_none());
    doc.layers[0].surface().unwrap().read_region(doc.bounds())
}
fn assert_stored_filter_pixels(actual: &[f32], expected: &[f32], route: &str) {
    assert_eq!(actual.len(), expected.len(), "{route}: sample count");
    let max = actual.iter().zip(expected).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
    assert!(actual.iter().all(|v| v.is_finite()) && max <= 2. / 255., "{route}: native counterpart max sample error {max}");
}

#[test]
fn actual_stored_xcf_adjustments_direct_convert_match_native_commands_and_reopen_without_source() {
    for operation in ["gegl:invert-gamma", "gegl:exposure"] {
        let files = RouteFiles::new();
        let source = files.path("source.xcf");
        let first = files.path("first.png");
        let converted = files.path("converted.pcraft");
        let reopened = files.path("reopened.pcraft");
        let bytes = stored_native_effect_xcf(operation);
        std::fs::write(&source, &bytes).unwrap();
        let expected = independently_native_stored_filter_samples(operation);
        let (code, info, err) = call(vec!["info".into(), source.clone(), "--compact".into()]);
        assert_eq!(code, 0, "{operation}: {err}");
        let info: serde_json::Value = serde_json::from_str(&info).unwrap();
        assert_eq!(info["importConversion"]["sourceFormat"], "xcf");
        let (code, _, err) = call(vec!["convert".into(), source.clone(), first.clone()]);
        assert_eq!(code, 0, "{operation}: first native output: {err}");
        let png = photocraft_io::import("first.png", &std::fs::read(&first).unwrap()).unwrap().document;
        assert_stored_filter_pixels(&png.layers[0].surface().unwrap().read_region(png.bounds()), &expected, operation);
        let (code, _, err) = call(vec!["convert".into(), source.clone(), converted.clone()]);
        assert_eq!(code, 0, "{operation}: native save: {err}");
        let native = photocraft_io::import("converted.pcraft", &std::fs::read(&converted).unwrap()).unwrap().document;
        assert_eq!(native.layers[0].content.kind_name(), "Smart Object");
        assert_stored_filter_pixels(
            &native.layers[0].surface().expect("CLI must persist prepared native cache").read_region(native.bounds()),
            &expected,
            operation,
        );
        let report = native.metadata.import_conversion.clone();
        let mut session = Session::new();
        session.add_document(native, None);
        let page = session.execute("document.importReport", json!({"limit":250})).unwrap();
        assert!(page["entries"].as_array().unwrap().iter().any(|row| row["featureCode"] == "xcf.layer_effect" && row["disposition"] == "native_approximation"));
        assert!(
            page["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["featureCode"] == "xcf.source_property.9001" && row["disposition"] == "retained_unapplied")
        );
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        std::fs::remove_file(&source).unwrap();
        drop(bytes);
        let (code, _, err) = call(vec!["convert".into(), converted.clone(), reopened.clone()]);
        assert_eq!(code, 0, "{operation}: reopen without source: {err}");
        let native = photocraft_io::import("reopened.pcraft", &std::fs::read(&reopened).unwrap()).unwrap().document;
        assert_eq!(native.metadata.import_conversion, report);
        assert_stored_filter_pixels(&native.layers[0].surface().unwrap().read_region(native.bounds()), &expected, operation);
        let (code, native_info, err) = call(vec!["info".into(), reopened, "--compact".into()]);
        assert_eq!(code, 0, "{operation}: {err}");
        let native_info: serde_json::Value = serde_json::from_str(&native_info).unwrap();
        assert_eq!(native_info["importConversion"], info["importConversion"]);
    }
}

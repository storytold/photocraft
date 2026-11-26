//! File › Open As… (#14049): an error names the file that was chosen and the format asked for,
//! never the stem plus the `as` extension that only picks the decoder.

use super::*;

fn tmp(name: &str) -> String {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("photocraft-open-as-name-{}-{name}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

fn write_notes(dir: &str) -> String {
    let path = join(dir, "notes.txt");
    std::fs::write(&path, b"just some text.\n").unwrap();
    path
}

fn error_text(s: &mut Session, path: &str, as_ext: &str) -> String {
    match s.execute("file.openAs", json!({"path": path, "as": as_ext})) {
        Err(e) => e.to_string(),
        Ok(v) => panic!("opening a text file as {as_ext} succeeded: {v}"),
    }
}

#[test]
fn a_text_file_opened_as_pcx_names_notes_txt_and_pcx() {
    let dir = tmp("pcx");
    let path = write_notes(&dir);
    let mut s = crate::session();
    let msg = error_text(&mut s, &path, "pcx");
    assert!(msg.contains("notes.txt"), "{msg}");
    assert!(msg.contains("PCX"), "{msg}");
    assert!(!msg.contains("notes.pcx"), "{msg}");
    assert!(msg.contains("unrecognized image format"), "the decoder's detail stays: {msg}");
    assert_eq!(s.documents().len(), 0);
}

#[test]
fn a_text_file_opened_as_raw_names_notes_txt_and_raw() {
    let dir = tmp("raw");
    let path = write_notes(&dir);
    let mut s = crate::session();
    let msg = error_text(&mut s, &path, "raw");
    assert!(msg.contains("notes.txt"), "{msg}");
    assert!(msg.contains("RAW"), "{msg}");
    assert!(!msg.contains("notes.raw"), "{msg}");
    assert_eq!(s.documents().len(), 0);
}

#[test]
fn an_unknown_as_value_is_a_parameter_error_naming_it() {
    let dir = tmp("bogus");
    let path = write_notes(&dir);
    let mut s = crate::session();
    match s.execute("file.openAs", json!({"path": path, "as": "bogus"})) {
        Err(EngineError::BadParams { cmd, msg }) => {
            assert_eq!(cmd, "file.openAs");
            assert!(msg.contains("bogus"), "{msg}");
        }
        other => panic!("expected BadParams, got {other:?}"),
    }
    assert_eq!(s.documents().len(), 0);
}

#[test]
fn every_camera_raw_and_affinity_extension_is_an_open_as_format() {
    // The `as` check must take every format the importer reads, not a copy of the lists that can
    // fall behind them: a text file asked to open as one fails to read, never as a parameter error.
    let dir = tmp("raw-exts");
    let path = write_notes(&dir);
    let mut s = crate::session();
    for ext in photocraft_io::raw::EXTENSIONS.iter().chain(photocraft_io::affinity::EXTENSIONS) {
        let r = s.execute("file.openAs", json!({"path": path, "as": ext}));
        assert!(!matches!(r, Err(EngineError::BadParams { .. })), "{ext}: {r:?}");
    }
}

#[test]
fn a_png_opened_as_png_keeps_its_real_file_name() {
    let dir = tmp("png");
    let img = photocraft_codecs::Image::from_u8(4, 4, photocraft_codecs::ChannelLayout::Rgb, vec![200; 48]).unwrap();
    let bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let path = join(&dir, "photo one.png");
    std::fs::write(&path, bytes).unwrap();
    let mut s = crate::session();
    s.execute("file.openAs", json!({"path": path, "as": "png"})).unwrap();
    assert_eq!(s.documents().len(), 1);
    assert_eq!(s.active().unwrap().doc.name, "photo one.png");
}

#[test]
fn historical_order_params_are_exact_and_clear_other_source_approvals() {
    use photocraft_io::{HistoricalXcfOrder as Policy, HistoricalXcfSampleOrder as Order};
    for (value, expected) in [(json!("little-endian"), Order::LittleEndian), (json!("big-endian"), Order::BigEndian)] {
        let params = json!({"historicalXcfSampleOrder": value, "approveHighMemory": true});
        assert_eq!(historical_xcf_sample_order(&params, "file.openAs").unwrap(), Some(expected));
        let opts = import_options_from_params(Default::default(), &params, "file.openAs").unwrap();
        assert_eq!(opts.historical_xcf_order, Policy::Explicit(expected));
        assert!(opts.psd_high_memory_approved);
    }
    for params in [json!({}), json!({"historicalXcfSampleOrder": null, "approveHighMemory": null})] {
        let base = photocraft_io::ImportOptions {
            historical_xcf_order: Policy::Explicit(Order::LittleEndian),
            psd_high_memory_approved: true,
            psd_approved_estimate: Some(123),
            max_group_depth: 17,
            ..Default::default()
        };
        let opts = import_options_from_params(base, &params, "file.openAs").unwrap();
        assert_eq!(opts.historical_xcf_order, Policy::RequireExplicit);
        assert!(!opts.psd_high_memory_approved);
        assert!(opts.psd_approved_estimate.is_none());
        assert_eq!(opts.max_group_depth, 17, "unrelated caller limits survive");
    }
}

#[test]
fn open_as_rejects_invalid_order_before_reading_or_changing_a_document() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 2, "height": 2})).unwrap();
    let before = s.active().unwrap().doc.clone();
    for value in [json!(true), json!(1), json!([]), json!({}), json!("LE"), json!("Little-Endian"), json!("big-endian "), json!("")] {
        let err = s.execute("file.openAs", json!({"path": "nonexistent-for-order-validation.xcf", "historicalXcfSampleOrder": value}));
        assert!(
            matches!(err, Err(EngineError::BadParams { ref cmd, ref msg }) if cmd == "file.openAs" && msg.contains("historicalXcfSampleOrder")),
            "{value}: {err:?}"
        );
        assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
        assert_eq!(s.documents().len(), 1);
    }
    for value in [json!("true"), json!(1), json!([])] {
        let err = s.execute("file.openAs", json!({"path": "nonexistent-for-order-validation.xcf", "approveHighMemory": value}));
        assert!(matches!(err, Err(EngineError::BadParams { ref msg, .. }) if msg.contains("approveHighMemory")));
        assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
    }
}

/// A single raw RGBA16 tile in a version-10 file. Structural words are always BE;
/// the intentionally asymmetric sample bytes let the command's explicit choice be observed.
fn historical_rgba16() -> Vec<u8> {
    fn word(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_be_bytes());
    }
    fn patch(out: &mut [u8], slot: usize, target: usize) {
        out[slot..slot + 4].copy_from_slice(&(target as u32).to_be_bytes());
    }
    let mut out = b"gimp xcf v010\0".to_vec();
    for value in [1, 1, 0, 250, 17, 1] {
        word(&mut out, value);
    }
    out.push(0); // no compression
    for value in [0, 0] {
        word(&mut out, value);
    }
    let layer = out.len();
    for value in [0, 0, 0] {
        word(&mut out, value);
    }
    let at = out.len();
    patch(&mut out, layer, at);
    for value in [1, 1, 1, 2] {
        word(&mut out, value);
    }
    out.extend_from_slice(b"L\0");
    for value in [0, 0] {
        word(&mut out, value);
    }
    let hierarchy = out.len();
    for value in [0, 0] {
        word(&mut out, value);
    }
    let at = out.len();
    patch(&mut out, hierarchy, at);
    for value in [1, 1, 8] {
        word(&mut out, value);
    }
    let level = out.len();
    for value in [0, 0] {
        word(&mut out, value);
    }
    let at = out.len();
    patch(&mut out, level, at);
    for value in [1, 1] {
        word(&mut out, value);
    }
    let tile = out.len();
    for value in [0, 0] {
        word(&mut out, value);
    }
    let at = out.len();
    patch(&mut out, tile, at);
    out.extend_from_slice(&[0x12, 0x34, 0x23, 0x45, 0x34, 0x56, 0xff, 0xff]);
    out
}

#[test]
fn open_as_forwards_each_explicit_stored_order_without_guessing_missing_or_null() {
    let dir = tmp("xcf-order");
    let path = join(&dir, "historical.data");
    let bytes = historical_rgba16();
    std::fs::write(&path, &bytes).unwrap();
    for (order, expected) in [("big-endian", [0x1234u16, 0x2345, 0x3456]), ("little-endian", [0x3412u16, 0x4523, 0x5634])] {
        let mut s = crate::session();
        s.execute("file.openAs", json!({"path": path, "as": "xcf", "historicalXcfSampleOrder": order})).unwrap();
        let pixel = s.active().unwrap().doc.layers[0].surface().unwrap().pixel(0, 0);
        for (actual, expected) in pixel.iter().zip(expected) {
            assert!((*actual - f32::from(expected) / 65535.).abs() < 0.00002, "{order}: {pixel:?}");
        }
        assert_eq!(pixel[3], 1.);
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "opening never rewrites the source");
    }
    for params in [json!({"path": path, "as": "xcf"}), json!({"path": path, "as": "xcf", "historicalXcfSampleOrder": null})] {
        let mut s = crate::session();
        assert!(s.execute("file.openAs", params).is_err());
        assert!(s.documents().is_empty(), "an unanswered source is not admitted");
    }
    // The pre-existing byte API keeps its compatible default rather than gaining a command prompt.
    let mut compatible = crate::session();
    open_bytes_as_with_options(&mut compatible, "historical.xcf", &bytes, None, None, false).unwrap();
    let pixel = compatible.active().unwrap().doc.layers[0].surface().unwrap().pixel(0, 0);
    assert!((pixel[0] - f32::from(0x1234u16) / 65535.).abs() < 0.00002);
}

#[test]
fn every_xcf_wrapper_alias_is_accepted_by_open_as_and_the_format_alias() {
    let dir = tmp("xcf-aliases");
    let path = join(&dir, "source.data");
    std::fs::write(&path, historical_rgba16()).unwrap();
    for ext in ["xcf", "xcf.gz", "xcfgz", "xcf.bz2", "xcfbz2", "xcf.xz", "xcfxz", ".XCF.XZ"] {
        let mut s = crate::session();
        s.execute("file.openAs", json!({"path": path, "format": ext, "historicalXcfSampleOrder": "big-endian"})).unwrap();
        assert_eq!(s.active().unwrap().doc.size, photocraft_geom::Size::new(1, 1));
    }
}

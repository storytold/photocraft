//! File › Open As › Photoshop Raw (#14051): a file read as nothing but pixel bytes, laid out by
//! the Raw Options fields `file.openAs` takes with `"as": "raw"`.

use super::*;
use photocraft_io::photoshop_raw::{ByteOrder, RawLayout};

/// The measured file: 16 bytes, read as 4 rows of 4 gray pixels.
const NOTES: &[u8; 16] = b"hello world 1234";

/// A fresh folder holding `bytes` as `notes.txt`; returns the file's path.
fn notes_file(case: &str, bytes: &[u8]) -> String {
    let dir = std::env::temp_dir().join(format!("photocraft-open-as-raw-{}-{case}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = join(&dir.to_string_lossy(), "notes.txt");
    std::fs::write(&path, bytes).unwrap();
    path
}

/// `file.openAs` on `path` as Photoshop Raw with the Raw Options fields in `fields`.
fn open_raw(s: &mut Session, path: &str, fields: Value) -> Result<Value> {
    let mut p = json!({"path": path, "as": "raw"});
    p.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
    s.execute("file.openAs", p)
}

/// The active document's pixels as 8-bit gray values, row by row.
fn gray_bytes(s: &Session) -> Vec<u8> {
    let doc = &s.active().unwrap().doc;
    let surface = doc.layers[0].surface().unwrap();
    let (w, h) = (doc.size.width as i32, doc.size.height as i32);
    (0..h).flat_map(|y| (0..w).map(move |x| (y, x))).map(|(y, x)| (surface.pixel(x, y)[0] * 255.0).round() as u8).collect()
}

/// An open that must fail without a document: returns the error's text.
fn refused(s: &mut Session, path: &str, fields: Value) -> String {
    let e = open_raw(s, path, fields.clone()).expect_err(&format!("{fields} opened"));
    assert_eq!(s.documents().len(), 0, "{fields}");
    e.to_string()
}

#[test]
fn notes_txt_with_the_measured_defaults_opens_as_its_bytes_in_a_4_by_4_grayscale_document() {
    let path = notes_file("measured", NOTES);
    let mut s = crate::session();
    open_raw(&mut s, &path, json!({"width": 4, "height": 4, "channels": 1, "depth": 8, "byteOrder": "mac", "interleaved": true, "header": 0})).unwrap();
    assert_eq!(s.documents().len(), 1);
    let st = s.active().unwrap();
    assert_eq!(st.doc.name, "notes.txt");
    assert_eq!((st.doc.size.width, st.doc.size.height), (4, 4));
    assert_eq!(st.doc.mode, photocraft_color::ColorMode::Grayscale);
    assert_eq!(st.doc.depth, photocraft_doc::SampleType::U8);
    let names: Vec<&str> = st.doc.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Background"]);
    // Row 1 `hell`, row 2 `o wo`, row 3 `rld `, row 4 `1234`.
    assert_eq!(gray_bytes(&s), NOTES);
    let d = crate::inspect::document(s.active().unwrap());
    assert_eq!((d["history"].clone(), d["snapshots"].clone()), (json!(["Open"]), json!([{"id": 0, "name": "notes.txt"}])));
}

#[test]
fn width_and_height_alone_take_the_other_fields_from_raw_options_defaults() {
    let path = notes_file("defaults", NOTES);
    let mut s = crate::session();
    open_raw(&mut s, &path, json!({"width": 4, "height": 4})).unwrap();
    assert_eq!(gray_bytes(&s), NOTES);
    // Null is the same as leaving a field out.
    open_raw(&mut s, &path, json!({"width": 4, "height": 4, "channels": null, "depth": null, "header": null})).unwrap();
    assert_eq!(gray_bytes(&s), NOTES);
}

#[test]
fn the_width_and_height_given_shape_the_rows() {
    let path = notes_file("shape", NOTES);
    let mut s = crate::session();
    open_raw(&mut s, &path, json!({"width": 8, "height": 2})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert_eq!((doc.size.width, doc.size.height), (8, 2));
    let surface = doc.layers[0].surface().unwrap();
    // Row 1 is `hello wo`, row 2 starts at the ninth byte, `r`.
    assert_eq!((surface.pixel(0, 1)[0] * 255.0).round() as u8, b'r');
    assert_eq!((surface.pixel(7, 0)[0] * 255.0).round() as u8, b'o');
    assert_eq!(gray_bytes(&s), NOTES);
}

#[test]
fn a_header_is_skipped_before_the_first_pixel() {
    let mut bytes = b"HDR".to_vec();
    bytes.extend_from_slice(NOTES);
    let path = notes_file("header", &bytes);
    let mut s = crate::session();
    open_raw(&mut s, &path, json!({"width": 4, "height": 4, "header": 3})).unwrap();
    assert_eq!(gray_bytes(&s), NOTES);
}

#[test]
fn the_document_is_never_saved_back_over_the_raw_file() {
    let path = notes_file("read-only", NOTES);
    let mut s = crate::session();
    open_raw(&mut s, &path, json!({"width": 4, "height": 4})).unwrap();
    let st = s.active().unwrap();
    assert!(st.source_read_only);
    assert_eq!(st.path, None);
    assert_eq!(std::fs::read(&path).unwrap(), NOTES);
}

#[test]
fn sizes_the_file_cannot_fill_are_refused_without_a_document() {
    let path = notes_file("misfit", NOTES);
    let mut s = crate::session();
    for (fields, says) in [
        // More bytes than the file has, and fewer.
        (json!({"width": 5, "height": 4}), "take 20 bytes, but the file has 16"),
        (json!({"width": 2, "height": 2}), "take 4 bytes, but the file has 16"),
        // Zero and huge dimensions.
        (json!({"width": 0, "height": 4}), "width and height must be 1 to 300000 pixels"),
        (json!({"width": 4, "height": 0}), "width and height must be 1 to 300000 pixels"),
        (json!({"width": 300_001, "height": 1}), "width and height must be 1 to 300000 pixels"),
        (json!({"width": 300_000, "height": 300_000, "channels": 56, "depth": 32}), "take 20160000000000 bytes"),
        // Absurd and misplaced headers.
        (json!({"width": 4, "height": 4, "header": u64::MAX}), "header is longer than the file (16 bytes)"),
        (json!({"width": 4, "height": 4, "header": 17}), "header is longer than the file"),
        (json!({"width": 4, "height": 4, "header": 16}), "the file has 0 after its 16-byte header"),
        (json!({"width": 4, "height": 4, "header": 1}), "the file has 15 after its 1-byte header"),
        // Channel counts and depths Raw Options doesn't offer.
        (json!({"width": 4, "height": 4, "channels": 0}), "channel count must be 1 to 56"),
        (json!({"width": 4, "height": 4, "channels": 57}), "channel count must be 1 to 56"),
        (json!({"width": 4, "height": 4, "depth": 12}), "depth must be 8, 16 or 32 bits"),
        (json!({"width": 4, "height": 4, "depth": 0}), "depth must be 8, 16 or 32 bits"),
    ] {
        let msg = refused(&mut s, &path, fields.clone());
        assert!(msg.contains("\"notes.txt\" could not be opened as RAW"), "{fields}: {msg}");
        assert!(msg.contains(says), "{fields}: {msg}");
    }
}

#[test]
fn layouts_whose_result_was_not_measured_are_refused_even_when_they_fit() {
    // 16 bytes fit 2 × 4 at 16 bits, 2 × 2 with 4 channels and 2 × 2 at 32 bits exactly.
    let path = notes_file("unmeasured", NOTES);
    let mut s = crate::session();
    for fields in [
        json!({"width": 2, "height": 4, "depth": 16, "byteOrder": "ibm"}),
        json!({"width": 2, "height": 2, "channels": 4}),
        json!({"width": 2, "height": 2, "depth": 32}),
        json!({"width": 2, "height": 4, "channels": 2, "interleaved": false}),
    ] {
        let msg = refused(&mut s, &path, fields.clone());
        assert!(msg.contains("only 1 channel at 8 bits can be read so far"), "{fields}: {msg}");
    }
}

#[test]
fn malformed_raw_options_are_parameter_errors() {
    let path = notes_file("params", NOTES);
    let mut s = crate::session();
    for (fields, says) in [
        (json!({"height": 4}), "Photoshop Raw needs \"width\""),
        (json!({"width": 4}), "Photoshop Raw needs \"height\""),
        (json!({"header": 0}), "Photoshop Raw needs \"width\""),
        (json!({"width": "4", "height": 4}), "expected a whole number for \"width\""),
        (json!({"width": -4, "height": 4}), "expected a whole number for \"width\""),
        (json!({"width": 4.5, "height": 4}), "expected a whole number for \"width\""),
        (json!({"width": 4, "height": 4, "header": -1}), "expected a whole number for \"header\""),
        (json!({"width": 4_294_967_296u64, "height": 4}), "\"width\" is too large"),
        (json!({"width": 4, "height": 4, "byteOrder": "motorola"}), "expected \"mac\" or \"ibm\" for \"byteOrder\""),
        (json!({"width": 4, "height": 4, "interleaved": "yes"}), "expected boolean for \"interleaved\""),
    ] {
        match open_raw(&mut s, &path, fields.clone()) {
            Err(EngineError::BadParams { cmd, msg }) => {
                assert_eq!(cmd, "file.openAs");
                assert!(msg.contains(says), "{fields}: {msg}");
            }
            other => panic!("{fields}: expected BadParams, got {other:?}"),
        }
        assert_eq!(s.documents().len(), 0);
    }
}

#[test]
fn raw_options_fields_only_apply_to_the_raw_type() {
    let path = notes_file("other-type", NOTES);
    let mut s = crate::session();
    // A bare `raw` is still the camera raw type: a text file isn't one.
    let bare = s.execute("file.openAs", json!({"path": path, "as": "raw"})).unwrap_err().to_string();
    assert!(bare.contains("could not be opened as RAW"), "{bare}");
    assert!(!bare.contains("Photoshop Raw needs"), "{bare}");
    // With another type the fields are ignored and the file is decoded as that type.
    let png = s.execute("file.openAs", json!({"path": path, "as": "png", "width": 4, "height": 4})).unwrap_err().to_string();
    assert!(png.contains("could not be opened as PNG"), "{png}");
    assert_eq!(s.documents().len(), 0);
    // `.RAW` in any case is the raw type.
    s.execute("file.openAs", json!({"path": path, "as": ".RAW", "width": 4, "height": 4})).unwrap();
    assert_eq!(gray_bytes(&s), NOTES);
}

#[test]
fn raw_options_start_at_a_square_guess_one_channel_8_bits_and_no_header() {
    let measured = RawLayout { width: 4, height: 4, channels: 1, depth: 8, byte_order: ByteOrder::Mac, interleaved: true, header: 0 };
    assert_eq!(RawLayout::defaults_for(16), measured);
    assert_eq!(RawLayout::defaults_for(16).check(16).ok(), Some(()));
    for (len, side) in [(1, 1), (9, 3), (1 << 20, 1 << 10), (90_000_000_000, 300_000)] {
        assert_eq!(photocraft_io::photoshop_raw::guess_size(len), Some((side, side)), "{len}");
    }
    // Sizes that make no square within the largest side start empty for the user to fill.
    for len in [0, 2, 15, 17, 90_000_600_001, u64::MAX] {
        assert_eq!(photocraft_io::photoshop_raw::guess_size(len), None, "{len}");
        assert_eq!((RawLayout::defaults_for(len).width, RawLayout::defaults_for(len).height), (0, 0), "{len}");
    }
}

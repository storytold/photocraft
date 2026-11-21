//! #14048: File › Open As decodes with the chosen format only. Bytes of another format give an error
//! naming the file and the chosen format, and no document opens. Bytes that match the chosen
//! format open whatever their name; Open without a format keeps detecting the content.

use serde_json::json;

use super::open::open_bytes_as;
use crate::Session;

/// The bytes of a small document saved as `ext`, with a second layer for a layered format.
fn bytes(ext: &str) -> Vec<u8> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 6, "height": 5, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
    photocraft_io::export(&s.active().unwrap().doc, &format!("sample.{ext}"), &Default::default()).unwrap().bytes
}

fn documents(s: &Session) -> usize {
    s.documents().len()
}

fn refused(name: &str, data: &[u8], as_ext: &str, want: &str) {
    let mut s = Session::new();
    let err = open_bytes_as(&mut s, name, data, Some(as_ext), None).unwrap_err();
    assert_eq!(err.to_string(), want, "{name} as {as_ext}");
    assert_eq!(documents(&s), 0, "{name} as {as_ext}: no document");
}

fn opens(name: &str, data: &[u8], as_ext: Option<&str>) -> usize {
    let mut s = Session::new();
    open_bytes_as(&mut s, name, data, as_ext, None).unwrap_or_else(|e| panic!("{name} as {as_ext:?}: {e}"));
    assert_eq!(documents(&s), 1);
    s.active().unwrap().doc.layers.len()
}

#[test]
fn png_bytes_opened_as_jpeg_are_refused() {
    refused("lake-named.jpg", &bytes("png"), "jpg", "Could not open \"lake-named.jpg\" because it is not a valid JPEG file.");
}

#[test]
fn jpeg_bytes_opened_as_png_are_refused() {
    refused("pier.jpg", &bytes("jpg"), "png", "Could not open \"pier.jpg\" because it is not a valid PNG file.");
}

#[test]
fn png_bytes_opened_as_the_layered_format_are_refused_as_before() {
    let mut s = Session::new();
    assert!(open_bytes_as(&mut s, "pier.png", &bytes("png"), Some("psd"), None).is_err());
    assert_eq!(documents(&s), 0);
}

#[test]
fn bytes_that_match_the_chosen_format_open_whatever_their_name() {
    assert_eq!(opens("layers-named.png", &bytes("psd"), Some("psd")), 2, "the layered document, both layers");
    opens("jpeg-data.dat", &bytes("jpg"), Some("jpg"));
    opens("png-named.jpg", &bytes("png"), Some("png"));
    opens("noext", &bytes("png"), Some("png"));
}

#[test]
fn open_without_a_format_keeps_detecting_the_content() {
    opens("png-named.jpg", &bytes("png"), None);
    opens("jpeg-named.png", &bytes("jpg"), None);
}

//! #14030: `file.new` makes Bitmap documents at 1 bit only and without a Transparent background. The dialog side
//! (Bitmap first in Color Mode, 1 bit only, no Transparent) is the framework's New Document; Create with Bitmap is
//! tested in `dialogs/new_doc_ui/tests.rs`.

use photocraft_color::ColorMode;
use serde_json::json;

#[test]
fn file_new_takes_bitmap_and_refuses_a_transparent_one() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "mode": "bitmap"})).unwrap();
    assert_eq!(s.active().unwrap().doc.mode, ColorMode::Bitmap);

    let mut s = photocraft_engine::Session::new();
    let e = s.execute("file.new", json!({"width": 4, "height": 4, "mode": "bitmap", "background": "transparent"})).unwrap_err().to_string();
    assert!(e.contains("transparent"), "{e}");
    assert!(s.active().is_none(), "no document was made");
}

#[test]
fn file_new_refuses_depths_that_do_not_suit_the_mode() {
    for (mode, depth) in [("bitmap", 8), ("bitmap", 16), ("bitmap", 32), ("rgb", 1), ("gray", 1)] {
        let mut s = photocraft_engine::Session::new();
        assert!(s.execute("file.new", json!({"width": 4, "height": 4, "mode": mode, "depth": depth})).is_err(), "{mode} at {depth} bits");
        assert!(s.active().is_none(), "{mode} at {depth} bits");
    }
    let mut s = photocraft_engine::Session::new();
    assert!(s.execute("file.new", json!({"width": 4, "height": 4, "depth": 1})).is_err(), "RGB (the default) is never 1 bit");
}

#[test]
fn file_new_documents_bitmap() {
    let spec = photocraft_engine::commands::find("file.new").expect("registered");
    assert!(spec.params.contains("\"mode\":\"bitmap|"), "{}", spec.params);
}

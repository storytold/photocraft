//! #14263: Edit > Convert to Profile is greyed for a Bitmap document, and running it anyway is
//! refused with the mode named, leaving the document and its history as they were. The other
//! modes keep it.

use super::*;

fn bitmap() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    s.execute("image.mode.grayscale", json!({})).unwrap();
    s.execute("image.mode.bitmap", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.mode, ColorMode::Bitmap);
    s
}

#[test]
fn convert_to_profile_is_greyed_and_refused_for_bitmap() {
    let mut s = bitmap();
    assert!(!s.is_enabled("edit.convertToProfile"));
    assert!(s.disabled_reason("edit.convertToProfile").is_some_and(|why| why.contains("Bitmap")));
    let (doc, past) = (s.active().unwrap().doc.clone(), s.active().unwrap().history.past_len());
    let err = s.execute("edit.convertToProfile", json!({"profile": "sgray"})).unwrap_err().to_string();
    assert!(err.contains("Bitmap"), "{err}");
    assert_eq!(s.active().unwrap().doc, doc, "unchanged");
    assert_eq!(s.active().unwrap().history.past_len(), past, "no history state");
    // The command itself refuses too, for a caller that skips the enabled check.
    let err = convert_to_profile(&mut s, &json!({"profile": "sgray"})).unwrap_err().to_string();
    assert!(err.contains("Bitmap"), "{err}");
}

#[test]
fn convert_to_profile_stays_enabled_for_the_other_modes() {
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("cmyk", 8), ("grayscale", 16), ("lab", 8), ("indexedColor", 8)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth, "background": "white"})).unwrap();
        if mode != "rgb" {
            s.execute(&format!("image.mode.{mode}"), json!({})).unwrap();
        }
        assert!(s.is_enabled("edit.convertToProfile"), "{mode} {depth}-bit");
    }
    assert!(!Session::new().is_enabled("edit.convertToProfile"), "no document");
}

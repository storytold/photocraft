//! Layer › New › Layer from Background per colour mode (#16762).
//!
//! The reference greys the command in Indexed Color documents, and a scripted conversion fails
//! there and in Bitmap and Multichannel documents; the Background stays. RGB, Grayscale, CMYK and
//! Lab convert it to a normal layer.

use super::*;
use photocraft_color::ColorMode;

const CMD: &str = "layer.new.layerFromBackground";

/// A white document with a Background, of `mode` (as `file.new` names it) at `depth` bits.
fn session(mode: &str, depth: u32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 8, "mode": mode, "depth": depth, "background": "white"})).unwrap();
    assert!(s.active().unwrap().doc.background().is_some(), "{mode} {depth}-bit has a Background");
    s
}

/// Greyed, and a scripted run, also as an action step with `ifPresent`, keeps the Background.
fn assert_refused(s: &mut Session, what: &str) {
    assert!(!s.is_enabled(CMD), "{what}: Layer from Background is greyed");
    for p in [json!({}), json!({"ifPresent": true, "select": false})] {
        assert!(s.execute(CMD, p.clone()).is_err(), "{what}: {p} is refused");
        assert!(s.active().unwrap().doc.background().is_some(), "{what}: {p} keeps the Background");
    }
}

#[test]
fn indexed_keeps_its_background() {
    let mut s = session("rgb", 8);
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none", "transparency": false})).unwrap();
    assert_eq!(s.active().unwrap().doc.mode, ColorMode::Indexed);
    assert_refused(&mut s, "Indexed");
}

#[test]
fn bitmap_keeps_its_background() {
    let mut s = session("gray", 8);
    s.execute("image.mode.bitmap", json!({"method": "threshold"})).unwrap();
    assert_eq!(s.active().unwrap().doc.mode, ColorMode::Bitmap);
    assert_refused(&mut s, "Bitmap");
}

#[test]
fn multichannel_keeps_its_background() {
    // Image › Mode › Multichannel leaves no Background here, so the mode is set on a document
    // that has one, as a Multichannel file with a Background opens.
    let mut s = session("rgb", 8);
    s.edit("mode", |doc, _| {
        doc.mode = ColorMode::Multichannel;
        Ok(())
    })
    .unwrap();
    assert_refused(&mut s, "Multichannel");
}

#[test]
fn rgb_grayscale_cmyk_and_lab_still_convert() {
    for (mode, depths) in [("rgb", &[8, 16, 32][..]), ("gray", &[8, 16, 32]), ("cmyk", &[8, 16]), ("lab", &[8, 16])] {
        for &depth in depths {
            let mut s = session(mode, depth);
            assert!(s.is_enabled(CMD), "{mode} {depth}-bit");
            s.execute(CMD, json!({})).unwrap();
            assert!(s.active().unwrap().doc.background().is_none(), "{mode} {depth}-bit: converted");
        }
    }
}

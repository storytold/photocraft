//! Image > Adjustments > HDR Toning is greyed with an active selection and in CMYK, Lab and
//! Indexed Color documents (#14735), stays available in RGB at 8, 16 and 32 bit and Grayscale 8
//! bit (keeping the mode and depth), and Duotone, Multichannel and deeper Grayscale (not measured)
//! keep today's behaviour.

use crate::Session;
use serde_json::json;

const HDR: &str = "image.adjustments.hdrToning";

fn new_doc(mode: &str, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "mode": mode, "depth": depth, "background": "white"})).unwrap();
    s
}

fn assert_refused(mut s: Session, what: &str) {
    assert!(!s.is_enabled(HDR), "{what}: the menu entry is greyed out");
    let d = s.active().unwrap();
    let (steps, selection, mode) = (d.history.past_len(), d.doc.selection.clone(), d.doc.mode);
    assert!(s.execute(HDR, json!({})).is_err(), "{what}: running it is refused");
    let d = s.active().unwrap();
    assert_eq!(d.history.past_len(), steps, "{what}: no history state");
    assert_eq!(d.doc.selection, selection, "{what}: the selection stays");
    assert_eq!(d.doc.mode, mode, "{what}: the mode stays");
}

#[test]
fn an_active_selection_refuses_hdr_toning() {
    let mut s = new_doc("rgb", 8);
    s.execute("select.all", json!({})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_some());
    assert_refused(s, "RGB with a selection");
}

#[test]
fn cmyk_lab_and_indexed_documents_refuse_hdr_toning() {
    assert_refused(new_doc("cmyk", 8), "CMYK 8 bit");
    assert_refused(new_doc("lab", 8), "Lab 8 bit");
    let mut indexed = new_doc("rgb", 8);
    indexed.execute("image.mode.indexedColor", json!({"palette": "exact", "dither": "none", "forced": "none"})).unwrap();
    assert_refused(indexed, "Indexed Color");
}

#[test]
fn rgb_all_depths_and_grayscale_8_bit_keep_hdr_toning_and_their_mode_and_depth() {
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("grayscale", 8)] {
        let mut s = new_doc(mode, depth);
        assert!(s.is_enabled(HDR), "{mode} {depth} bit");
        let d = s.active().unwrap();
        let (steps, before_mode, before_depth) = (d.history.past_len(), d.doc.mode, d.doc.depth);
        s.execute(HDR, json!({})).unwrap();
        let d = s.active().unwrap();
        assert_eq!(d.history.past_len(), steps + 1, "{mode} {depth} bit: one history state");
        assert_eq!((d.doc.mode, d.doc.depth), (before_mode, before_depth), "{mode} {depth} bit keeps its mode and depth");
    }
}

/// Not measured: Multichannel, Duotone and Grayscale at 16 and 32 bit keep today's behaviour.
#[test]
fn unmeasured_documents_keep_todays_behaviour() {
    let mut multi = new_doc("rgb", 8);
    multi.execute("image.mode.multichannel", json!({})).unwrap();
    assert!(multi.is_enabled(HDR), "Multichannel");

    let mut duotone = new_doc("grayscale", 8);
    duotone
        .edit("duotone", |d, _| {
            d.mode = photocraft_color::ColorMode::Duotone;
            Ok(())
        })
        .unwrap();
    assert!(duotone.is_enabled(HDR), "Duotone");

    for depth in [16, 32] {
        assert!(new_doc("grayscale", depth).is_enabled(HDR), "Grayscale {depth} bit");
    }
}

#[test]
fn the_refusals_say_why() {
    let why = super::hdr_toning_enabled(&new_doc("lab", 8)).unwrap_err();
    assert!(why.contains("Lab Color"), "{why}");
    let mut s = new_doc("rgb", 8);
    s.execute("select.all", json!({})).unwrap();
    let why = super::hdr_toning_enabled(&s).unwrap_err();
    assert!(why.contains("deselect"), "{why}");
    // A CMYK document with a selection names the mode: deselecting would not help.
    let mut s = new_doc("cmyk", 8);
    s.execute("select.all", json!({})).unwrap();
    let why = super::hdr_toning_enabled(&s).unwrap_err();
    assert!(why.contains("CMYK Color"), "{why}");
}

#[test]
fn no_document_is_unchanged() {
    let mut s = Session::new();
    assert!(!s.is_enabled(HDR));
    assert_eq!(super::hdr_toning_enabled(&s).unwrap_err(), "no document open");
    assert!(s.execute(HDR, json!({})).is_err());
}

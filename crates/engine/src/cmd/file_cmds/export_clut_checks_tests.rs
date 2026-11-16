//! #13743: File › Export › Color Lookup Tables is enabled whenever a document is open and checks
//! the document first, with the reference application's messages, in a stable order: colour mode,
//! Background, adjustment layers (hidden ones count). A refused document writes nothing.

use super::*;
use serde_json::json;

const MODE: &str = "Could not export Color Lookup Tables because only RGB, LAB, and CMYK color modes are supported.";
const NO_BACKGROUND: &str = "Could not export Color Lookup Tables because this document has no background.";
const NO_ADJUSTMENT: &str = "Could not export Color Lookup Tables because this document has no adjustment layers.";
const CMD: &str = "file.export.colorLookupTables";

fn doc(mode: &str, background: &str, invert: bool) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 10, "mode": mode, "background": background})).unwrap();
    if invert {
        s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    }
    s
}

/// The command's error for `s`, checking that it wrote nothing and left the document alone.
fn refused(mut s: Session) -> String {
    let path = std::env::temp_dir().join(format!("pc-clut-check-{}-{}.cube", std::process::id(), s.active().unwrap().doc.layers.len()));
    let _ = std::fs::remove_file(&path);
    let before = s.active().unwrap().doc.clone();
    let err = s.execute(CMD, json!({"size": 7, "path": path.to_string_lossy()})).unwrap_err().to_string();
    assert!(!path.exists(), "nothing written");
    assert_eq!(s.active().unwrap().doc, before, "the document is unchanged");
    err
}

#[test]
fn the_menu_entry_is_enabled_whenever_a_document_is_open() {
    let mut s = Session::new();
    assert!(!s.is_enabled(CMD), "greyed with no document");
    s.execute("file.new", json!({"width": 20, "height": 10})).unwrap();
    assert!(s.is_enabled(CMD), "enabled for any open document, even without an adjustment layer");
}

#[test]
fn each_refusal_has_the_reference_message() {
    assert_eq!(refused(doc("grayscale", "white", true)), MODE);
    assert_eq!(refused(doc("rgb", "transparent", true)), NO_BACKGROUND);
    assert_eq!(refused(doc("rgb", "white", false)), NO_ADJUSTMENT);
    assert_eq!(super::super::color_lookup_check(&doc("lab", "white", true).active().unwrap().doc), Ok(()), "Lab passes the checks");
    assert_eq!(super::super::color_lookup_check(&doc("cmyk", "white", true).active().unwrap().doc), Ok(()), "CMYK passes the checks");
}

#[test]
fn several_failures_report_mode_then_background_then_adjustments() {
    assert_eq!(refused(doc("grayscale", "transparent", false)), MODE);
    assert_eq!(refused(doc("rgb", "transparent", false)), NO_BACKGROUND);
}

#[test]
fn a_hidden_adjustment_layer_counts_and_the_export_runs() {
    let mut s = doc("rgb", "white", true);
    let id = s.active().unwrap().active_layer.unwrap();
    {
        let st = s.active_mut().unwrap();
        std::sync::Arc::make_mut(&mut st.doc).layer_mut(id).unwrap().visible = false;
    }
    assert!(!s.active().unwrap().doc.layer(id).unwrap().visible);
    let r = s.execute(CMD, json!({"size": 3})).unwrap();
    assert!(r["cube"].as_str().unwrap().contains("0.500000 0.500000 0.500000"), "the identity table");
}

/// CodeRabbit on #15046: a CMYK or Lab document whose only adjustment layer is hidden exports the identity table, as an
/// RGB one does; only an explicit list or selection with nothing visible to export is refused.
#[test]
fn a_hidden_only_adjustment_stack_exports_the_identity_in_cmyk_and_lab() {
    for mode in ["cmyk", "lab"] {
        let mut s = doc(mode, "white", true);
        let id = s.active().unwrap().active_layer.unwrap();
        {
            let st = s.active_mut().unwrap();
            std::sync::Arc::make_mut(&mut st.doc).layer_mut(id).unwrap().visible = false;
        }
        let r = s.execute(CMD, json!({"size": 3})).unwrap_or_else(|e| panic!("{mode}: {e}"));
        assert!(r["cube"].as_str().unwrap().contains("0.500000 0.500000 0.500000"), "{mode}: the identity table");
        assert_eq!(r["layerCount"], json!(0), "{mode}");
        let e = s.execute(CMD, json!({"size": 3, "layers": [id.0]})).unwrap_err().to_string();
        assert!(e.contains("hidden") || e.contains("no visible"), "{mode}: an explicit hidden layer is still refused: {e}");
    }
}

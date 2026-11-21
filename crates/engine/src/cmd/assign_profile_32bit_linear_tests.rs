//! #14239: assigning a profile to a 32-bit document tags it with the linear variant of that profile (same primaries
//! and white point, linear tone curves, "(Linear RGB Profile)" / "(Linear Grayscale Profile)" after its name); the
//! pixels don't change. A linear profile is kept as it is, 8- and 16-bit documents keep the profile unchanged, and a
//! profile with only lookup tables is refused with the document left alone.

use std::sync::Arc;

use craft_cms::pipeline::Stage;
use craft_cms::profile::{Lut, LutKind};
use craft_cms::{Builtin, Curve, Profile};
use serde_json::json;

use crate::Session;

/// A 24×16 document in `mode` at `depth` bits with a few distinct colours.
fn doc(mode: &str, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 24, "height": 16, "mode": mode, "depth": depth, "background": "#933a1c"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 16})).unwrap();
    s.execute("edit.fill", json!({"color": "#030c93"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn tag(s: &Session) -> Option<Arc<Vec<u8>>> {
    s.active().unwrap().doc.icc_profile.clone()
}

fn tagged(s: &Session) -> Profile {
    Profile::parse(&tag(s).expect("a tag")).unwrap()
}

fn pixels(s: &Session) -> Vec<[f32; 4]> {
    photocraft_compose::flatten(&s.active().unwrap().doc).px.to_vec()
}

fn history(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries().to_vec()
}

fn close(a: &craft_cms::Mat3, b: &craft_cms::Mat3) -> bool {
    a.iter().flatten().zip(b.iter().flatten()).all(|(x, y)| (x - y).abs() < 1e-3)
}

#[test]
fn a_32_bit_rgb_document_gets_the_linear_variant_with_the_same_pixels() {
    let mut s = doc("rgb", 32);
    let (before, steps) = (pixels(&s), history(&s).len());
    let r = s.execute("edit.assignProfile", json!({"profile": "adobe-rgb-compat"})).unwrap();
    let source = Builtin::AdobeRgbCompat.profile();
    let got = tagged(&s);
    assert!(got.description.starts_with(&source.description) && got.description.ends_with("(Linear RGB Profile)"), "{}", got.description);
    assert_eq!(r["profile"], json!(got.description), "the reply names the stored tag");
    assert!(got.trc.as_ref().unwrap().iter().all(Curve::is_identity), "linear tone curves");
    assert!(!source.trc.as_ref().unwrap().iter().all(Curve::is_identity), "the source was not linear");
    assert!(close(got.matrix.as_ref().unwrap(), source.matrix.as_ref().unwrap()), "the same primaries");
    assert!(got.white_point.iter().zip(source.white_point).all(|(a, b)| (a - b).abs() < 1e-3), "the same white point");
    assert_eq!(pixels(&s), before, "pixels unchanged");
    assert_eq!(history(&s).len(), steps + 1);
    assert_eq!(history(&s).last().map(String::as_str), Some("Assign Profile"));
}

#[test]
fn undo_restores_the_previous_tag_in_one_step() {
    let mut s = doc("rgb", 32);
    let before = tag(&s);
    s.execute("edit.assignProfile", json!({"profile": "adobe-rgb-compat"})).unwrap();
    assert_ne!(tag(&s), before);
    s.execute("history.undo", json!({})).unwrap();
    assert_eq!(tag(&s), before);
}

#[test]
fn a_32_bit_gray_document_gets_the_linear_grayscale_variant() {
    let mut s = doc("grayscale", 32);
    s.execute("edit.assignProfile", json!({"profile": "gray-gamma-2.2"})).unwrap();
    let got = tagged(&s);
    assert!(got.description.ends_with("(Linear Grayscale Profile)"), "{}", got.description);
    assert!(got.gray_trc.as_ref().is_some_and(Curve::is_identity));
}

#[test]
fn a_linear_profile_is_kept_as_it_is() {
    let mut s = doc("rgb", 32);
    s.execute("edit.assignProfile", json!({"profile": "linear-srgb"})).unwrap();
    assert_eq!(tagged(&s).description, Builtin::LinearSrgb.description(), "no suffix added");
    s.execute("edit.assignProfile", json!({"profile": "adobe-rgb-compat"})).unwrap();
    let once = tagged(&s);
    // Assigning the stored variant again doesn't double the suffix.
    let dir = std::env::temp_dir().join(format!("pc-14239-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("variant.icc");
    std::fs::write(&path, tag(&s).unwrap().as_slice()).unwrap();
    s.execute("edit.assignProfile", json!({"profile": path.to_str().unwrap()})).unwrap();
    assert_eq!(tagged(&s).description, once.description);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn eight_and_sixteen_bit_documents_keep_the_profile_unchanged() {
    for depth in [8, 16] {
        let mut s = doc("rgb", depth);
        s.execute("edit.assignProfile", json!({"profile": "adobe-rgb-compat"})).unwrap();
        assert_eq!(tag(&s), Some(Builtin::AdobeRgbCompat.profile().to_bytes()), "{depth}-bit");
    }
}

#[test]
fn a_profile_with_only_lookup_tables_is_refused_and_nothing_changes() {
    let mut s = doc("rgb", 32);
    let curves = || Lut { kind: LutKind::Ab, inputs: 3, outputs: 3, stages: vec![Stage::Curves(vec![Curve::Identity; 3])] };
    let mut lut = Builtin::Srgb.profile().clone();
    lut.description = "Table-only RGB".into();
    (lut.matrix, lut.trc) = (None, None);
    lut.a2b = [Some(curves()), None, None];
    lut.b2a = [Some(curves()), None, None];
    let dir = std::env::temp_dir().join(format!("pc-14239-lut-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("table-only.icc");
    std::fs::write(&path, lut.with_encoded_bytes().to_bytes().as_slice()).unwrap();
    let (before, steps) = (tag(&s), history(&s).len());
    assert!(s.execute("edit.assignProfile", json!({"profile": path.to_str().unwrap()})).is_err());
    assert_eq!((tag(&s), history(&s).len()), (before, steps), "the document is not changed");
    let _ = std::fs::remove_dir_all(&dir);
}

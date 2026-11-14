//! #14261: converting a document to the profile it already has changes nothing: no history
//! state, no change to the document, its pixels or its profile, and the reply says so.

use super::*;

/// A 16×8 sRGB-tagged document with a red background.
fn tagged() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 8, "background": "#c81e1e"})).unwrap();
    s.execute("edit.assignProfile", json!({"profile": "srgb"})).unwrap();
    assert!(s.active().unwrap().doc.icc_profile.is_some(), "tagged");
    s
}

/// History length, dirty flag, embedded profile and composite pixels.
type State = (usize, bool, Option<std::sync::Arc<Vec<u8>>>, Vec<[f32; 4]>);

fn state(s: &Session) -> State {
    let d = s.active().unwrap();
    (d.history.entries().len(), d.is_dirty(), d.doc.icc_profile.clone(), photocraft_compose::flatten(&d.doc).px.to_vec())
}

#[test]
fn the_same_profile_converts_nothing() {
    let mut s = tagged();
    let before = state(&s);
    let r = s.execute("edit.convertToProfile", json!({"profile": "srgb"})).unwrap();
    assert_eq!(r["converted"], false, "{r}");
    assert_eq!(state(&s), before, "no history state, not dirtied, the same pixels and profile");
}

#[test]
fn a_different_profile_still_converts() {
    let mut s = tagged();
    let entries = s.active().unwrap().history.entries().len();
    let r = s.execute("edit.convertToProfile", json!({"profile": "adobe-rgb-compat"})).unwrap();
    assert_eq!(r["converted"], true, "{r}");
    let history = s.active().unwrap().history.entries();
    assert_eq!(history.len(), entries + 1);
    assert_eq!(history.last().map(String::as_str), Some("Convert to Profile"));
}

#[test]
fn flatten_with_the_same_profile_runs_as_before() {
    let mut s = tagged();
    let entries = s.active().unwrap().history.entries().len();
    let r = s.execute("edit.convertToProfile", json!({"profile": "srgb", "flatten": true})).unwrap();
    assert_eq!(r["converted"], true, "{r}");
    assert_eq!(s.active().unwrap().history.entries().len(), entries + 1);
}

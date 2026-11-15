//! #13804: Fade works again right after a Fade, which the new one replaces (one Fade state, always
//! blended over the pixels before the faded step), and again after undoing a Fade.

use super::*;

/// 50 x 50 white with a red square, inverted: (25,25) is 0,255,255 after Invert.
fn inverted() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 50, "height": 50})).unwrap();
    s.execute("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("edit.fill", json!({"use": "foreground"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("image.adjustments.invert", json!({})).unwrap();
    s
}

fn pixel(s: &Session) -> [u8; 3] {
    let st = s.active().unwrap();
    let p = photocraft_compose::flatten(&st.doc).into_px()[25 * 50 + 25];
    [p[0], p[1], p[2]].map(|v| (v * 255.0).round() as u8)
}

fn tail(s: &Session) -> Vec<String> {
    let e = s.active().unwrap().history.entries();
    e[e.len().saturating_sub(2)..].to_vec()
}

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 1)
}

#[test]
fn fading_again_replaces_the_earlier_fade() {
    let mut s = inverted();
    assert_eq!(pixel(&s), [0, 255, 255]);
    s.execute("edit.fade", json!({"opacity": 37, "mode": "multiply"})).unwrap();
    assert!(s.is_enabled("edit.fade"), "still available right after a Fade");
    s.execute("edit.fade", json!({"opacity": 100, "mode": "normal"})).unwrap();
    assert_eq!(pixel(&s), [0, 255, 255], "the full Invert again");
    assert_eq!(tail(&s), ["Invert", "Fade Invert"], "one Fade state");
    assert_eq!(s.active().unwrap().history.entries().iter().filter(|e| e.starts_with("Fade")).count(), 1);
}

#[test]
fn a_repeated_fade_blends_over_the_pixels_before_the_step() {
    let mut once = inverted();
    once.execute("edit.fade", json!({"opacity": 50, "mode": "normal"})).unwrap();
    let mut again = inverted();
    again.execute("edit.fade", json!({"opacity": 37, "mode": "multiply"})).unwrap();
    again.execute("edit.fade", json!({"opacity": 50, "mode": "normal"})).unwrap();
    assert_eq!(pixel(&again), pixel(&once), "not over the first Fade's result");
    assert!(near(pixel(&again), [128, 128, 128]), "{:?}", pixel(&again));
    // The dialog's live preview shows the same.
    let mut preview = inverted();
    preview.execute("edit.fade", json!({"opacity": 37, "mode": "multiply"})).unwrap();
    let doc = fade_preview(&preview, &json!({"opacity": 50, "mode": "normal"})).unwrap().render().unwrap();
    let p = photocraft_compose::flatten(&doc).into_px()[25 * 50 + 25].map(|v| (v * 255.0).round() as u8);
    assert_eq!([p[0], p[1], p[2]], pixel(&once));
}

#[test]
fn fade_is_available_again_after_undoing_a_fade() {
    let mut s = inverted();
    s.execute("edit.fade", json!({"opacity": 37, "mode": "multiply"})).unwrap();
    assert!(s.undo());
    assert_eq!(tail(&s).last().map(String::as_str), Some("Invert"));
    assert!(s.is_enabled("edit.fade"), "available again at the Invert state");
    s.execute("edit.fade", json!({"opacity": 50, "mode": "normal"})).unwrap();
    assert!(near(pixel(&s), [128, 128, 128]), "it fades Invert: {:?}", pixel(&s));
}

#[test]
fn undo_after_a_replaced_fade_returns_to_the_step() {
    let mut s = inverted();
    s.execute("edit.fade", json!({"opacity": 37, "mode": "multiply"})).unwrap();
    s.execute("edit.fade", json!({"opacity": 50, "mode": "normal"})).unwrap();
    assert!(s.undo());
    assert_eq!(tail(&s).last().map(String::as_str), Some("Invert"));
    assert_eq!(pixel(&s), [0, 255, 255]);
    // A bad parameter when fading again changes nothing.
    s.execute("edit.fade", json!({"opacity": 50})).unwrap();
    let before = pixel(&s);
    assert!(s.execute("edit.fade", json!({"mode": "nonsense"})).is_err());
    assert_eq!(pixel(&s), before);
    assert_eq!(tail(&s), ["Invert", "Fade Invert"]);
}

/// CodeRabbit on #15094: Invert, a layer hidden in History, Fade, Undo: Fade is available again, as the
/// state to come back to is the hide's row, not the Invert's.
#[test]
fn fade_is_available_after_undoing_a_fade_that_followed_a_visibility_change() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 50, "height": 50, "background": "white"})).unwrap();
    let bg = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("image.adjustments.invert", json!({})).unwrap();
    s.execute("layer.hideLayers", json!({"layer": bg.0})).unwrap();
    assert!(s.is_enabled("edit.fade"), "still available after hiding another layer");
    s.execute("edit.fade", json!({"opacity": 40})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().history.entries().last().map(String::as_str), Some("Hide Layer"), "the Undo returned to the hide's row");
    assert!(s.is_enabled("edit.fade"), "Fade is available again after undoing the Fade");
    s.execute("edit.fade", json!({"opacity": 40})).unwrap();
    assert_eq!(s.active().unwrap().history.entries().iter().filter(|e| e.starts_with("Fade")).count(), 1);
}

/// CodeRabbit on #15094: a Fade that fails while replacing an earlier one leaves that Fade as it was. The
/// document, the History and Fade itself are untouched, and fading again works.
#[test]
fn a_failed_second_fade_keeps_the_first() {
    let mut s = inverted();
    s.execute("edit.fade", json!({"opacity": 40, "mode": "normal"})).unwrap();
    let (doc, history) = (s.active().unwrap().doc.clone(), s.active().unwrap().history.entries());
    crate::allocation::fail_next_for_test();
    let result = s.execute("edit.fade", json!({"opacity": 70, "mode": "normal"}));
    let _ = crate::allocation::capacity::<u8>(0, "reset failpoint");
    assert!(result.is_err());
    assert_eq!(*s.active().unwrap().doc, *doc, "the first Fade's result is still there");
    assert_eq!(s.active().unwrap().history.entries(), history, "and its History step, with no redo lost");
    assert!(s.is_enabled("edit.fade"));
    s.execute("edit.fade", json!({"opacity": 70, "mode": "normal"})).unwrap();
    assert_eq!(s.active().unwrap().history.entries().iter().filter(|e| e.starts_with("Fade")).count(), 1);
}

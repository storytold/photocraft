//! #14159: Render Video names an image sequence as the reference application does:
//! `<name><number>.<ext>` with no separator, the number starting at Starting # for the first frame
//! and zero-padded to Digits (wider numbers in full), and a single frame without a number. The
//! animated GIF keeps `<name>.gif`, and a name can't reach outside the folder.

use serde_json::{Value, json};

use super::render_video_with;
use crate::Session;

/// A document named `name` with a timeline of `frames` frames (none when `frames` is 0).
fn session(name: &str, frames: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"name": name, "width": 6, "height": 4, "background": "white"})).unwrap();
    if frames > 0 {
        s.execute("timeline.create", json!({"duration": frames, "fps": 10})).unwrap();
    }
    s
}

/// The file names a render with `params` writes, through the caller-owned writer.
fn names(s: &Session, params: Value) -> crate::Result<Vec<String>> {
    let mut out = Vec::new();
    render_video_with(
        s,
        &params,
        |name, _| {
            out.push(name.to_string());
            Ok(())
        },
        |_, _| true,
    )?;
    Ok(out)
}

#[test]
fn name_starting_number_and_digits_name_the_sequence() {
    let s = session("rv-test", 5);
    let got = names(&s, json!({"format": "png", "name": "seq", "startNumber": 7, "digits": 2})).unwrap();
    assert_eq!(got, ["seq07.png", "seq08.png", "seq09.png", "seq10.png", "seq11.png"]);
}

#[test]
fn the_defaults_are_the_document_name_from_0_with_4_digits() {
    let s = session("rv-test.psd", 3);
    let got = names(&s, json!({"format": "png"})).unwrap();
    assert_eq!(got, ["rv-test0000.png", "rv-test0001.png", "rv-test0002.png"]);
    // A `null` counts as unset, as it does for `format` and the engine's other params.
    let got = names(&s, json!({"format": "png", "name": null, "startNumber": null, "digits": null})).unwrap();
    assert_eq!(got, ["rv-test0000.png", "rv-test0001.png", "rv-test0002.png"]);
}

#[test]
fn numbers_wider_than_digits_are_written_in_full() {
    let s = session("File", 2);
    let got = names(&s, json!({"format": "png", "startNumber": 10000, "digits": 4})).unwrap();
    assert_eq!(got, ["File10000.png", "File10001.png"]);
}

#[test]
fn a_single_frame_has_no_number() {
    // A document without a timeline renders one frame.
    let s = session("rv-test", 0);
    assert_eq!(names(&s, json!({"format": "png", "name": "alpha"})).unwrap(), ["alpha.png"]);
    assert_eq!(names(&s, json!({"format": "png"})).unwrap(), ["rv-test.png"]);
    let s = session("rv-test", 1);
    assert_eq!(names(&s, json!({"format": "png", "name": "alpha", "startNumber": 3})).unwrap(), ["alpha.png"]);
}

#[test]
fn the_animated_gif_keeps_its_name() {
    let s = session("rv-test", 3);
    assert_eq!(names(&s, json!({"format": "gif"})).unwrap(), ["rv-test.gif"]);
    assert_eq!(names(&s, json!({"format": "gif", "name": "loop"})).unwrap(), ["loop.gif"]);
}

#[test]
fn a_name_cannot_reach_outside_the_folder() {
    let s = session("rv-test", 2);
    for (hostile, want) in [("../../outside/seq", "seq"), ("C:\\x\\seq", "seq"), ("..", "video"), ("a/..", "video")] {
        let got = names(&s, json!({"format": "png", "name": hostile, "digits": 1})).unwrap();
        assert_eq!(got, [format!("{want}0.png"), format!("{want}1.png")], "{hostile:?}");
        assert!(got.iter().all(|n| !n.contains(['/', '\\'])));
    }
    // A blank name falls back to the document name.
    assert_eq!(names(&s, json!({"format": "png", "name": "  ", "digits": 1})).unwrap(), ["rv-test0.png", "rv-test1.png"]);
}

#[test]
fn bad_numbers_are_errors_not_panics() {
    let s = session("rv-test", 2);
    for bad in [
        json!({"startNumber": 10001}),
        json!({"startNumber": -1}),
        json!({"startNumber": 1.5}),
        json!({"startNumber": "7"}),
        json!({"digits": 0}),
        json!({"digits": 6}),
        json!({"digits": true}),
        json!({"name": 5}),
    ] {
        let mut p = bad.clone();
        p["format"] = json!("png");
        assert!(names(&s, p).is_err(), "{bad} is refused");
    }
    // A whole number given as a float is fine.
    assert_eq!(names(&s, json!({"format": "png", "startNumber": 7.0, "digits": 2.0})).unwrap(), ["rv-test07.png", "rv-test08.png"]);
}

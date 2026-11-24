//! #14164: Render Video scales the output to another size and samples the timeline at another
//! frame rate.

use super::*;
use serde_json::json;

/// An 8 × 6 document with a timeline of `frames` frames at 30 fps.
fn session(frames: usize) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 6, "name": "rv", "background": "white"})).unwrap();
    s.execute("timeline.create", json!({"duration": frames, "fps": 30})).unwrap();
    s
}

fn render(s: &Session, params: Value) -> Result<Vec<(String, Vec<u8>)>> {
    let mut written = Vec::new();
    render_video_with(
        s,
        &params,
        |name, bytes| {
            written.push((name.to_owned(), bytes.to_vec()));
            Ok(())
        },
        |_, _| true,
    )?;
    Ok(written)
}

/// The size of a written PNG.
fn png_size(bytes: &[u8]) -> (u32, u32) {
    let doc = photocraft_io::import("f.png", bytes).unwrap().document;
    (doc.size.width, doc.size.height)
}

#[test]
fn another_rate_samples_the_same_span_of_time() {
    let s = session(150);
    assert_eq!(render(&s, json!({"format": "png", "fps": 15})).unwrap().len(), 75, "150 frames at 30 fps are 75 at 15");
    assert_eq!(render(&s, json!({"format": "png", "fps": 60})).unwrap().len(), 300);
    assert_eq!(render(&s, json!({"format": "png", "fps": 12.5, "range": {"start": 0, "end": 11}})).unwrap().len(), 5, "12 frames are 0.4 s");
}

#[test]
fn the_plan_shows_the_frame_on_screen_at_each_output_time() {
    let plan = render_plan(&json!({"fps": 15}), 10..=19, 30.0, (8, 6)).unwrap();
    assert_eq!(plan.frames, [10, 12, 14, 16, 18], "every other frame at half the rate");
    let plan = render_plan(&json!({"fps": 60}), 0..=2, 30.0, (8, 6)).unwrap();
    assert_eq!(plan.frames, [0, 0, 1, 1, 2, 2], "each frame twice at double the rate");
    let plan = render_plan(&json!({}), 3..=5, 30.0, (8, 6)).unwrap();
    assert_eq!(plan.frames, [3, 4, 5], "the document's rate keeps the frames");
}

#[test]
fn another_size_resamples_every_frame_to_exactly_that_size() {
    let s = session(3);
    let files = render(&s, json!({"format": "png", "width": 17, "height": 5})).unwrap();
    assert_eq!(files.len(), 3);
    assert!(files.iter().all(|(_, bytes)| png_size(bytes) == (17, 5)), "every frame is 17 × 5");
    let gif = render(&s, json!({"format": "gif", "width": 4, "height": 3})).unwrap();
    let mut sizes = Vec::new();
    photocraft_io::import_gif_frames("rv.gif", &gif[0].1, &mut |doc, _| {
        sizes.push((doc.size.width, doc.size.height));
        Ok(())
    })
    .unwrap();
    assert_eq!(sizes, [(4, 3); 3]);
}

#[test]
fn without_size_or_rate_the_output_is_as_before() {
    let s = session(4);
    let plain = render(&s, json!({"format": "png"})).unwrap();
    let same = render(&s, json!({"format": "png", "width": 8, "height": 6, "fps": 30})).unwrap();
    assert_eq!(plain, same, "the document's own size and rate change nothing");
    assert!(plain.iter().all(|(_, bytes)| png_size(bytes) == (8, 6)));
}

#[test]
fn bad_sizes_and_rates_are_refused() {
    let s = session(4);
    for (params, says) in [
        (json!({"width": 0}), "An integer between 1 and 5000 is required."),
        (json!({"height": 5001}), "An integer between 1 and 5000 is required."),
        (json!({"width": "8"}), "An integer between 1 and 5000 is required."),
        (json!({"width": 7.5}), "An integer between 1 and 5000 is required."),
        (json!({"fps": 0}), "A number between 1.000 and 5000.000 is required."),
        (json!({"fps": 5000.5}), "A number between 1.000 and 5000.000 is required."),
        (json!({"fps": "fast"}), "A number between 1.000 and 5000.000 is required."),
    ] {
        let mut p = params.clone();
        p["format"] = json!("png");
        let err = render(&s, p).expect_err(&format!("{params} is refused")).to_string();
        assert!(err.contains(says), "{params}: {err}");
    }
}

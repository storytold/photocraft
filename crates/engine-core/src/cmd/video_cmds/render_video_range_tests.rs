//! #14162: Render Video renders a frame range or the timeline's work area, not only every frame.

use super::*;
use serde_json::json;

/// A small document with a 150-frame timeline at 30 fps.
fn session(frames: Option<usize>) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 6, "name": "rv"})).unwrap();
    if let Some(duration) = frames {
        s.execute("timeline.create", json!({"duration": duration, "fps": 30})).unwrap();
    }
    s
}

/// Renders with `params`, keeping the files in memory: their names and contents.
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

fn names(files: &[(String, Vec<u8>)]) -> Vec<&str> {
    files.iter().map(|(n, _)| n.as_str()).collect()
}

/// The grey frame `n` is painted with in [`distinct_frames`]: different for every frame under 200.
pub(super) fn grey(n: usize) -> f32 {
    ((n * 37) % 200 + 20) as f32 / 255.0
}

/// A document whose video layer shows its own grey in each of `frames` frames ([`grey`]), so a
/// rendered file says which frame it is.
pub(super) fn distinct_frames(frames: usize) -> Session {
    let mut s = session(Some(frames));
    s.execute("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    for n in 0..frames {
        s.execute("timeline.setFrame", json!({"frame": n})).unwrap();
        s.edit("Paint", |doc, _| {
            if let Some(Layer { content: LayerContent::Raster(r), .. }) = doc.layer_mut(id) {
                r.fill_rect(photocraft_geom::Rect::from_xywh(0, 0, 8, 6), &[grey(n), grey(n), grey(n), 1.0]);
            }
            Ok(())
        })
        .unwrap();
    }
    s
}

/// The grey at the middle of a rendered still (PNG).
pub(super) fn grey_of_png(bytes: &[u8]) -> f32 {
    let doc = photocraft_io::import("f.png", bytes).unwrap().document;
    photocraft_compose::flatten_to_surface(&doc, doc.pixel_format(), None).pixel(4, 3)[0]
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1.5 / 255.0
}

#[test]
fn a_start_and_end_render_only_those_frames_numbered_from_the_first() {
    let s = distinct_frames(30);
    let files = render(&s, json!({"format": "png", "range": {"start": 20, "end": 24}})).unwrap();
    assert_eq!(names(&files), ["rv_0000.png", "rv_0001.png", "rv_0002.png", "rv_0003.png", "rv_0004.png"]);
    for (i, (name, bytes)) in files.iter().enumerate() {
        assert!(close(grey_of_png(bytes), grey(20 + i)), "{name} is frame {}", 20 + i);
    }
}

#[test]
fn a_range_limits_an_animated_gif_too() {
    let s = distinct_frames(30);
    let files = render(&s, json!({"format": "gif", "range": {"start": 5, "end": 14}})).unwrap();
    assert_eq!(files.len(), 1);
    let mut greys = Vec::new();
    photocraft_io::import_gif_frames("rv.gif", &files[0].1, &mut |doc, _| {
        greys.push(photocraft_compose::flatten_to_surface(&doc, doc.pixel_format(), None).pixel(4, 3)[0]);
        Ok(())
    })
    .unwrap();
    assert_eq!(greys.len(), 10);
    for (i, g) in greys.iter().enumerate() {
        assert!(close(*g, grey(5 + i)), "GIF frame {i} is timeline frame {}", 5 + i);
    }
}

#[test]
fn the_work_area_renders_its_frames_and_no_range_renders_all() {
    let mut s = session(Some(30));
    assert_eq!(render(&s, json!({"format": "png", "range": "workArea"})).unwrap().len(), 30, "by default the work area is the timeline");
    s.execute("timeline.setProps", json!({"workStart": 10, "workEnd": 16})).unwrap();
    assert_eq!(render(&s, json!({"format": "png", "range": "workArea"})).unwrap().len(), 6, "frames 10 to 15");
    assert_eq!(render(&s, json!({"format": "png"})).unwrap().len(), 30);
    assert_eq!(render(&s, json!({"format": "png", "range": "all"})).unwrap().len(), 30);
}

#[test]
fn bad_ranges_are_refused_and_write_nothing() {
    let s = session(Some(150));
    for (range, says) in [
        (json!({"start": 0, "end": 150}), "An integer between 0 and 149 is required."),
        (json!({"start": -3, "end": 4}), "An integer between 0 and 149 is required."),
        (json!({"start": 50, "end": 10}), "The range of frames is incorrect."),
        (json!({"start": "0", "end": 4}), "An integer between 0 and 149 is required."),
        (json!({"start": 0}), "An integer between 0 and 149 is required."),
        (json!({"start": 0, "end": 4, "step": 2}), "not `step`"),
        (json!("some"), "must be"),
        (json!(7), "must be"),
    ] {
        let mut wrote = false;
        let r = render_video_with(
            &s,
            &json!({"format": "png", "range": range}),
            |_, _| {
                wrote = true;
                Ok(())
            },
            |_, _| true,
        );
        let err = r.expect_err(&format!("{range} is refused")).to_string();
        assert!(err.contains(says), "{range}: {err}");
        assert!(!wrote, "{range}: nothing is written");
    }
}

#[test]
fn without_a_timeline_only_the_one_frame_renders() {
    let s = session(None);
    assert_eq!(render(&s, json!({"format": "png"})).unwrap().len(), 1);
    assert_eq!(render(&s, json!({"format": "png", "range": {"start": 0, "end": 0}})).unwrap().len(), 1);
    assert!(render(&s, json!({"format": "png", "range": "workArea"})).is_err());
    assert!(render(&s, json!({"format": "png", "range": {"start": 0, "end": 1}})).is_err());
}

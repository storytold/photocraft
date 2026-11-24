//! Layer › Frame Range against a 10-frame timeline: a layer with a range is on screen on its
//! frames only, its eye survives the playhead, and the range is saved, undone and rendered.

use super::*;
use photocraft_geom::Rect;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

/// A 16 x 12 white document with a 10-frame timeline, the playhead at `at`.
fn shot(at: u64) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 16, "height": 12, "background": "#ffffff"})).unwrap();
    s.execute("timeline.create", json!({"duration": 10, "fps": 24.0})).unwrap();
    s.execute("timeline.setFrame", json!({"frame": at})).unwrap();
    s
}

fn go(s: &mut Session, frame: u64) {
    s.execute("timeline.setFrame", json!({"frame": frame})).unwrap();
}

fn layer(s: &Session, id: LayerId) -> &Layer {
    s.active().unwrap().doc.layer(id).unwrap()
}

fn visible(s: &Session, id: LayerId) -> bool {
    layer(s, id).visible
}

fn id_of(v: &Value) -> LayerId {
    LayerId(v["layer"].as_u64().unwrap())
}

/// Paints the whole of `id` red, as a brush stroke would.
fn paint_red(s: &mut Session, id: LayerId) {
    s.edit("Paint", |doc, _| {
        if let Some(surf) = doc.layer_mut(id).and_then(Layer::surface_mut) {
            surf.fill_rect(Rect::new(0, 0, 16, 12), &RED);
        }
        Ok(())
    })
    .unwrap();
}

/// The composite's red-minus-green at the centre: high where the red paint shows, 0 on white.
fn redness(s: &Session) -> f32 {
    let doc = &s.active().unwrap().doc;
    let px = crate::file_cmds::flattened(doc, doc.pixel_format()).pixel(8, 6);
    px[0] - px[1]
}

#[test]
fn a_layer_for_one_frame_shows_on_that_frame_only() {
    let mut s = shot(4);
    let id = id_of(&s.execute("layer.frameRange.newLayerForFrame", json!({})).unwrap());
    paint_red(&mut s, id);
    assert!(redness(&s) > 0.5, "the paint shows on its frame");
    for frame in [3, 5, 9, 0] {
        go(&mut s, frame);
        assert!(!visible(&s, id), "hidden on frame {frame}");
        assert!(redness(&s) < 0.01, "the plate alone on frame {frame}");
    }
    go(&mut s, 4);
    assert!(visible(&s, id) && redness(&s) > 0.5, "back on its frame, it shows again");
    assert!(layer(&s, id).name.contains('5'), "named after the frame the panel calls 5");
}

#[test]
fn the_eye_the_user_set_survives_the_playhead() {
    let mut s = shot(2);
    let id = id_of(&s.execute("layer.frameRange.newLayerForFrame", json!({})).unwrap());
    s.execute("layer.frameRange.set", json!({"layer": id.0, "start": 2, "end": 5})).unwrap();

    // Hidden by the user on one of its frames: it stays hidden on the others.
    s.execute("layer.setProps", json!({"layer": id.0, "visible": false})).unwrap();
    go(&mut s, 7);
    go(&mut s, 3);
    assert!(!visible(&s, id), "the eye the user turned off stays off");

    // Turned back on off its frames: still hidden there, then shown on its frames.
    go(&mut s, 8);
    s.execute("layer.setProps", json!({"layer": id.0, "visible": true})).unwrap();
    assert!(!visible(&s, id), "off its frames even with the eye on");
    go(&mut s, 4);
    assert!(visible(&s, id), "the eye turned on off its frames shows it on them");
}

#[test]
fn split_hands_the_frames_from_the_playhead_to_a_copy() {
    let mut s = shot(0);
    let id = id_of(&s.execute("layer.frameRange.newLayerForFrame", json!({})).unwrap());
    paint_red(&mut s, id);
    s.execute("layer.frameRange.set", json!({"layer": id.0, "start": 0, "end": 10})).unwrap();
    go(&mut s, 6);
    let r = s.execute("layer.frameRange.split", json!({})).unwrap();
    let copy = id_of(&r["layers"][1]);
    assert_ne!(copy, id);
    assert_eq!(s.active().unwrap().active_layer, Some(copy), "the copy is the one to paint next");
    for frame in 0..10 {
        go(&mut s, frame);
        assert!(visible(&s, id) != visible(&s, copy), "exactly one of the halves shows on frame {frame}");
        assert_eq!(visible(&s, copy), frame >= 6, "the copy takes the playhead's frame onward (frame {frame})");
        assert!(redness(&s) > 0.5, "the copy has the same pixels (frame {frame})");
    }
}

#[test]
fn split_needs_the_playhead_inside_the_range() {
    let mut s = shot(3);
    s.execute("layer.frameRange.newLayerForFrame", json!({})).unwrap();
    assert!(s.execute("layer.frameRange.split", json!({})).is_err(), "a one-frame range has nothing to split");
}

#[test]
fn start_and_end_at_the_playhead_trim_the_range() {
    let mut s = shot(2);
    // A layer on every frame first.
    let id = id_of(&s.execute("layer.frameRange.newLayerForFrame", json!({})).unwrap());
    s.execute("layer.frameRange.clear", json!({"layer": id.0})).unwrap();
    s.execute("layer.frameRange.startAtPlayhead", json!({"layer": id.0})).unwrap();
    go(&mut s, 5);
    let r = s.execute("layer.frameRange.endAtPlayhead", json!({"layer": id.0})).unwrap();
    assert_eq!(r["range"], json!({"start": 2, "end": 6}));
    for (frame, shown) in [(1, false), (2, true), (5, true), (6, false)] {
        go(&mut s, frame);
        assert_eq!(visible(&s, id), shown, "frame {frame}");
    }
}

#[test]
fn bad_ranges_change_nothing() {
    let mut s = shot(1);
    let id = id_of(&s.execute("layer.frameRange.newLayerForFrame", json!({})).unwrap());
    for (start, end) in [(4, 4), (5, 2), (0, 11)] {
        assert!(s.execute("layer.frameRange.set", json!({"layer": id.0, "start": start, "end": end})).is_err(), "{start}..{end}");
    }
    assert_eq!(layer(&s, id).span.map(|sp| (sp.start, sp.end)), Some((1, 2)));
    assert!(s.execute("layer.frameRange.set", json!({"layer": id.0, "start": -1, "end": 2})).is_err());
}

#[test]
fn show_on_every_frame_and_undo() {
    let mut s = shot(1);
    let id = id_of(&s.execute("layer.frameRange.newLayerForFrame", json!({})).unwrap());
    go(&mut s, 7);
    assert!(!visible(&s, id));
    s.execute("layer.frameRange.clear", json!({"layer": id.0})).unwrap();
    assert!(visible(&s, id) && layer(&s, id).span.is_none(), "every frame again, with its eye on");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(layer(&s, id).span.is_some() && !visible(&s, id), "undo brings the range back");
}

#[test]
fn deleting_the_timeline_shows_the_layer_by_its_eye() {
    let mut s = shot(1);
    let id = id_of(&s.execute("layer.frameRange.newLayerForFrame", json!({})).unwrap());
    go(&mut s, 6);
    assert!(!visible(&s, id));
    s.execute("timeline.delete", json!({})).unwrap();
    assert!(visible(&s, id), "without a timeline a range hides nothing");
}

#[test]
fn render_video_draws_each_layer_on_its_own_frames() {
    let mut s = shot(3);
    let id = id_of(&s.execute("layer.frameRange.newLayerForFrame", json!({})).unwrap());
    paint_red(&mut s, id);
    go(&mut s, 0);
    let dir = std::env::temp_dir().join(format!("photocraft-frame-range-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    s.execute("file.export.renderVideo", json!({"dir": dir.to_str().unwrap(), "format": "png"})).unwrap();
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    assert_eq!(files.len(), 10);
    let red: Vec<bool> = files
        .iter()
        .map(|f| {
            let bytes = std::fs::read(f).unwrap();
            let doc = photocraft_io::import(&f.to_string_lossy(), &bytes).unwrap().document;
            let px = crate::file_cmds::flattened(&doc, doc.pixel_format()).pixel(8, 6);
            px[0] - px[1] > 0.5
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(red.iter().filter(|r| **r).count(), 1, "the paint is in one rendered frame: {red:?}");
    assert!(red[3], "and it is frame 3's: {red:?}");
    assert!(!visible(&s, id), "rendering leaves the playhead where it was");
}

#[test]
fn ranges_list_top_to_bottom() {
    let mut s = shot(2);
    let a = id_of(&s.execute("layer.frameRange.newLayerForFrame", json!({})).unwrap());
    go(&mut s, 5);
    let b = id_of(&s.execute("layer.frameRange.newLayerForFrame", json!({})).unwrap());
    let r = s.execute("layer.frameRange.list", json!({})).unwrap();
    let ids: Vec<u64> = r["layers"].as_array().unwrap().iter().map(|l| l["layer"].as_u64().unwrap()).collect();
    assert_eq!(ids, [b.0, a.0]);
}

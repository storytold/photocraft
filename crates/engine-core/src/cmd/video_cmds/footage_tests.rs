//! Video layers whose frames come from a linked movie file, decoded when shown (#9040).

use std::sync::atomic::Ordering;

use super::*;
use crate::footage::testing::Synthetic;
use photocraft_doc::{FootageLink, LayerId};
use photocraft_geom::Rect;
use serde_json::json;

const W: u32 = 32;
const H: u32 = 24;

/// A unique path per test, so tests running at once don't share a registry entry.
fn path(name: &str) -> String {
    format!("test-footage://{}/{name}.mp4", std::process::id())
}

/// A document the footage's size with a video layer linked to `frames` synthetic frames.
fn linked(name: &str, frames: usize) -> (Session, LayerId, Synthetic) {
    let file = path(name);
    let synthetic = Synthetic::new(W, H, frames, 24.0);
    let counter = Synthetic { info: synthetic.info, decodes: synthetic.decodes.clone() };
    let info = crate::footage::register(&file, Box::new(counter));
    let mut s = crate::session();
    s.execute("file.new", json!({"width": W, "height": H})).unwrap();
    let id = s
        .edit("New Video Layer from File", |doc, active| {
            let fmt = doc.pixel_format();
            doc.timeline = Some(Timeline::new(frames, info.fps));
            let mut l = Layer::raster("Movie", fmt);
            l.video = Some(VideoData::from_footage(FootageLink::new(file.clone(), info), fmt));
            let id = crate::layer_place::insert_new(doc, *active, l, crate::layer_place::Place::AutoEditable);
            *active = Some(id);
            sync(doc);
            Ok(id)
        })
        .unwrap();
    (s, id, synthetic)
}

fn layer(s: &Session, id: LayerId) -> &Layer {
    s.active().unwrap().doc.layer(id).unwrap()
}

fn video(s: &Session, id: LayerId) -> &VideoData {
    layer(s, id).video.as_ref().unwrap()
}

/// The displayed pixel of the layer at (x, y).
fn shown(s: &Session, id: LayerId, x: i32, y: i32) -> Vec<f32> {
    match &layer(s, id).content {
        LayerContent::Raster(r) => r.pixel(x, y),
        _ => panic!("a video layer shows raster content"),
    }
}

fn goto(s: &mut Session, frame: usize) {
    s.execute("timeline.setFrame", json!({"frame": frame})).unwrap();
}

/// Paint a red square on the displayed frame, as a brush or fill does to the layer's content.
fn paint(s: &mut Session, id: LayerId) {
    s.edit("Paint", |doc, _| {
        if let Some(Layer { content: LayerContent::Raster(r), .. }) = doc.layer_mut(id) {
            r.fill_rect(Rect::from_xywh(2, 2, 4, 4), &[1.0, 0.0, 0.0, 1.0]);
        }
        Ok(())
    })
    .unwrap();
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1.5 / 255.0
}

#[test]
fn footage_frames_are_decoded_when_shown_not_held() {
    let (mut s, id, synthetic) = linked("on-demand", 60);
    // Nothing but empty placeholders is held for 60 frames.
    assert!(video(&s, id).frames.iter().all(|f| f.tile_count() == 0));
    for frame in [0, 17, 59] {
        goto(&mut s, frame);
        assert!(close(shown(&s, id, 10, 10)[0], synthetic.level(frame)), "frame {frame} shows its own footage frame");
    }
    // Only the frames shown were decoded, not the whole file.
    assert!(synthetic.decodes.load(Ordering::Relaxed) < 10);
    // Showing a frame again reuses the decoded copy.
    let before = synthetic.decodes.load(Ordering::Relaxed);
    goto(&mut s, 17);
    assert_eq!(synthetic.decodes.load(Ordering::Relaxed), before);
}

#[test]
fn scrubbing_does_not_copy_frames_into_the_document() {
    let (mut s, id, _) = linked("scrub", 12);
    for frame in (0..12).chain((0..12).rev()) {
        goto(&mut s, frame);
    }
    let v = video(&s, id);
    assert!(v.footage.as_ref().unwrap().altered.iter().all(|a| !a), "no frame counts as painted after only moving the playhead");
    assert!(v.frames.iter().all(|f| f.tile_count() == 0));
}

#[test]
fn a_painted_frame_is_kept_and_restore_brings_the_footage_back() {
    let (mut s, id, synthetic) = linked("paint", 8);
    goto(&mut s, 3);
    paint(&mut s, id);
    goto(&mut s, 4);
    assert!(close(shown(&s, id, 3, 3)[0], synthetic.level(4)), "the next frame is untouched footage");
    goto(&mut s, 3);
    assert!(close(shown(&s, id, 3, 3)[1], 0.0), "the painted square is still on frame 3");
    assert!(close(shown(&s, id, 20, 20)[0], synthetic.level(3)), "outside the square frame 3 is its footage");
    assert!(video(&s, id).footage.as_ref().unwrap().altered[3]);

    // Show Altered Video off shows the footage under the paint.
    s.execute("layer.videoLayers.showAlteredVideo", json!({})).unwrap();
    assert!(close(shown(&s, id, 3, 3)[1], synthetic.level(3)));
    s.execute("layer.videoLayers.showAlteredVideo", json!({})).unwrap();
    assert!(close(shown(&s, id, 3, 3)[1], 0.0));

    s.execute("layer.videoLayers.restoreFrame", json!({})).unwrap();
    assert!(close(shown(&s, id, 3, 3)[1], synthetic.level(3)));
    assert!(!video(&s, id).footage.as_ref().unwrap().altered[3]);
}

#[test]
fn undo_takes_paint_off_a_footage_frame() {
    let (mut s, id, synthetic) = linked("undo", 6);
    goto(&mut s, 2);
    paint(&mut s, id);
    goto(&mut s, 0);
    goto(&mut s, 2);
    assert!(close(shown(&s, id, 3, 3)[1], 0.0));
    s.execute("edit.undo", json!({})).unwrap();
    goto(&mut s, 2);
    assert!(close(shown(&s, id, 3, 3)[1], synthetic.level(2)));
}

#[test]
fn inserted_duplicated_and_deleted_frames_keep_their_footage() {
    let (mut s, id, synthetic) = linked("frame-ops", 6);
    goto(&mut s, 1);
    s.execute("layer.videoLayers.insertBlankFrame", json!({})).unwrap();
    // The blank frame comes after the playhead; footage frame 2 moved to layer frame 3.
    goto(&mut s, 2);
    assert!(close(shown(&s, id, 10, 10)[3], 0.0), "the inserted frame is blank");
    goto(&mut s, 3);
    assert!(close(shown(&s, id, 10, 10)[0], synthetic.level(2)));

    s.execute("layer.videoLayers.duplicateFrame", json!({})).unwrap();
    goto(&mut s, 4);
    assert!(close(shown(&s, id, 10, 10)[0], synthetic.level(2)), "the duplicate shows the same footage frame");

    goto(&mut s, 2);
    s.execute("layer.videoLayers.deleteFrame", json!({})).unwrap();
    goto(&mut s, 2);
    assert!(close(shown(&s, id, 10, 10)[0], synthetic.level(2)), "the blank frame is gone");
    assert_eq!(video(&s, id).len(), 7);
}

#[test]
fn whole_image_edits_move_decoded_frames_with_the_painted_ones() {
    let (mut s, id, synthetic) = linked("geometry", 4);
    goto(&mut s, 1);
    paint(&mut s, id);

    // Canvas turned a quarter: both kinds of frame turn with it, pixel for pixel.
    s.execute("image.imageRotation.90cw", json!({})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert_eq!((doc.size.width, doc.size.height), (H, W));
    goto(&mut s, 2);
    assert!(close(shown(&s, id, 0, 0)[0], synthetic.level(2)));
    assert!(close(shown(&s, id, (H - 1) as i32, (W - 1) as i32)[0], synthetic.level(2)), "the decoded frame covers the turned canvas");
    assert!(close(shown(&s, id, H as i32, 0)[3], 0.0), "and stops at its edge");
    goto(&mut s, 1);
    // The painted square at (2..6, 2..6) turned clockwise lands at x = H-6..H-2, y = 2..6.
    assert!(close(shown(&s, id, (H - 4) as i32, 3)[1], 0.0));

    // Image Size halves both kinds of frame.
    s.execute("image.imageSize", json!({"percent": 50})).unwrap();
    goto(&mut s, 3);
    let (w, h) = (H / 2, W / 2);
    assert!(shown(&s, id, (w - 2) as i32, (h - 2) as i32)[3] > 0.5, "the decoded frame fills the smaller canvas");
    assert!(shown(&s, id, (w + 2) as i32, (h + 2) as i32)[3] < 0.5, "and no more");

    // A crop that deletes the cropped pixels keeps only the cropped part of the footage.
    s.execute("image.crop", json!({"x": 2, "y": 2, "width": 4, "height": 4, "deleteCroppedPixels": true})).unwrap();
    goto(&mut s, 0);
    let bounds = match &layer(&s, id).content {
        LayerContent::Raster(r) => r.content_bounds(),
        _ => panic!("a video layer shows raster content"),
    };
    assert!(bounds.width() <= 6 && bounds.height() <= 6, "the decoded frame was cropped too: {bounds:?}");
}

#[test]
fn native_documents_keep_the_link_and_only_the_painted_frames() {
    let (mut s, id, synthetic) = linked("pcraft", 30);
    goto(&mut s, 5);
    paint(&mut s, id);
    goto(&mut s, 6);
    let doc = s.active().unwrap().doc.clone();
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let mut again = photocraft_format::load_from_bytes(&bytes).unwrap();
    let v = again.layer(id).unwrap().video.as_ref().unwrap();
    let f = v.footage.as_ref().unwrap();
    assert_eq!(f.link, video(&s, id).footage.as_ref().unwrap().link);
    assert_eq!(f.altered.iter().filter(|a| **a).count(), 1);
    assert!(v.frames[5].tile_count() > 0, "the painted frame's pixels are saved");
    assert!(v.frames.iter().enumerate().all(|(i, fr)| i == 5 || fr.tile_count() == 0), "unpainted frames are not");

    // The reopened document shows the footage from the file again.
    if let Some(t) = &mut again.timeline {
        t.current = 9;
    }
    sync(&mut again);
    match &again.layer(id).unwrap().content {
        LayerContent::Raster(r) => assert!(close(r.pixel(10, 10)[0], synthetic.level(9))),
        _ => panic!("a video layer shows raster content"),
    }
}

#[test]
fn footage_that_cant_be_read_is_reported_not_rendered() {
    let (mut s, id, _) = linked("offline", 5);
    let file = video(&s, id).footage.as_ref().unwrap().link.path.clone();
    // The file goes away: with no movie reader installed in tests, it can't be opened again.
    crate::footage::forget(&file);
    let info = crate::inspect::layer(layer(&s, id));
    assert_eq!(info["video"]["footage"]["online"], false);
    assert!(info["video"]["footage"]["offline"].as_str().unwrap().contains(&file));

    goto(&mut s, 2);
    assert!(close(shown(&s, id, 10, 10)[3], 0.0), "an offline frame shows nothing, not a stale frame");
    let dir = std::env::temp_dir().join(format!("pc-footage-offline-{}", std::process::id()));
    let err = s.execute("file.export.renderVideo", json!({"dir": dir.to_string_lossy(), "format": "png"})).unwrap_err().to_string();
    assert!(err.contains(&file), "Render Video names the missing file: {err}");
    assert!(std::fs::read_dir(&dir).map_or(true, |mut d| d.next().is_none()), "and writes no frames");
}

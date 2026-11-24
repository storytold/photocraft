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
    linked_with(name, Synthetic::new(W, H, frames, 24.0))
}

/// [`linked`] with the decoder `decoder` (a damaged frame, a hook while decoding). The returned
/// `Synthetic` shares its decode count and draws the same levels.
fn linked_with(name: &str, decoder: Synthetic) -> (Session, LayerId, Synthetic) {
    let file = path(name);
    let twin = Synthetic { decodes: decoder.decodes.clone(), ..Synthetic::new(W, H, decoder.info.frames, decoder.info.fps) };
    let info = crate::footage::register(&file, Box::new(decoder));
    let mut s = crate::session();
    s.execute("file.new", json!({"width": W, "height": H})).unwrap();
    let id = s
        .edit("New Video Layer from File", |doc, active| {
            let fmt = doc.pixel_format();
            doc.timeline = Some(Timeline::new(info.frames, info.fps));
            let mut l = Layer::raster("Movie", fmt);
            l.video = Some(VideoData::from_footage(FootageLink::new(file.clone(), info), fmt));
            let id = crate::layer_place::insert_new(doc, *active, l, crate::layer_place::Place::AutoEditable);
            *active = Some(id);
            sync(doc);
            Ok(id)
        })
        .unwrap();
    (s, id, twin)
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
    // Inspecting doesn't open files: until something tries the file, its state is unknown.
    let info = crate::inspect::layer(layer(&s, id));
    assert!(info["video"]["footage"]["online"].is_null());

    goto(&mut s, 2);
    assert!(close(shown(&s, id, 10, 10)[3], 0.0), "an offline frame shows nothing, not a stale frame");
    let info = crate::inspect::layer(layer(&s, id));
    assert_eq!(info["video"]["footage"]["online"], false);
    assert!(info["video"]["footage"]["offline"].as_str().unwrap().contains(&file));
    let dir = std::env::temp_dir().join(format!("pc-footage-offline-{}", std::process::id()));
    let err = s.execute("file.export.renderVideo", json!({"dir": dir.to_string_lossy(), "format": "png"})).unwrap_err().to_string();
    assert!(err.contains(&file), "Render Video names the missing file: {err}");
    assert!(std::fs::read_dir(&dir).map_or(true, |mut d| d.next().is_none()), "and writes no frames");
}

/// A crop that deletes pixels keeps exactly the crop of footage that was turned by an angle that
/// isn't a quarter turn, as it does for the painted frames: no footage outside it comes back.
#[test]
fn a_deleting_crop_stays_exact_on_slanted_footage() {
    let (mut s, id, _) = linked("slanted-crop", 3);
    // Image Rotation by 30° about the canvas origin, as an arbitrary rotation moves the footage.
    let turn = photocraft_geom::Affine::rotate(30f64.to_radians());
    s.edit("Turn", |doc, _| {
        crate::canvas_geom::transform_geometry(doc, &turn);
        Ok(())
    })
    .unwrap();
    s.execute("image.crop", json!({"x": 0, "y": 4, "width": 12, "height": 10, "deleteCroppedPixels": true})).unwrap();
    goto(&mut s, 1);
    let LayerContent::Raster(frame) = &layer(&s, id).content else { panic!("a video layer shows raster content") };
    let canvas = s.active().unwrap().doc.bounds();
    let shown = frame.content_bounds();
    assert!(!shown.is_empty(), "part of the footage is inside the crop");
    for y in shown.y0..shown.y1 {
        for x in shown.x0..shown.x1 {
            let alpha = frame.pixel(x, y)[3];
            assert!(alpha == 0.0 || canvas.contains(x, y), "footage at ({x}, {y}) is outside the {canvas:?} crop (alpha {alpha})");
        }
    }
}

/// Render Video stops at a frame the decoder fails on instead of writing an empty frame.
#[test]
fn render_video_fails_on_a_frame_that_cant_be_decoded() {
    let (s, id, _) = linked_with("damaged", Synthetic { fail_at: Some(1), ..Synthetic::new(W, H, 4, 24.0) });
    let file = video(&s, id).footage.as_ref().unwrap().link.path.clone();
    let mut written = Vec::new();
    let err = render_video_with(
        &s,
        &json!({"format": "png"}),
        |name, _| {
            written.push(name.to_owned());
            Ok(())
        },
        |_, _| true,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("frame 2") && err.contains("damaged") && err.contains(&file), "the error names the frame, the file and why: {err}");
    assert!(written.iter().all(|n| !n.contains("_0001")), "the damaged frame isn't written: {written:?}");
}

/// A frame decoded while its file was closed (Reload Frame, a relink) isn't cached for the file
/// as it is opened next.
#[test]
fn a_frame_decoded_while_the_file_is_closed_is_not_cached() {
    let file = path("closed-mid-decode");
    let closing = file.clone();
    let decoder = Synthetic { during_decode: Some(Box::new(move || crate::footage::forget(&closing))), ..Synthetic::new(W, H, 3, 24.0) };
    crate::footage::register(&file, Box::new(decoder));
    let link = FootageLink::new(file.clone(), crate::footage::probe(&file).unwrap());
    let fmt = photocraft_color::PixelFormat { mode: photocraft_color::ColorMode::Rgb, sample: photocraft_color::SampleType::U8, alpha: true };
    assert!(crate::footage::frame(&link, 0, fmt).is_ok(), "the decode itself completes");
    // Had the frame been cached, this would be a cache hit; the file is closed, so it can't be read.
    assert!(crate::footage::frame(&link, 0, fmt).is_err(), "the closed file's frame was cached");
}

/// A native document whose video layer shows a footage frame past the end of its file is
/// refused as damaged rather than read into a layer that overflows counting its frames.
#[test]
fn a_saved_frame_past_the_footage_is_refused() {
    let (s, id, _) = linked("past-the-end", 2);
    let mut doc = (*s.active().unwrap().doc).clone();
    if let Some(f) = doc.layer_mut(id).and_then(|l| l.video.as_mut()).and_then(|v| v.footage.as_mut()) {
        std::sync::Arc::make_mut(&mut f.origin)[0] = Some(usize::MAX);
    }
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let err = photocraft_format::load_from_bytes(&bytes).unwrap_err().to_string();
    assert!(err.contains("past the end"), "{err}");
}

/// A session whose gate refuses the footage's path reads none of it: not for Render Video, not
/// when moving the playhead, and not from frames a trusted session decoded and cached before.
#[test]
fn a_restricted_session_reads_no_footage_it_may_not() {
    let (mut s, id, synthetic) = linked("restricted", 4);
    goto(&mut s, 2);
    assert!(close(shown(&s, id, 10, 10)[0], synthetic.level(2)), "trusted: frame 2 is decoded and cached");
    goto(&mut s, 0);
    s.authorize = Some(|_, params| if params.get("path").is_some() { Err(EngineError::Other("outside the read roots".into())) } else { Ok(()) });
    goto(&mut s, 2);
    assert!(close(shown(&s, id, 10, 10)[3], 0.0), "the cached frame isn't shown to the restricted session");
    let mut wrote = false;
    let err = render_video_with(
        &s,
        &json!({"format": "png"}),
        |_, _| {
            wrote = true;
            Ok(())
        },
        |_, _| true,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("may not be read"), "{err}");
    assert!(!wrote, "nothing is written");
}

/// Hidden layers add nothing to the output, so Render Video doesn't stop at their footage: a
/// damaged frame or a missing file only matters on a layer that shows.
#[test]
fn render_video_ignores_the_footage_of_hidden_layers() {
    let (mut s, id, _) = linked_with("hidden-damaged", Synthetic { fail_at: Some(1), ..Synthetic::new(W, H, 3, 24.0) });
    s.edit("Hide", |doc, _| {
        if let Some(l) = doc.layer_mut(id) {
            l.visible = false;
        }
        Ok(())
    })
    .unwrap();
    let files = render_video_with(&s, &json!({"format": "png"}), |_, _| Ok(()), |_, _| true).unwrap();
    assert_eq!(files["frames"], 3, "every frame renders without the hidden layer");
    let file = video(&s, id).footage.as_ref().unwrap().link.path.clone();
    crate::footage::forget(&file);
    assert!(render_video_with(&s, &json!({"format": "png"}), |_, _| Ok(()), |_, _| true).is_ok(), "nor does its missing file stop it");
}

/// The same relative path stored in two documents in different folders names two files: each
/// document shows its own, not whichever was decoded first.
#[test]
fn relative_footage_paths_name_the_file_beside_each_document() {
    let dir = |name: &str| std::env::temp_dir().join(format!("pc-footage-rel-{}-{name}", std::process::id()));
    let (a, b) = (dir("a"), dir("b"));
    // Different lengths, so their frame 0 is drawn in different greys.
    let (short, long) = (Synthetic::new(W, H, 3, 24.0), Synthetic::new(W, H, 9, 24.0));
    let (short_level, long_level) = (short.level(0), long.level(0));
    crate::footage::register(&a.join("clip.mp4").to_string_lossy(), Box::new(short));
    crate::footage::register(&b.join("clip.mp4").to_string_lossy(), Box::new(long));
    let shown_in = |folder: &std::path::Path| {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": W, "height": H})).unwrap();
        s.active_mut().unwrap().path = Some(folder.join("doc.pcraft").to_string_lossy().into_owned());
        let info = photocraft_doc::FootageInfo { width: W, height: H, frames: 3, fps: 24.0 };
        let id = s
            .edit("Link", |doc, active| {
                let fmt = doc.pixel_format();
                doc.timeline = Some(Timeline::new(3, 24.0));
                let mut l = Layer::raster("Movie", fmt);
                l.video = Some(VideoData::from_footage(FootageLink::new("clip.mp4".into(), info), fmt));
                let id = crate::layer_place::insert_new(doc, *active, l, crate::layer_place::Place::AutoEditable);
                *active = Some(id);
                sync(doc);
                Ok(id)
            })
            .unwrap();
        shown(&s, id, 10, 10)[0]
    };
    assert!(close(shown_in(&a), short_level), "the document in a shows a's clip");
    assert!(close(shown_in(&b), long_level), "the document in b shows b's clip");
}

/// Reopening a native document doesn't turn the unpainted footage frame at the playhead into the
/// layer's own pixels the first time anything is stored.
#[test]
fn a_reopened_document_keeps_its_footage_frame_unpainted() {
    let (mut s, id, _) = linked("reopened", 6);
    goto(&mut s, 2);
    let doc = s.active().unwrap().doc.clone();
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let loaded = photocraft_format::load_from_bytes(&bytes).unwrap();
    let mut again = crate::session();
    again.add_document(loaded, None);
    goto(&mut again, 3);
    let v = again.active().unwrap().doc.layer(id).unwrap().video.clone().unwrap();
    assert!(v.footage.as_ref().unwrap().altered.iter().all(|a| !a), "no frame became painted");
    assert!(v.frames.iter().all(|f| f.tile_count() == 0));
}

/// Render Video reads the file only for frames that show it: a layer whose every frame is
/// painted renders while its file is missing.
#[test]
fn painted_frames_render_without_their_file() {
    let (mut s, id, _) = linked("painted-offline", 2);
    for frame in [0, 1] {
        goto(&mut s, frame);
        paint(&mut s, id);
    }
    goto(&mut s, 0);
    let file = video(&s, id).footage.as_ref().unwrap().link.path.clone();
    crate::footage::forget(&file);
    assert!(render_video_with(&s, &json!({"format": "png"}), |_, _| Ok(()), |_, _| true).is_ok());
    // With Show Altered Video off the footage shows again, and the missing file stops the render.
    s.execute("layer.videoLayers.showAlteredVideo", json!({})).unwrap();
    assert!(render_video_with(&s, &json!({"format": "png"}), |_, _| Ok(()), |_, _| true).is_err());
}

/// Replacing a linked movie with image footage leaves no link behind: the layer holds the new
/// frames, and nothing is decoded from the old file.
#[test]
fn replacing_linked_footage_with_images_drops_the_link() {
    let (mut s, id, _) = linked("replaced", 4);
    let mut img = crate::session();
    img.execute("file.new", json!({"width": W, "height": H, "background": "white"})).unwrap();
    let png = std::env::temp_dir().join(format!("pc-footage-replaced-{}.png", std::process::id()));
    std::fs::write(&png, photocraft_io::export(&img.active().unwrap().doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes).unwrap();
    s.execute("layer.videoLayers.replaceFootage", json!({"path": png.to_string_lossy()})).unwrap();
    let v = video(&s, id);
    assert!(v.footage.is_none() && v.shown.is_none());
    assert_eq!(v.len(), 1);
    let _ = std::fs::remove_file(&png);
}

/// Moving the playhead copies the document but not a linked layer's frame tables: they stay
/// shared until a frame is painted, and the history state before the paint keeps its own.
#[test]
fn moving_the_playhead_shares_the_frame_tables() {
    let (mut s, id, _) = linked("shared-tables", 500);
    let before = video(&s, id).clone();
    goto(&mut s, 7);
    let after = video(&s, id).clone();
    assert!(std::sync::Arc::ptr_eq(&before.frames, &after.frames), "the frames table is shared");
    let (fb, fa) = (before.footage.as_ref().unwrap(), after.footage.as_ref().unwrap());
    assert!(std::sync::Arc::ptr_eq(&fb.origin, &fa.origin) && std::sync::Arc::ptr_eq(&fb.altered, &fa.altered));
    paint(&mut s, id);
    goto(&mut s, 8);
    let painted = video(&s, id).clone();
    assert!(!std::sync::Arc::ptr_eq(&after.frames, &painted.frames), "painting copies the table");
    assert!(after.frames[7].tile_count() == 0 && painted.frames[7].tile_count() > 0, "and the earlier copy keeps its own");
}

/// Render Video only visits the timeline's frames: footage frames of the layer past the end of
/// the timeline don't need the file.
#[test]
fn only_frames_the_render_visits_need_the_file() {
    let (mut s, id, _) = linked("visited", 4);
    paint(&mut s, id);
    s.execute("timeline.setProps", json!({"duration": 1})).unwrap();
    let file = video(&s, id).footage.as_ref().unwrap().link.path.clone();
    crate::footage::forget(&file);
    assert!(render_video_with(&s, &json!({"format": "png"}), |_, _| Ok(()), |_, _| true).is_ok(), "frame 0 is painted, and frames 1-3 aren't rendered");
}

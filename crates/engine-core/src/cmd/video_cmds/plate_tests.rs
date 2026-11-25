//! An image sequence's plate stays in its files: unpainted frames are read when shown, through a
//! cache of the frames nearest the playhead, so a plate far larger than the footage limit opens,
//! scrubs, paints, saves and renders while memory stays bounded.

use super::*;
use photocraft_doc::LayerId;
use serde_json::json;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const W: u32 = 40;
const H: u32 = 30;
/// Bytes one 40 x 30 RGBA8 frame keeps: one whole 256 x 256 tile.
const FRAME_BYTES: u64 = 256 * 256 * 4;

/// A fresh, empty folder for one test.
fn folder(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pc-plate-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The red bar frame `i` of a test plate draws: 1 to 39 pixels wide, so neighbours differ.
fn bar_of(i: usize) -> u32 {
    (i % (W as usize - 1)) as u32 + 1
}

fn frame_path(dir: &std::path::Path, i: usize) -> std::path::PathBuf {
    dir.join(format!("plate.{:04}.png", i + 1))
}

/// PNG bytes of a white `W` x `H` frame with a red bar `bar` pixels wide down its left edge.
fn bar_png(s: &mut Session, bar: u32) -> Vec<u8> {
    s.edit("Fill", |doc, active| {
        let fmt = doc.pixel_format();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surf.fill_rect(photocraft_geom::Rect::from_xywh(0, 0, W, H), &photocraft_raster::from_rgba(&fmt, [1.0; 4]));
        surf.fill_rect(photocraft_geom::Rect::from_xywh(0, 0, bar, H), &photocraft_raster::from_rgba(&fmt, RED));
        Ok(())
    })
    .unwrap();
    photocraft_io::export(&s.active().unwrap().doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes
}

/// `plate.0001.png` .. in `dir`, frame `i` with the bar [`bar_of`]`(i)`.
fn write_plate(dir: &std::path::Path, frames: usize) {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": W, "height": H, "background": "#ffffff"})).unwrap();
    let pngs: Vec<Vec<u8>> = (0..W as usize - 1).map(|i| bar_png(&mut s, bar_of(i))).collect();
    for i in 0..frames {
        std::fs::write(frame_path(dir, i), &pngs[i % pngs.len()]).unwrap();
    }
}

/// Opens the plate in `dir` as a new document; the video layer's id.
fn open(s: &mut Session, dir: &std::path::Path) -> LayerId {
    let r = s.execute("file.openImageSequence", json!({"path": frame_path(dir, 0).to_string_lossy()})).unwrap();
    LayerId(r["layer"].as_u64().unwrap())
}

/// The width of the red run from the left on row 15.
fn bar(frame: &Surface) -> u32 {
    (0..W as i32).take_while(|&x| frame.pixel(x, 15) == RED).count() as u32
}

fn video(s: &Session, id: LayerId) -> &VideoData {
    s.active().unwrap().doc.layer(id).unwrap().video.as_ref().unwrap()
}

fn shown(s: &Session, id: LayerId) -> Surface {
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Raster(surf) => surf.clone(),
        _ => panic!("a video layer shows a raster"),
    }
}

fn go(s: &mut Session, frame: usize) {
    s.execute("timeline.setFrame", json!({"frame": frame})).unwrap();
}

/// Frames the document holds as pixels rather than reading from the plate.
fn held(v: &VideoData) -> usize {
    v.frames.iter().filter(|f| f.pixels().is_some()).count()
}

/// Paint one green pixel at (`x`, 20) into the frame on screen.
fn paint(s: &mut Session, id: LayerId, x: i32) {
    s.edit("Brush", |doc, _| {
        doc.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(x, 20, &[0.0, 1.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
}

/// A 200-frame plate needing far more than the footage limit opens; scrubbing every frame shows
/// each one's own pixels while the decoded frames stay within the cache budget, no frame is
/// copied into the document, and Render Video writes every frame as the plate holds it.
#[test]
fn a_plate_beyond_the_footage_limit_opens_scrubs_and_renders_in_bounded_memory() {
    const FRAMES: usize = 200;
    let dir = folder("long");
    write_plate(&dir, FRAMES);
    let limit = 16 * FRAME_BYTES;
    let budget = 4 * FRAME_BYTES;
    assert!(FRAMES as u64 * FRAME_BYTES > limit, "the plate is beyond the limit");
    let _limit = limits::Scoped::max_footage_bytes(limit);
    let _budget = limits::Scoped::plate_cache_bytes(budget);
    let mut s = crate::session();
    let id = open(&mut s, &dir);
    assert_eq!(video(&s, id).len(), FRAMES);
    let mut most = 0;
    for f in 0..FRAMES {
        go(&mut s, f);
        assert_eq!(bar(&shown(&s, id)), bar_of(f), "frame {f} shows its own file");
        let plate = video(&s, id).plate.clone().unwrap();
        most = most.max(plate.cached_bytes());
    }
    assert!(most <= budget, "the cache stays within its budget: {most} > {budget}");
    assert!(video(&s, id).plate.as_ref().unwrap().cached_frames().len() < FRAMES);
    assert_eq!(held(video(&s, id)), 0, "scrubbing copies no frame into the document");

    let mut rendered = Vec::new();
    render_video_with(
        &s,
        &json!({"format": "png"}),
        |_, bytes| {
            rendered.push(bytes.to_vec());
            Ok(())
        },
        |_, _| true,
    )
    .unwrap();
    assert_eq!(rendered.len(), FRAMES);
    for f in [0, 1, 57, 199] {
        let doc = crate::file_cmds::import_with_options("frame.png", &rendered[f], &photocraft_io::ImportOptions::default()).unwrap();
        assert_eq!(bar(&crate::file_cmds::flattened(&doc, doc.pixel_format())), bar_of(f), "rendered frame {f}");
    }
    assert!(video(&s, id).plate.as_ref().unwrap().cached_bytes() <= budget, "rendering stays within the cache too");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A frame the cache dropped is read again when the playhead comes back, painting it keeps the
/// paint over the plate's pixels, its neighbours stay unpainted plate frames, and undo takes the
/// paint back off.
#[test]
fn painting_a_frame_the_cache_dropped_keeps_the_paint() {
    let dir = folder("paint");
    write_plate(&dir, 12);
    let _budget = limits::Scoped::plate_cache_bytes(FRAME_BYTES);
    let mut s = crate::session();
    let id = open(&mut s, &dir);
    go(&mut s, 2);
    go(&mut s, 11);
    assert!(!video(&s, id).plate.as_ref().unwrap().cached_frames().contains(&2), "frame 2 left the cache");
    go(&mut s, 2);
    paint(&mut s, id, 30);
    go(&mut s, 3);
    go(&mut s, 2);
    let frame = shown(&s, id);
    assert_eq!(frame.pixel(30, 20), vec![0.0, 1.0, 0.0, 1.0], "the paint stays on frame 2");
    assert_eq!(bar(&frame), bar_of(2), "over frame 2's own pixels");
    let v = video(&s, id);
    assert!(v.frames[2].pixels().is_some() && held(v) == 1, "only the painted frame is held");
    assert!(v.original_frames.as_ref().unwrap()[2].plate_index().is_some(), "the original stays the plate's");

    while s.undo() && video(&s, id).frames[2].pixels().is_some() {}
    go(&mut s, 3);
    go(&mut s, 2);
    assert_ne!(shown(&s, id).pixel(30, 20), vec![0.0, 1.0, 0.0, 1.0], "undo took the paint off");
    assert_eq!(held(video(&s, id)), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A saved document stores its painted frames and refers to the plate for the rest (smaller than
/// storing every frame), and reopened, reads the plate frames again.
#[test]
fn saving_stores_painted_frames_and_refers_to_the_plate() {
    let dir = folder("save");
    write_plate(&dir, 8);
    let mut s = crate::session();
    let id = open(&mut s, &dir);
    go(&mut s, 4);
    paint(&mut s, id, 33);
    go(&mut s, 0);
    let doc = (*s.active().unwrap().doc).clone();
    let bytes = photocraft_format::save_to_bytes(&doc, &photocraft_format::SaveOptions::default()).unwrap();

    let mut whole = doc.clone();
    for l in whole.layers.iter_mut().flat_map(|l| l.children_mut().into_iter().flatten()) {
        if let Some(v) = &mut l.video {
            hold_plate_frames(v);
        }
    }
    let whole_bytes = photocraft_format::save_to_bytes(&whole, &photocraft_format::SaveOptions::default()).unwrap();
    assert!(bytes.len() < whole_bytes.len(), "plate frames are references, not pixels: {} >= {}", bytes.len(), whole_bytes.len());

    let back = photocraft_format::load_from_bytes(&bytes).unwrap();
    let mut s = crate::session();
    s.add_document(back, None);
    let id = s.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.video.is_some()).map(|(_, _, l)| l.id).unwrap();
    let v = video(&s, id);
    assert_eq!((v.len(), held(v)), (8, 1), "the painted frame is stored, the others refer to the plate");
    assert_eq!(v.plate_problem, None);
    go(&mut s, 4);
    assert_eq!(shown(&s, id).pixel(33, 20), vec![0.0, 1.0, 0.0, 1.0]);
    go(&mut s, 6);
    assert_eq!(bar(&shown(&s, id)), bar_of(6), "an unpainted frame reads its file");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A plate file changed since the plate was opened, or gone, is not shown as its frame: the frame
/// shows empty with the reason, stays a plate reference, and Render Video refuses it. Reload
/// Frame reads the files as they are now.
#[test]
fn a_plate_file_that_changes_or_disappears_is_reported() {
    let dir = folder("changed");
    write_plate(&dir, 8);
    let _budget = limits::Scoped::plate_cache_bytes(FRAME_BYTES);
    let mut s = crate::session();
    let id = open(&mut s, &dir);
    go(&mut s, 7);
    go(&mut s, 0);
    assert!(!video(&s, id).plate.as_ref().unwrap().cached_frames().contains(&5));

    // Frame 6 (index 5) now holds frame 3's pixels, written later.
    std::fs::copy(frame_path(&dir, 2), frame_path(&dir, 5)).unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
    std::fs::File::options().write(true).open(frame_path(&dir, 5)).unwrap().set_modified(later).unwrap();
    go(&mut s, 5);
    let problem = video(&s, id).plate_problem.clone().expect("the change is reported");
    assert!(problem.contains("changed"), "{problem}");
    assert_ne!(bar(&shown(&s, id)), bar_of(2), "the changed file is not shown as the frame");
    assert!(render_video_with(&s, &json!({"format": "png"}), |_, _| Ok(()), |_, _| true).is_err(), "rendering refuses the frame");

    std::fs::remove_file(frame_path(&dir, 6)).unwrap();
    let moved = s.execute("timeline.setFrame", json!({"frame": 6})).unwrap();
    assert!(video(&s, id).plate_problem.is_some(), "a missing file is reported");
    assert!(moved["warnings"].as_array().is_some_and(|w| !w.is_empty()), "and the move says so: {moved}");
    go(&mut s, 1);
    assert_eq!(video(&s, id).plate_problem, None, "a readable frame clears the report");
    assert_eq!(held(video(&s, id)), 0, "unreadable frames stay plate references");

    assert!(s.execute("layer.videoLayers.reloadFrame", json!({})).is_err(), "reloading needs every file");
    std::fs::copy(frame_path(&dir, 0), frame_path(&dir, 6)).unwrap();
    s.execute("layer.videoLayers.reloadFrame", json!({})).unwrap();
    go(&mut s, 5);
    assert_eq!(video(&s, id).plate_problem, None);
    assert_eq!(bar(&shown(&s, id)), bar_of(2), "after reloading, frame 6 shows its file as it is now");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An edit to every frame (a canvas flip) reads the plate frames in and flips them all; when that
/// would hold more than the footage limit, it is refused and the plate is left as it was.
#[test]
fn an_edit_to_every_frame_reads_the_plate_in_or_is_refused() {
    let dir = folder("flip");
    write_plate(&dir, 6);
    let mut s = crate::session();
    let id = open(&mut s, &dir);
    {
        let _limit = limits::Scoped::max_footage_bytes(3 * FRAME_BYTES);
        let err = s.execute("image.imageRotation.flipCanvasHorizontal", json!({})).unwrap_err().to_string();
        assert!(err.contains("plate frames"), "{err}");
        assert_eq!(held(video(&s, id)), 0, "the refused edit left the plate alone");
    }
    s.execute("image.imageRotation.flipCanvasHorizontal", json!({})).unwrap();
    let v = video(&s, id);
    assert!(v.plate.is_none() && held(v) == 6);
    for f in 0..6 {
        let frame = super::frame_pixels(v, f);
        assert_eq!(frame.pixel(W as i32 - 1, 15), RED, "frame {f} is flipped");
        assert_eq!((0..W as i32).rev().take_while(|&x| frame.pixel(x, 15) == RED).count() as u32, bar_of(f));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A lower cache budget (Memory Usage turned down) reaches a plate that is already open: its next
/// read, even one the cache answers, drops the frames that no longer fit.
#[test]
fn an_open_plate_applies_a_lowered_cache_budget() {
    let dir = folder("memory");
    write_plate(&dir, 12);
    let mut s = crate::session();
    let id = open(&mut s, &dir);
    let opened = video(&s, id).plate.as_ref().unwrap().cached_bytes();
    let low = 2 * FRAME_BYTES;
    assert!(opened > low, "the plate opened with more cached than the lower budget allows");
    let _low = limits::Scoped::plate_cache_bytes(low);
    go(&mut s, 11);
    go(&mut s, 3);
    let cached = video(&s, id).plate.as_ref().unwrap().cached_bytes();
    assert!(cached <= low, "{cached} > {low}");
    assert_eq!(bar(&shown(&s, id)), bar_of(3));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Another frame of the document (what a clone source with a frame offset samples) reads a plate
/// frame the cache dropped from its file, leaves the document's own frame as it was, and refuses
/// a frame whose file is gone.
#[test]
fn another_frame_reads_a_dropped_plate_frame_from_its_file() {
    let dir = folder("at-frame");
    write_plate(&dir, 12);
    let mut s = crate::session();
    let id = open(&mut s, &dir);
    let _low = limits::Scoped::plate_cache_bytes(FRAME_BYTES);
    go(&mut s, 0);
    go(&mut s, 6);
    assert!(!video(&s, id).plate.as_ref().unwrap().cached_frames().contains(&5), "frame 5 is out of the cache");
    let doc = s.active().unwrap().doc.clone();
    let at = at_frame(&doc, 5).unwrap();
    let LayerContent::Raster(previous) = &at.layer(id).unwrap().content else { panic!("a raster") };
    assert_eq!(bar(previous), bar_of(5));
    assert_eq!(bar(&shown(&s, id)), bar_of(6), "the document still shows its own frame");
    std::fs::remove_file(frame_path(&dir, 4)).unwrap();
    let err = at_frame(&doc, 4).unwrap_err();
    assert!(err.contains("can't be read"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Opening decodes only the frames that fit the cache; the files past it have their headers read.
/// So a later file whose header is right but whose pixels are broken opens, and is reported
/// when its frame is shown, while a later file of another size still refuses the open.
#[test]
fn opening_reads_only_the_headers_of_frames_past_the_cache() {
    let dir = folder("probe");
    write_plate(&dir, 10);
    let _low = limits::Scoped::plate_cache_bytes(2 * FRAME_BYTES);
    let whole = std::fs::read(frame_path(&dir, 7)).unwrap();
    // The compressed pixels made invalid (a reserved deflate block type), the header left as is.
    let mut broken = whole.clone();
    let idat = broken.windows(4).position(|w| w == b"IDAT").unwrap() + 4;
    broken[idat + 2..idat + 12].fill(0xff);
    std::fs::write(frame_path(&dir, 7), &broken).unwrap();
    let mut s = crate::session();
    let id = open(&mut s, &dir);
    assert!(video(&s, id).plate.as_ref().unwrap().cached_frames().len() < 10, "frames past the cache were not kept");
    go(&mut s, 6);
    assert!(video(&s, id).plate_problem.is_none());
    let moved = s.execute("timeline.setFrame", json!({"frame": 7})).unwrap();
    assert!(video(&s, id).plate_problem.is_some() && moved["warnings"].as_array().is_some_and(|w| !w.is_empty()), "{moved}");

    // A later frame of another size is still refused at open.
    std::fs::write(frame_path(&dir, 7), &whole).unwrap();
    let mut other = crate::session();
    other.execute("file.new", json!({"width": W + 2, "height": H, "background": "#ffffff"})).unwrap();
    let wider = photocraft_io::export(&other.active().unwrap().doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes;
    std::fs::write(frame_path(&dir, 8), wider).unwrap();
    let err = s.execute("file.openImageSequence", json!({"path": frame_path(&dir, 0).to_string_lossy()})).unwrap_err().to_string();
    assert!(err.contains("plate.0009"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

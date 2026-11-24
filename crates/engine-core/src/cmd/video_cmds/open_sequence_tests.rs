//! File › Open with Image Sequence (#14057): `file.openImageSequence` against the measured
//! reference case, a folder with `frame001.png` to `frame003.png` (40 x 30, a red bar 10, 20 and
//! 30 pixels wide) and an unnumbered `flat.png`.

use super::*;
use photocraft_doc::LayerId;
use serde_json::json;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// A fresh, empty folder for one test.
fn folder(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pc-imgseq-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Writes a `w` x `h` white PNG with a red bar `bar` pixels wide down its left edge.
fn write_bar(path: &std::path::Path, w: u32, h: u32, bar: u32) {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": w, "height": h, "background": "#ffffff"})).unwrap();
    // Edit › Fill lives in the paint crate; paint the bar into the Background directly.
    s.edit("Fill", |doc, active| {
        let fmt = doc.pixel_format();
        let bar = photocraft_geom::Rect { x0: 0, y0: 0, x1: bar as i32, y1: h as i32 };
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(bar, &photocraft_raster::from_rgba(&fmt, RED));
        Ok(())
    })
    .unwrap();
    let bytes = photocraft_io::export(&s.active().unwrap().doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes;
    std::fs::write(path, bytes).unwrap();
}

/// The measured test folder: `frame001.png` .. `frame003.png` with bars 10, 20 and 30 pixels
/// wide, and the unnumbered `flat.png`.
fn measured_folder(name: &str) -> std::path::PathBuf {
    let d = folder(name);
    for (i, bar) in [10, 20, 30].into_iter().enumerate() {
        write_bar(&d.join(format!("frame00{}.png", i + 1)), 40, 30, bar);
    }
    write_bar(&d.join("flat.png"), 40, 30, 5);
    d
}

/// Opens the sequence `file` in `folder` belongs to.
fn open(s: &mut Session, folder: &std::path::Path, file: &str, params: Value) -> Result<Value> {
    let mut p = json!({"path": folder.join(file).to_string_lossy()});
    if let (Some(p), Some(extra)) = (p.as_object_mut(), params.as_object()) {
        p.extend(extra.clone());
    }
    s.execute("file.openImageSequence", p)
}

/// The width of the red bar in `frame`: the first column, on row 15, that isn't red. Every column
/// before it must be red and every one from it on white, so a frame with the wrong pixels fails.
fn bar_width(frame: &Surface) -> u32 {
    let width = (0..40).find(|&x| frame.pixel(x, 15) != RED).unwrap_or(40);
    for x in 0..40 {
        assert_eq!(frame.pixel(x, 15), if x < width { RED } else { WHITE }, "column {x}");
    }
    width as u32
}

/// The bar widths of every frame of the video layer `id`, in frame order.
fn bars(s: &Session, id: LayerId) -> Vec<u32> {
    s.active().unwrap().doc.layer(id).unwrap().video.as_ref().unwrap().frames.iter().map(bar_width).collect()
}

/// The measured result: one new unsaved "Untitled-1", 40 x 30, holding "Video Group 1" with the
/// video layer "Layer 1" (the three frames in name order at the chosen rate), History "Open",
/// "Open". `flat.png` stays out.
#[test]
fn a_numbered_run_opens_as_one_untitled_video_document() {
    let d = measured_folder("measured");
    let mut s = crate::session();
    let r = open(&mut s, &d, "frame001.png", json!({"fps": 12.5})).unwrap();
    assert_eq!(s.documents().len(), 1);
    assert_eq!(r["document"], 0);
    assert_eq!(r["frames"], 3);
    let st = s.active().unwrap();
    assert_eq!(st.doc.name, "Untitled-1");
    assert_eq!(st.path, None, "the document is unsaved, not the first frame's file");
    assert_eq!((st.doc.size.width, st.doc.size.height), (40, 30));
    let names: Vec<(&str, Option<Vec<&str>>)> =
        st.doc.layers.iter().map(|l| (l.name.as_str(), l.children().map(|c| c.iter().map(|c| c.name.as_str()).collect()))).collect();
    assert_eq!(names, [("Video Group 1", Some(vec!["Layer 1"]))]);
    let group = &st.doc.layers[0];
    assert!(group.is_group() && group.video.is_none());
    let layer = &group.children().unwrap()[0];
    assert_eq!(r["layer"], layer.id.0);
    assert_eq!(st.active_layer, Some(layer.id));
    assert_eq!(st.selected_layers(), [layer.id]);
    let video = layer.video.as_ref().unwrap();
    assert_eq!(video.fps, 12.5);
    assert_eq!(video.original_frames.as_ref().map(Vec::len), Some(3), "Restore Frame has the footage");
    assert_eq!(bars(&s, layer.id), [10, 20, 30]);
    let LayerContent::Raster(shown) = &layer.content else { panic!("a video layer shows a raster") };
    assert_eq!(bar_width(shown), 10, "the layer shows the first frame");
    let timeline = st.doc.timeline.as_ref().unwrap();
    assert_eq!((timeline.duration, timeline.fps, timeline.current), (3, 12.5, 0));
    assert_eq!(st.history.entries(), ["Open", "Open"]);
}

/// Picking a later frame opens the whole run from its first frame, and the frame rate defaults to
/// the dialog's 30.
#[test]
fn any_frame_of_the_run_opens_all_of_it_at_30_fps_by_default() {
    let d = measured_folder("middle");
    let mut s = crate::session();
    let r = open(&mut s, &d, "frame002.png", json!({})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(bars(&s, id), [10, 20, 30]);
    assert_eq!(r["fps"], 30.0);
    assert_eq!(s.active().unwrap().doc.timeline.as_ref().unwrap().fps, 30.0);
}

/// Each sequence opens as the next free Untitled-N, beside the documents already open.
#[test]
fn the_document_takes_the_next_free_untitled_name() {
    let d = measured_folder("untitled");
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 8, "name": "Untitled-1"})).unwrap();
    assert_eq!(s.active().unwrap().doc.name, "Untitled-1");
    open(&mut s, &d, "frame001.png", json!({})).unwrap();
    open(&mut s, &d, "frame003.png", json!({})).unwrap();
    let names: Vec<&str> = s.documents().iter().map(|d| d.doc.name.as_str()).collect();
    assert_eq!(names, ["Untitled-1", "Untitled-2", "Untitled-3"]);
}

/// Undo takes the video layer back out, leaving the document as it opened; Redo brings it back.
#[test]
fn undo_and_redo_step_through_the_two_open_states() {
    let d = measured_folder("undo");
    let mut s = crate::session();
    let r = open(&mut s, &d, "frame001.png", json!({})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert!(s.undo());
    assert!(s.active().unwrap().doc.layers.is_empty());
    assert!(s.redo());
    assert_eq!(bars(&s, id), [10, 20, 30]);
}

/// The measured alert for a file whose name has no number, and no document.
#[test]
fn an_unnumbered_file_is_not_part_of_a_numeric_sequence() {
    let d = measured_folder("flat");
    let mut s = crate::session();
    let e = open(&mut s, &d, "flat.png", json!({})).unwrap_err();
    assert_eq!(e.to_string(), "Could not complete the Open command because the file is not part of a numeric sequence.");
    assert!(s.documents().is_empty());
}

/// Bad sequences are errors that open nothing: a frame number missing in the middle, two files
/// with the same number, frames of another size, a frame that isn't an image, a missing file.
#[test]
fn bad_sequences_are_errors_and_open_nothing() {
    let mut s = crate::session();

    let gap = folder("gap");
    write_bar(&gap.join("frame001.png"), 40, 30, 10);
    write_bar(&gap.join("frame003.png"), 40, 30, 30);
    let e = open(&mut s, &gap, "frame001.png", json!({})).unwrap_err().to_string();
    assert!(e.contains("missing frame `frame002.png`"), "{e}");

    let twice = folder("twice");
    write_bar(&twice.join("frame1.png"), 40, 30, 10);
    write_bar(&twice.join("frame01.png"), 40, 30, 20);
    let e = open(&mut s, &twice, "frame1.png", json!({})).unwrap_err().to_string();
    assert!(e.contains("the same frame number"), "{e}");

    let sizes = folder("sizes");
    write_bar(&sizes.join("frame001.png"), 40, 30, 10);
    write_bar(&sizes.join("frame002.png"), 30, 40, 20);
    let e = open(&mut s, &sizes, "frame001.png", json!({})).unwrap_err().to_string();
    assert!(e.contains("frame002.png") && e.contains("30 x 40 pixels") && e.contains("frames are 40 x 30"), "{e}");

    let broken = folder("broken");
    write_bar(&broken.join("frame001.png"), 40, 30, 10);
    std::fs::write(broken.join("frame002.png"), b"not a png").unwrap();
    assert!(open(&mut s, &broken, "frame001.png", json!({})).is_err());

    assert!(open(&mut s, &broken, "frame009.png", json!({})).unwrap_err().to_string().contains("is not a file"));
    assert!(s.documents().is_empty(), "no document is left open");
}

/// Missing, wrong-typed and out-of-range params are errors, never a panic or a document.
#[test]
fn bad_params_are_errors() {
    let d = measured_folder("params");
    let mut s = crate::session();
    assert!(s.execute("file.openImageSequence", json!({})).is_err());
    assert!(s.execute("file.openImageSequence", json!({"path": 7})).is_err());
    for fps in [json!(0), json!(-1), json!(1001), json!("30"), json!(f64::MAX)] {
        assert!(open(&mut s, &d, "frame001.png", json!({"fps": fps})).is_err(), "{fps}");
    }
    assert!(s.documents().is_empty());
}

/// Frames are matched on the text around the last run of digits before the extension.
#[test]
fn file_names_split_around_their_frame_number() {
    let n = |name: &'static str| Numbered::parse(name).map(|n| (n.prefix, n.digits, n.suffix));
    assert_eq!(n("frame001.png"), Some(("frame", "001", ".png")));
    assert_eq!(n("shot2_frame010.tif"), Some(("shot2_frame", "010", ".tif")));
    assert_eq!(n("take3_v2.png"), Some(("take3_v", "2", ".png")));
    assert_eq!(n("0007.exr"), Some(("", "0007", ".exr")));
    assert_eq!(n("scène 12.jpg"), Some(("scène ", "12", ".jpg")));
    assert_eq!(n("é001.png"), Some(("é", "001", ".png")), "a multibyte character right before the digits");
    assert_eq!(Numbered::parse("é001.png").and_then(|n| n.number()), Some(1));
    assert_eq!(n("12"), Some(("", "12", "")));
    assert_eq!(n("flat.png"), None);
    assert_eq!(n("clip.mp4"), None, "the extension's digits aren't a frame number");
    assert_eq!(n(".123"), Some((".", "123", "")), "a dot file's name is all stem");
    assert_eq!(n("f1234567890123456789.png"), None, "past 18 digits");
    let a = Numbered::parse("frame001.png").unwrap();
    assert!(a.same_run(&Numbered::parse("frame002.PNG").unwrap()));
    assert!(!a.same_run(&Numbered::parse("frames002.png").unwrap()));
    assert!(!a.same_run(&Numbered::parse("frame002.jpg").unwrap()));
}

/// More matching files than a video layer can hold is refused before any frame is read.
#[test]
fn a_run_longer_than_the_frame_limit_is_refused() {
    let d = folder("limit");
    for i in 1..=3 {
        // Not images at all: the count is refused before anything is decoded.
        std::fs::write(d.join(format!("frame{i}.png")), b"x").unwrap();
    }
    let path = d.join("frame1.png").to_string_lossy().into_owned();
    assert_eq!(sequence_files_within(&path, 3).unwrap().len(), 3);
    let e = sequence_files_within(&path, 2).unwrap_err().to_string();
    assert!(e.contains("more than 2 frames"), "{e}");
}

/// Each file is one frame of the sequence, even an animated GIF: its first frame, as File › Open
/// reads it, not every frame of its animation.
#[test]
fn each_file_is_one_frame_even_an_animated_gif() {
    let d = folder("gif");
    for i in 1..=2 {
        let path = d.join(format!("clip{i}.gif")).to_string_lossy().into_owned();
        super::tests::write_gif(&path, 40, 30, &[([255, 0, 0], 10), ([0, 0, 255], 10), ([0, 255, 0], 10)]);
    }
    let mut s = crate::session();
    let r = open(&mut s, &d, "clip1.gif", json!({})).unwrap();
    assert_eq!(r["frames"], 2);
    let id = LayerId(r["layer"].as_u64().unwrap());
    let frames = &s.active().unwrap().doc.layer(id).unwrap().video.as_ref().unwrap().frames;
    assert!(frames.iter().all(|f| f.pixel(20, 15) == RED), "each file's first frame");
    assert_eq!(s.active().unwrap().doc.timeline.as_ref().unwrap().duration, 2);
}

//! Tests that run paint commands, moved from the engine-core `video_cmds` tests: 5 test(s).

use photocraft_doc::LayerContent;
use photocraft_engine::Session;
use photocraft_engine::video_cmds::*;

use photocraft_doc::LayerId;

use serde_json::json;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
    s
}

fn write_png(path: &str, w: u32, h: u32, color: &str) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": color})).unwrap();
    let bytes = photocraft_io::export(&s.active().unwrap().doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes;
    std::fs::write(path, bytes).unwrap();
}

fn tmpdir(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("pc-video-{}-{name}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    d.to_string_lossy().into_owned()
}

#[test]
fn animated_gif_keeps_transparency() {
    // #3061: a transparent document rendered as an animated GIF came out opaque black.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "transparent"})).unwrap();
    s.execute("timeline.create", json!({"duration": 2, "fps": 10})).unwrap();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 4, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let mut gif = Vec::new();
    render_video_with(
        &s,
        &json!({"format": "gif"}),
        |_, bytes| {
            gif = bytes.to_vec();
            Ok(())
        },
        |_, _| true,
    )
    .unwrap();
    // Each frame has a transparent index and is cleared before the next (disposal 2).
    let packed: Vec<u8> = gif.windows(4).filter(|w| w[..3] == [0x21, 0xF9, 0x04]).map(|w| w[3]).collect();
    assert_eq!(packed, [0x09, 0x09]);
    let px = photocraft_compose::flatten(&photocraft_io::import("t.gif", &gif).unwrap().document).into_px();
    assert_eq!(px[0][3], 0.0, "the corner stays transparent");
    assert_eq!(px[3 * 8 + 3], [1.0, 0.0, 0.0, 1.0], "the square stays red");
}

fn layer_px(s: &Session, id: LayerId) -> Vec<f32> {
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Raster(r) => r.pixel(2, 2),
        _ => panic!("not a raster"),
    }
}

/// #1288: a fill on a video frame survives scrubbing away and back, and reaches Render Video.
#[test]
fn edits_to_a_frame_survive_scrubbing_and_render() {
    let mut s = session();
    let r = s.execute("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    s.execute("layer.videoLayers.insertBlankFrame", json!({})).unwrap();
    s.execute("timeline.setFrame", json!({"frame": 0})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("timeline.setFrame", json!({"frame": 1})).unwrap();
    assert!(layer_px(&s, id)[3] < 0.01, "frame 1 stays blank");
    s.execute("timeline.setFrame", json!({"frame": 0})).unwrap();
    let p = layer_px(&s, id);
    assert!(p[0] > 0.99 && p[3] > 0.99, "frame 0 keeps its fill: {p:?}");
    // Duplicate Frame copies the edited frame.
    s.execute("layer.videoLayers.duplicateFrame", json!({})).unwrap();
    s.execute("timeline.setFrame", json!({"frame": 1})).unwrap();
    assert!(layer_px(&s, id)[0] > 0.99);

    // Render Video picks up an edit made at the playhead without scrubbing first.
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    let out = tmpdir("edited");
    s.execute("file.export.renderVideo", json!({"dir": out, "format": "png"})).unwrap();
    let stem = s.active().unwrap().doc.name.rsplit_once('.').map_or(s.active().unwrap().doc.name.clone(), |(a, _)| a.to_string());
    let f1 = std::fs::read(format!("{out}/{stem}_0001.png")).unwrap();
    let doc = photocraft_io::import("f1.png", &f1).unwrap().document;
    let flat = photocraft_compose::flatten_to_surface(&doc, doc.pixel_format(), None);
    let q = flat.pixel(2, 2);
    assert!(q[2] > 0.99 && q[0] < 0.01, "rendered frame 1 is blue: {q:?}");
}

#[test]
fn imported_video_restore_frame_and_all_keep_source_pixels() {
    let dir = tmpdir("original-footage");
    write_png(&format!("{dir}/f0.png"), 8, 8, "#ff0000");
    write_png(&format!("{dir}/f1.png"), 8, 8, "#00ff00");
    let mut s = session();
    let id = LayerId(s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": dir})).unwrap()["layer"].as_u64().unwrap());
    assert_eq!(layer_px(&s, id), vec![1.0, 0.0, 0.0, 1.0]);

    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("layer.videoLayers.restoreFrame", json!({})).unwrap();
    assert_eq!(layer_px(&s, id), vec![1.0, 0.0, 0.0, 1.0], "current frame restores the red original");

    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("timeline.setFrame", json!({"frame": 1})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("layer.videoLayers.restoreAllFrames", json!({})).unwrap();
    assert_eq!(layer_px(&s, id), vec![0.0, 1.0, 0.0, 1.0], "frame 1 restores green");
    s.execute("timeline.setFrame", json!({"frame": 0})).unwrap();
    assert_eq!(layer_px(&s, id), vec![1.0, 0.0, 0.0, 1.0], "frame 0 restores red");
    assert!(s.undo(), "restoration stays in history");
}

#[test]
fn show_altered_video_switches_between_original_and_edited_frames() {
    let dir = tmpdir("show-altered-original");
    write_png(&format!("{dir}/f0.png"), 8, 8, "#ff0000");
    write_png(&format!("{dir}/f1.png"), 8, 8, "#00ff00");
    let mut s = session();
    let id = LayerId(s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": dir})).unwrap()["layer"].as_u64().unwrap());
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    assert_eq!(layer_px(&s, id), vec![0.0, 0.0, 1.0, 1.0]);

    s.execute("layer.videoLayers.showAlteredVideo", json!({})).unwrap();
    assert!(!s.active().unwrap().doc.layer(id).unwrap().video.as_ref().unwrap().show_altered);
    assert_eq!(layer_px(&s, id), vec![1.0, 0.0, 0.0, 1.0], "hidden altered footage shows red original");
    s.execute("timeline.setFrame", json!({"frame": 1})).unwrap();
    assert_eq!(layer_px(&s, id), vec![0.0, 1.0, 0.0, 1.0], "scrubbing while hidden shows green original");
    s.execute("timeline.setFrame", json!({"frame": 0})).unwrap();
    s.execute("layer.videoLayers.showAlteredVideo", json!({})).unwrap();
    assert_eq!(layer_px(&s, id), vec![0.0, 0.0, 1.0, 1.0], "switching back reveals the blue edit");
}

#[test]
fn restoring_blank_video_frames_still_produces_blank_frames() {
    let mut s = session();
    let id = LayerId(s.execute("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("layer.videoLayers.restoreFrame", json!({})).unwrap();
    assert_eq!(layer_px(&s, id)[3], 0.0);
}

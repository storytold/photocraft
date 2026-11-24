//! Tests that run paint commands, moved from the engine-core `video_cmds` tests: 5 test(s), and the Reload Frame tests below them.

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

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const MAGENTA: [f32; 4] = [1.0, 0.0, 1.0, 1.0];
const YELLOW: [f32; 4] = [1.0, 1.0, 0.0, 1.0];

/// Pixel (2, 2) of every frame of the video layer: the altered (shown) stack, or with `original`
/// the footage stack that Restore Frame and Show Altered Video off bring back.
fn stack_px(s: &Session, id: LayerId, original: bool) -> Vec<Vec<f32>> {
    let v = s.active().unwrap().doc.layer(id).unwrap().video.as_ref().unwrap();
    let stack = if original { v.original_frames.as_ref().unwrap() } else { &v.frames };
    stack.iter().map(|f| f.pixel(2, 2)).collect()
}

/// An 8 × 8 image sequence in a new `name` folder, one solid PNG per colour (`f0.png`, `f1.png`,
/// ...), loaded as a video layer. Returns the folder and the layer.
fn footage(s: &mut Session, name: &str, colors: &[&str]) -> (String, LayerId) {
    let dir = tmpdir(name);
    for (i, c) in colors.iter().enumerate() {
        write_png(&format!("{dir}/f{i}.png"), 8, 8, c);
    }
    let r = s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": dir})).unwrap();
    (dir, LayerId(r["layer"].as_u64().unwrap()))
}

/// Reload Frame re-reads only the frame at the playhead, as Photoshop's does: that frame shows the
/// file's new pixels, its edit gone, while the edit on another frame survives and the other frames
/// keep the footage they had. Undo brings the edit and the old footage back. Before, the whole
/// footage was reloaded and the edits on every frame were lost.
#[test]
fn reload_frame_reloads_only_the_current_frame() {
    let mut s = session();
    let (dir, id) = footage(&mut s, "reload-frame", &["#ff0000", "#00ff00", "#0000ff"]);
    s.execute("edit.fill", json!({"color": "#ffff00"})).unwrap();
    s.execute("timeline.setFrame", json!({"frame": 1})).unwrap();
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    // The footage changes on disk.
    for (i, c) in ["#00ffff", "#ff00ff", "#ffffff"].iter().enumerate() {
        write_png(&format!("{dir}/f{i}.png"), 8, 8, c);
    }
    s.execute("layer.videoLayers.reloadFrame", json!({})).unwrap();

    assert_eq!(layer_px(&s, id), MAGENTA, "the current frame shows its new footage");
    assert_eq!(stack_px(&s, id, false), [YELLOW, MAGENTA, BLUE], "frame 0 keeps its edit, frame 2 its old footage");
    assert_eq!(stack_px(&s, id, true), [RED, MAGENTA, BLUE], "only the current frame's footage is reloaded");

    assert!(s.undo());
    assert_eq!(layer_px(&s, id), BLACK, "undo brings the edit back");
    // Scrubbing stores the displayed edit into its frame.
    s.execute("timeline.setFrame", json!({"frame": 0})).unwrap();
    assert_eq!(stack_px(&s, id, false), [YELLOW, BLACK, BLUE]);
    assert_eq!(stack_px(&s, id, true), [RED, GREEN, BLUE]);
}

/// Footage that no longer reaches the frame at the playhead is refused, and nothing changes.
#[test]
fn reload_frame_refuses_footage_without_the_current_frame() {
    let mut s = session();
    let (dir, id) = footage(&mut s, "reload-shorter", &["#ff0000", "#00ff00"]);
    s.execute("timeline.setFrame", json!({"frame": 1})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    std::fs::remove_file(format!("{dir}/f1.png")).unwrap();
    let err = s.execute("layer.videoLayers.reloadFrame", json!({})).unwrap_err().to_string();
    assert!(err.contains("now has 1 frames, so it has no frame 2"), "{err}");
    assert_eq!(layer_px(&s, id), BLUE);
    assert_eq!(stack_px(&s, id, true), [RED, GREEN]);
}

/// A recorded Reload Frame step does what the menu command does: it reloads the frame at the
/// playhead and keeps the edits on the others.
#[test]
fn a_recorded_reload_frame_step_reloads_only_the_current_frame() {
    let mut s = session();
    let (dir, id) = footage(&mut s, "reload-action", &["#ff0000", "#00ff00"]);
    s.execute("edit.fill", json!({"color": "#ffff00"})).unwrap();
    s.execute("timeline.setFrame", json!({"frame": 1})).unwrap();
    s.execute("actions.record", json!({"name": "Reload"})).unwrap();
    s.execute("layer.videoLayers.reloadFrame", json!({})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let steps = s.execute("actions.get", json!({"action": "Reload"})).unwrap()["steps"].clone();
    assert_eq!(steps, json!([["layer.videoLayers.reloadFrame", {}]]));

    for (i, c) in ["#00ffff", "#ff00ff"].iter().enumerate() {
        write_png(&format!("{dir}/f{i}.png"), 8, 8, c);
    }
    let played = s.execute("actions.play", json!({"action": "Reload"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(stack_px(&s, id, false), [YELLOW, MAGENTA]);
    assert_eq!(stack_px(&s, id, true), [RED, MAGENTA]);
}

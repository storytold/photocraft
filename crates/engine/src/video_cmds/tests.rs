use super::*;
use photocraft_doc::LayerId;
use serde_json::json;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
    s
}

fn frames(s: &Session, id: LayerId) -> usize {
    s.active().unwrap().doc.layer(id).unwrap().video.as_ref().unwrap().frames.len()
}

#[test]
fn new_blank_insert_duplicate_delete_frames() {
    let mut s = session();
    let r = s.execute("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert!(s.active().unwrap().doc.layer(id).unwrap().video.is_some());
    assert!(s.active().unwrap().doc.timeline.is_some());
    let start = frames(&s, id);
    s.execute("layer.videoLayers.insertBlankFrame", json!({})).unwrap();
    assert_eq!(frames(&s, id), start + 1);
    s.execute("layer.videoLayers.duplicateFrame", json!({})).unwrap();
    assert_eq!(frames(&s, id), start + 2);
    s.execute("layer.videoLayers.deleteFrame", json!({})).unwrap();
    assert_eq!(frames(&s, id), start + 1);
    assert!(s.active().unwrap().doc.timeline.as_ref().unwrap().duration >= frames(&s, id));
}

#[test]
fn rasterize_drops_the_stack() {
    let mut s = session();
    s.execute("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap();
    let r = s.execute("layer.videoLayers.rasterize", json!({})).unwrap();
    assert_eq!(r["rasterized"], true);
    let id = s.active().unwrap().active_layer.unwrap();
    assert!(s.active().unwrap().doc.layer(id).unwrap().video.is_none());
    assert!(matches!(s.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Raster(_)));
}

#[test]
fn frame_ops_need_a_video_layer() {
    let mut s = session();
    assert!(s.execute("layer.videoLayers.insertBlankFrame", json!({})).is_err());
}

#[test]
fn scrub_keeps_content_a_raster() {
    let mut s = session();
    let r = s.execute("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    s.execute("layer.videoLayers.insertBlankFrame", json!({})).unwrap();
    s.execute("timeline.setFrame", json!({"frame": 1})).unwrap();
    assert!(matches!(s.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Raster(_)));
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
fn new_video_layer_from_image_sequence() {
    let dir = tmpdir("seq");
    write_png(&format!("{dir}/f01.png"), 16, 12, "#ff0000");
    write_png(&format!("{dir}/f02.png"), 16, 12, "#00ff00");
    write_png(&format!("{dir}/f03.png"), 16, 12, "#0000ff");
    let mut s = session();
    let r = s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": dir})).unwrap();
    assert_eq!(r["frames"], 3);
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(frames(&s, id), 3);
    assert!(s.active().unwrap().doc.timeline.as_ref().unwrap().duration >= 3);
}

#[test]
fn frames_to_layers_and_render_video() {
    let dir = tmpdir("ftl");
    write_png(&format!("{dir}/a.png"), 10, 8, "#112233");
    write_png(&format!("{dir}/b.png"), 10, 8, "#445566");
    let mut s = session();
    let r = s.execute("file.import.videoFramesToLayers", json!({"path": dir})).unwrap();
    assert_eq!(r["layers"], 2);
    // Build a 2-frame video layer and render.
    let v = s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": dir})).unwrap();
    assert_eq!(v["frames"], 2);
    let out = tmpdir("render");
    let rr = s.execute("file.export.renderVideo", json!({"dir": out, "format": "png"})).unwrap();
    assert!(rr["frames"].as_u64().unwrap() >= 2);
    assert!(std::path::Path::new(&out).exists());
}

#[test]
fn render_video_to_animated_gif() {
    let dir = tmpdir("gifsrc");
    write_png(&format!("{dir}/a.png"), 10, 8, "#ff0000");
    write_png(&format!("{dir}/b.png"), 10, 8, "#00ff00");
    write_png(&format!("{dir}/c.png"), 10, 8, "#0000ff");
    let mut s = session();
    s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": dir})).unwrap();
    let out = tmpdir("gifout");
    let r = s.execute("file.export.renderVideo", json!({"dir": out, "format": "gif"})).unwrap();
    assert_eq!(r["frames"], 3);
    let gif = std::fs::read(r["file"].as_str().unwrap()).unwrap();
    assert_eq!(&gif[..6], b"GIF89a");
    assert_eq!(gif.iter().filter(|&&b| b == 0x2C).count(), 3); // 3 frames
}

#[test]
fn inspect_reports_video_layer() {
    let dir = tmpdir("vinsp");
    write_png(&format!("{dir}/a.png"), 6, 4, "#ff0000");
    write_png(&format!("{dir}/b.png"), 6, 4, "#00ff00");
    let mut s = session();
    s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": dir})).unwrap();
    let r = s.execute("document.inspect", json!({})).unwrap();
    // Find any layer node carrying a "video" block.
    fn find_video(v: &serde_json::Value) -> Option<serde_json::Value> {
        if let Some(vid) = v.get("video") {
            return Some(vid.clone());
        }
        v.get("children").and_then(|c| c.as_array()).and_then(|a| a.iter().find_map(find_video))
    }
    let layers = r["layers"].as_array().expect("layers array");
    let vid = layers.iter().find_map(find_video).expect("a video layer is reported");
    assert_eq!(vid["frames"], 2);
    assert_eq!(vid["source"]["file"].as_str(), Some(dir.as_str()));
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
    let flat = crate::file_cmds::flattened(&doc, doc.pixel_format());
    let q = flat.pixel(2, 2);
    assert!(q[2] > 0.99 && q[0] < 0.01, "rendered frame 1 is blue: {q:?}");
}

#[test]
fn render_writer_progress_and_cancellation_leave_session_unchanged() {
    let mut s = session();
    s.execute("timeline.create", json!({"duration": 4, "fps": 4})).unwrap();
    let before = s.active().unwrap().doc.clone();
    for format in ["png", "gif"] {
        let mut names = Vec::new();
        let mut progress = Vec::new();
        let r = render_video_with(
            &s,
            &json!({"format":format}),
            |name, bytes| {
                assert!(!bytes.is_empty());
                names.push(name.to_owned());
                Ok(())
            },
            |done, total| {
                progress.push((done, total));
                done < 2
            },
        );
        assert!(r.unwrap_err().to_string().contains("cancelled"));
        assert!(names.len() < 4);
        assert_eq!(progress.first(), Some(&(0, 4)));
        assert_eq!(progress.last(), Some(&(2, 4)));
        assert_eq!(s.active().unwrap().doc.timeline, before.timeline);
    }
}

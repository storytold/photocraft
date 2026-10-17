use photocraft_codecs::{self as codecs, AnimationFrame, ChannelLayout, DecodeOptions, EncodeOptions, Image, SampleType};
use photocraft_engine::Session;
use serde_json::json;

fn fixture() -> Vec<u8> {
    let frames: Vec<_> = [([255, 0, 0, 255], 27), ([0, 0, 255, 128], 91)]
        .into_iter()
        .map(|(c, delay_ms)| AnimationFrame { image: Image::from_raw(4, 3, ChannelLayout::Rgba, SampleType::U8, c.repeat(12)).unwrap(), delay_ms })
        .collect();
    codecs::encode_webp_animation(&frames, 7, &EncodeOptions::default()).unwrap()
}

#[test]
fn byte_open_selects_first_frame_and_save_copy_keeps_hidden_frames() {
    let mut s = Session::new();
    let bytes = fixture();
    photocraft_engine::file_cmds::open_bytes_as(&mut s, "upload.webp", &bytes, None, None).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.active_layer, Some(st.doc.layers[0].id));
    assert_eq!(st.selected_layers(), vec![st.doc.layers[0].id]);
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("animation.webp");
    s.execute("file.saveACopy", json!({"path":path})).unwrap();
    let decoded = codecs::decode_webp_frames(&std::fs::read(&path).unwrap(), &DecodeOptions::default()).unwrap();
    assert_eq!(decoded.loop_count, Some(7));
    assert_eq!(decoded.frames.len(), 2);
    assert_eq!(decoded.frames[1].delay_ms, 91);
    assert_eq!(&decoded.frames[1].image.to_rgba8()[..4], &[0, 0, 255, 128]);
    // Invalid edits must not replace the original file, even when explicitly saving a copy
    // to its current name. Encoding finishes before the atomic writer is called.
    let saved = std::fs::read(&path).unwrap();
    s.edit("Rename", |doc, _| {
        doc.layers[0].name = "No duration".into();
        Ok(())
    })
    .unwrap();
    assert!(s.execute("file.saveACopy", json!({"path":path})).is_err());
    assert_eq!(std::fs::read(path).unwrap(), saved);
}

#[test]
fn frame_import_is_one_history_step_in_the_destinations_color_and_depth() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("frames.webp");
    std::fs::write(&path, fixture()).unwrap();
    for mode in ["rgb", "gray", "cmyk"] {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":4,"height":3,"mode":mode,"depth":depth})).unwrap();
            let before = s.active().unwrap().doc.clone();
            let result = s.execute("file.import.videoFramesToLayers", json!({"path":path})).unwrap();
            assert_eq!(result["layers"], 2);
            let st = s.active().unwrap();
            let imported = &st.doc.layers[before.layers.len()..];
            assert_eq!(imported.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Frame1 (27 ms)", "Frame2 (91 ms)"]);
            assert_eq!(st.active_layer, Some(imported[0].id));
            assert!(imported[0].visible && !imported[1].visible);
            assert!(imported.iter().all(|l| l.surface().unwrap().format() == st.doc.pixel_format()));
            assert_eq!(st.doc.file_encoding.webp_animation_loops, None, "frame import doesn't reinterpret the existing document");
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*s.active().unwrap().doc, *before);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.layers.len(), before.layers.len() + 2);
        }
    }
}

#[test]
fn corrupt_frames_do_not_open_a_document_or_modify_an_import_target() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("broken.webp");
    let mut bytes = fixture();
    bytes.truncate(bytes.len() - 1);
    std::fs::write(&path, &bytes).unwrap();
    let mut s = Session::new();
    assert!(photocraft_engine::file_cmds::open_bytes_as(&mut s, "broken.webp", &bytes, None, None).is_err());
    assert!(s.active().is_none());
    s.execute("file.new", json!({"width":4,"height":3})).unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(s.execute("file.import.videoFramesToLayers", json!({"path":path})).is_err());
    assert_eq!(*s.active().unwrap().doc, *before);
    assert!(s.execute("file.import.videoFramesToLayers", json!({})).is_err());
    assert!(s.execute("file.import.videoFramesToLayers", json!({"path":3})).is_err());
}

#[test]
fn shared_video_footage_loader_takes_every_webp_frame() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("video.webp");
    std::fs::write(&path, fixture()).unwrap();
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":3})).unwrap();
    let result = s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path":path})).unwrap();
    assert_eq!(result["frames"], 2);
}

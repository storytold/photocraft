use super::*;
use photocraft_codecs::web::{GifFrame, WebpFrame, encode_gif_animated, encode_webp_animated};

/// An animated GIF of solid 3×2 frames, each `(colour, delay in centiseconds)`.
fn gif(colours: &[([u8; 3], u16)]) -> Vec<u8> {
    let frames: Vec<GifFrame> = colours.iter().map(|&(c, delay_cs)| GifFrame { indices: vec![0; 6], palette: vec![c], transparent: None, delay_cs }).collect();
    encode_gif_animated(3, 2, &frames, true).unwrap()
}

/// What File › Open's first read of `bytes` called `name` gives when it asks: the first frame.
fn read(name: &str, bytes: &[u8]) -> photocraft_io::ImportResult {
    photocraft_io::import(name, bytes).unwrap()
}

/// `read`, turned into a video layer as the Video choice does.
fn video(name: &str, bytes: &[u8], path: Option<&str>) -> (Document, Vec<String>) {
    let mut r = read(name, bytes);
    open_as_video(&mut r.document, &mut r.warnings, bytes, path, &Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
    (r.document, r.warnings)
}

/// The result of a background open (`start_open` or `start_open_for_user`).
fn finished(s: &mut crate::Session, started: crate::jobs::Started) -> Value {
    match started {
        crate::jobs::Started::Done(v) => v,
        crate::jobs::Started::Job(id) => s.wait_job(id).unwrap(),
    }
}

/// The red channel of the top-left pixel `doc` shows.
fn red(doc: &Document) -> f32 {
    let s = photocraft_compose::flatten_to_surface(doc, doc.pixel_format(), None);
    let mut px = vec![[0.0f32; 4]; 1];
    s.read_rgba_into(photocraft_geom::Rect::new(0, 0, 1, 1), &mut px);
    px[0][0]
}

fn prefs_with(gif: AnimatedOpen, webp: AnimatedOpen) -> Preferences {
    let mut p = Preferences::default();
    p.file_handling.animated_gif = gif;
    p.file_handling.animated_webp = webp;
    p
}

#[test]
fn asking_names_the_format_and_the_frame_count_from_the_first_frames_read() {
    let r = read("a.gif", &gif(&[([255, 0, 0], 5), ([0, 0, 255], 5)]));
    assert_eq!(question("a.gif", &r.warnings).unwrap(), json!({"format": "GIF", "key": "animatedGif", "frames": 2}));
    assert_eq!(preferred(&Preferences::default(), "a.gif"), Some(AnimatedOpen::Ask), "Ask is the default");
    assert_eq!(preferred(&prefs_with(AnimatedOpen::Video, AnimatedOpen::Ask), "a.gif"), Some(AnimatedOpen::Video));
    // A damaged file has more frames than can be counted.
    let uncounted = vec!["only the first frame of the animation was imported".to_string()];
    assert_eq!(question("a.gif", &uncounted).unwrap()["frames"], Value::Null);
}

#[test]
fn stills_and_other_formats_are_not_asked_about() {
    let still = read("still.gif", &gif(&[([255, 0, 0], 5)]));
    assert!(question("still.gif", &still.warnings).is_none());
    // A frame animation saved in a native document opens as it was saved.
    let r = read("a.gif", &gif(&[([255, 0, 0], 5), ([0, 0, 255], 5)]));
    assert!(question("a.pcraft", &r.warnings).is_none());
}

#[test]
fn only_a_frame_animation_reads_a_layer_per_frame() {
    for (choice, read) in [
        (AnimatedOpen::Frames, photocraft_io::AnimationImport::Frames),
        (AnimatedOpen::Video, photocraft_io::AnimationImport::FirstFrame),
        (AnimatedOpen::Ask, photocraft_io::AnimationImport::FirstFrame),
        (AnimatedOpen::FirstFrame, photocraft_io::AnimationImport::FirstFrame),
    ] {
        assert_eq!(import_for(choice), read, "{choice:?}");
    }
    assert_eq!(import_mode(&prefs_with(AnimatedOpen::Ask, AnimatedOpen::Frames), "a.webp"), photocraft_io::AnimationImport::Frames);
    assert_eq!(
        import_mode(&prefs_with(AnimatedOpen::Frames, AnimatedOpen::Ask), "a.webp"),
        photocraft_io::AnimationImport::FirstFrame,
        "the GIF preference doesn't answer for a WebP"
    );
}

#[test]
fn video_is_one_layer_with_every_frame_on_a_timeline() {
    let (doc, warnings) = video("clip.gif", &gif(&[([255, 0, 0], 5), ([0, 0, 255], 5), ([0, 255, 0], 10)]), Some("/tmp/clip.gif"));
    assert!(doc.frame_animation.is_none());
    assert_eq!(doc.layers.len(), 1);
    assert_eq!(doc.mode, ColorMode::Rgb, "a GIF's first frame reads Indexed; its video isn't");
    assert!(doc.color_table.is_none());
    assert!(warnings.is_empty(), "every frame came: {warnings:?}");
    let l = &doc.layers[0];
    assert_eq!(l.name, "clip");
    let v = l.video.as_ref().unwrap();
    assert_eq!(v.frames.len(), 3);
    assert_eq!(v.source, VideoSource::File { path: "/tmp/clip.gif".into() });
    // The most common delay, 50 ms, sets the rate.
    assert_eq!(v.fps, 20.0);
    let t = doc.timeline.as_ref().unwrap();
    assert_eq!((t.duration, t.fps, t.current), (3, 20.0, 0));
    assert!(red(&doc) > 0.99, "the playhead shows the first frame");
}

/// #6898 review: a video layer reads as footage, so the layer budget of a frame animation (at most
/// `MAX_GIF_LAYER_FRAMES` frames) doesn't cut it short.
#[test]
fn a_video_layer_keeps_the_frames_a_frame_animation_leaves_out() {
    let n = photocraft_io::MAX_GIF_LAYER_FRAMES + 5;
    let frames: Vec<GifFrame> = (0..n).map(|i| GifFrame { indices: vec![0], palette: vec![[(i % 251) as u8, 0, 0]], transparent: None, delay_cs: 5 }).collect();
    let bytes = encode_gif_animated(1, 1, &frames, true).unwrap();
    let (doc, warnings) = video("long.gif", &bytes, None);
    assert_eq!(doc.layers[0].video.as_ref().map(|v| v.frames.len()), Some(n));
    assert!(warnings.is_empty(), "{warnings:?}");
    let opts = photocraft_io::ImportOptions { animation: photocraft_io::AnimationImport::Frames, ..Default::default() };
    let layers = photocraft_io::import_with_options("long.gif", &bytes, &opts).unwrap();
    assert_eq!(layers.document.layers.len(), photocraft_io::MAX_GIF_LAYER_FRAMES);
}

/// Past the footage limit a video layer is refused, as a video layer from a file is, not cut short.
#[test]
fn a_video_layer_past_the_footage_limit_is_refused() {
    let _limit = crate::video_cmds::limits::Scoped::max_footage_bytes(1);
    let bytes = gif(&[([255, 0, 0], 5), ([0, 0, 255], 5)]);
    let mut r = read("a.gif", &bytes);
    let e = open_as_video(&mut r.document, &mut r.warnings, &bytes, None, &Default::default(), &photocraft_raster::Interrupt::NONE).unwrap_err();
    assert!(e.to_string().contains("the footage needs more than"), "{e}");
}

#[test]
fn cancelling_stops_a_video_layers_read() {
    let bytes = gif(&[([255, 0, 0], 5), ([0, 0, 255], 5)]);
    let mut r = read("a.gif", &bytes);
    let yes = || true;
    let e = open_as_video(&mut r.document, &mut r.warnings, &bytes, None, &Default::default(), &photocraft_raster::Interrupt::cancel_only(&yes)).unwrap_err();
    assert!(matches!(e, crate::EngineError::Cancelled), "{e}");
}

#[test]
fn a_still_or_native_document_is_not_made_a_video() {
    let bytes = gif(&[([255, 0, 0], 5)]);
    let (doc, _) = video("still.gif", &bytes, None);
    assert!(doc.layers[0].video.is_none() && doc.timeline.is_none());
    let mut other = read("a.gif", &gif(&[([255, 0, 0], 5), ([0, 0, 255], 5)]));
    open_as_video(&mut other.document, &mut Vec::new(), b"not an animation", None, &Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
    assert!(other.document.layers[0].video.is_none());
}

#[test]
fn an_animated_webp_is_asked_about_and_becomes_a_video() {
    let px = |c: [u8; 4]| c.repeat(6);
    let frames = [WebpFrame { rgba: px([255, 0, 0, 255]), delay_ms: 100 }, WebpFrame { rgba: px([0, 0, 255, 255]), delay_ms: 100 }];
    let bytes = encode_webp_animated(3, 2, &frames, 0).unwrap();
    assert_eq!(question("a.webp", &read("a.webp", &bytes).warnings).unwrap()["format"], "WebP");
    let (doc, _) = video("a.webp", &bytes, None);
    assert_eq!(doc.layers[0].video.as_ref().map(|v| (v.frames.len(), v.fps)), Some((2, 10.0)));
}

/// #6898 review: a headless open (scripts, the CLI, MCP) of an animated GIF reads its first frame,
/// as before frame animations, whatever File Handling says, and is never held back to ask.
#[test]
fn a_headless_open_takes_the_first_frame_whatever_file_handling_says() {
    use crate::jobs::OpenSource;
    let bytes = std::sync::Arc::new(gif(&[([255, 0, 0], 5), ([0, 0, 255], 5), ([0, 255, 0], 5)]));
    for choice in ["ask", "frames", "video", "firstFrame"] {
        let mut s = crate::Session::new();
        s.execute("prefs.set", json!({"path": "fileHandling.animatedGif", "value": choice})).unwrap();
        let started = s.start_open("a.gif", OpenSource::Bytes(bytes.clone())).unwrap();
        let v = finished(&mut s, started);
        assert!(v.get("held").is_none(), "{choice}: {v}");
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.layers.len(), 1, "{choice}");
        assert!(doc.frame_animation.is_none() && doc.timeline.is_none(), "{choice}");
        assert!(v["warnings"].to_string().contains("only the first of 3 frames"), "{choice}: {v}");
    }
}

/// File › Open (`start_open_for_user`) reads the frames as File Handling says: Ask holds the file
/// with the question, Frames and Video open it so, First Frame Only reads one frame.
#[test]
fn file_open_follows_file_handling() {
    use crate::jobs::OpenSource;
    let bytes = std::sync::Arc::new(gif(&[([255, 0, 0], 5), ([0, 0, 255], 5)]));
    let open = |choice: &str| {
        let mut s = crate::Session::new();
        s.execute("prefs.set", json!({"path": "fileHandling.animatedGif", "value": choice})).unwrap();
        let started = s.start_open_for_user("a.gif", OpenSource::Bytes(bytes.clone())).unwrap();
        let v = finished(&mut s, started);
        (s, v)
    };
    let (mut s, v) = open("ask");
    assert_eq!(v["animated"], json!({"format": "GIF", "key": "animatedGif", "frames": 2}));
    assert!(s.documents().is_empty(), "held back to ask");
    // #6898 review: asking reads the first frame as a normal open does, and keeps the bytes to
    // read again should the answer want the frames.
    let held = v["held"].as_u64().unwrap();
    assert!(s.take_held_open_bytes(held).is_some());
    let (doc, _) = s.take_held_open(held).unwrap();
    assert_eq!((doc.layers.len(), doc.mode), (1, ColorMode::Indexed));
    assert!(doc.frame_animation.is_none());
    let (s, _) = open("frames");
    assert_eq!(s.active().unwrap().doc.frame_animation.as_ref().map(|a| a.frames.len()), Some(2));
    let (s, _) = open("video");
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 1);
    assert!(doc.layers[0].video.is_some() && doc.timeline.is_some());
    let (s, _) = open("firstFrame");
    let doc = &s.active().unwrap().doc;
    assert_eq!((doc.layers.len(), doc.mode), (1, ColorMode::Indexed));
    assert!(doc.frame_animation.is_none());
}

/// The answer to the prompt reads the file again, in that shape and without asking again.
#[test]
fn an_answered_open_reads_the_shape_it_was_told() {
    use crate::jobs::OpenSource;
    let bytes = std::sync::Arc::new(gif(&[([255, 0, 0], 5), ([0, 0, 255], 5)]));
    for (choice, layers, is_video) in [(AnimatedOpen::Frames, 2, false), (AnimatedOpen::Video, 1, true)] {
        let mut s = crate::Session::new();
        let started = s.start_open_answered("a.gif", OpenSource::Bytes(bytes.clone()), choice, true).unwrap();
        let v = finished(&mut s, started);
        assert!(v.get("held").is_none(), "{choice:?}: {v}");
        let doc = &s.active().unwrap().doc;
        assert_eq!((doc.layers.len(), doc.layers[0].video.is_some()), (layers, is_video), "{choice:?}");
    }
}

/// A video layer read from a path keeps that path to reload from.
#[test]
fn a_video_layer_opened_from_a_path_reloads_from_it() {
    use crate::jobs::OpenSource;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.gif");
    std::fs::write(&path, gif(&[([255, 0, 0], 5), ([0, 0, 255], 5)])).unwrap();
    let path = path.to_string_lossy().into_owned();
    let mut s = crate::Session::new();
    let started = s.start_open_answered("a.gif", OpenSource::Path(path.clone()), AnimatedOpen::Video, true).unwrap();
    finished(&mut s, started);
    let v = s.active().unwrap().doc.layers[0].video.clone().unwrap();
    assert_eq!((v.frames.len(), v.source), (2, VideoSource::File { path }));
}

/// An automation caller's `animation`: the first frame when it doesn't say, frames or a video
/// layer when it asks; never a prompt.
#[test]
fn an_automation_caller_names_the_shape_or_gets_the_first_frame() {
    assert_eq!(requested(None).unwrap(), AnimatedOpen::FirstFrame);
    assert_eq!(requested(Some("firstFrame")).unwrap(), AnimatedOpen::FirstFrame);
    assert_eq!(requested(Some("frames")).unwrap(), AnimatedOpen::Frames);
    assert_eq!(requested(Some("video")).unwrap(), AnimatedOpen::Video);
    for bad in ["ask", "all", ""] {
        assert!(requested(Some(bad)).unwrap_err().to_string().contains("firstFrame, frames or video"), "{bad}");
    }
}

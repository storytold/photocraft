use super::*;
use photocraft_codecs::web::{GifFrame, WebpFrame, encode_gif_animated, encode_webp_animated};

/// An animated GIF of solid 3×2 frames, each `(colour, delay in centiseconds)`.
fn gif(colours: &[([u8; 3], u16)]) -> Vec<u8> {
    let frames: Vec<GifFrame> = colours.iter().map(|&(c, delay_cs)| GifFrame { indices: vec![0; 6], palette: vec![c], transparent: None, delay_cs }).collect();
    encode_gif_animated(3, 2, &frames, true).unwrap()
}

/// The document File › Open reads from `bytes` called `name` when File Handling says Ask: every frame.
fn read(name: &str, bytes: &[u8]) -> Document {
    let opts = photocraft_io::ImportOptions { animation: photocraft_io::AnimationImport::Frames, ..Default::default() };
    photocraft_io::import_with_options(name, bytes, &opts).unwrap().document
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
fn ask_is_the_default_and_names_the_format() {
    let doc = read("a.gif", &gif(&[([255, 0, 0], 5), ([0, 0, 255], 5)]));
    let q = question(&Preferences::default(), "a.gif", &doc).unwrap();
    assert_eq!(q, json!({"format": "GIF", "key": "animatedGif", "frames": 2}));
    // A chosen preference answers instead.
    assert!(question(&prefs_with(AnimatedOpen::Video, AnimatedOpen::Ask), "a.gif", &doc).is_none());
}

#[test]
fn stills_and_other_formats_are_not_asked_about() {
    let still = read("still.gif", &gif(&[([255, 0, 0], 5)]));
    assert!(question(&Preferences::default(), "still.gif", &still).is_none());
    // A frame animation saved in a native document opens as it was saved.
    let doc = read("a.gif", &gif(&[([255, 0, 0], 5), ([0, 0, 255], 5)]));
    assert!(question(&Preferences::default(), "a.pcraft", &doc).is_none());
}

#[test]
fn frames_keep_the_frame_animation() {
    let mut doc = read("a.gif", &gif(&[([255, 0, 0], 5), ([0, 0, 255], 5)]));
    apply(&mut doc, AnimatedOpen::Frames, None).unwrap();
    assert_eq!(doc.layers.len(), 2);
    assert_eq!(doc.frame_animation.as_ref().map(|a| a.frames.len()), Some(2));
}

#[test]
fn first_frame_only_keeps_the_bottom_layer() {
    let mut doc = read("a.gif", &gif(&[([255, 0, 0], 5), ([0, 0, 255], 5), ([0, 255, 0], 5)]));
    apply(&mut doc, AnimatedOpen::FirstFrame, None).unwrap();
    assert!(doc.frame_animation.is_none());
    assert_eq!(doc.layers.iter().map(|l| (l.name.as_str(), l.visible)).collect::<Vec<_>>(), [("Layer 0", true)]);
    assert!(red(&doc) > 0.99, "the first frame is red");
}

#[test]
fn video_is_one_layer_with_every_frame_on_a_timeline() {
    let mut doc = read("clip.gif", &gif(&[([255, 0, 0], 5), ([0, 0, 255], 5), ([0, 255, 0], 10)]));
    apply(&mut doc, AnimatedOpen::Video, Some("/tmp/clip.gif")).unwrap();
    assert!(doc.frame_animation.is_none());
    assert_eq!(doc.layers.len(), 1);
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

#[test]
fn an_animated_webp_is_asked_about_and_becomes_any_of_the_three() {
    let px = |c: [u8; 4]| c.repeat(6);
    let frames = [WebpFrame { rgba: px([255, 0, 0, 255]), delay_ms: 100 }, WebpFrame { rgba: px([0, 0, 255, 255]), delay_ms: 100 }];
    let bytes = encode_webp_animated(3, 2, &frames, 0).unwrap();
    let doc = read("a.webp", &bytes);
    assert_eq!(doc.frame_animation.as_ref().map(|a| a.frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>()), Some(vec![100, 100]));
    assert_eq!(question(&Preferences::default(), "a.webp", &doc).unwrap()["format"], "WebP");
    let mut video = doc.clone();
    apply_preferred(&prefs_with(AnimatedOpen::Ask, AnimatedOpen::Video), "a.webp", &mut video, None).unwrap();
    assert_eq!(video.layers[0].video.as_ref().map(|v| (v.frames.len(), v.fps)), Some((2, 10.0)));
    let mut first = doc.clone();
    apply_preferred(&prefs_with(AnimatedOpen::Video, AnimatedOpen::FirstFrame), "a.webp", &mut first, None).unwrap();
    assert_eq!(first.layers.len(), 1);
    // The GIF preference doesn't answer for a WebP.
    let mut asked = doc.clone();
    apply_preferred(&prefs_with(AnimatedOpen::Video, AnimatedOpen::Ask), "a.webp", &mut asked, None).unwrap();
    assert_eq!(asked.layers.len(), 2);
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
    let (s, v) = open("ask");
    assert_eq!(v["animated"], json!({"format": "GIF", "key": "animatedGif", "frames": 2}));
    assert!(s.documents().is_empty(), "held back to ask");
    let (s, _) = open("frames");
    assert_eq!(s.active().unwrap().doc.frame_animation.as_ref().map(|a| a.frames.len()), Some(2));
    let (s, _) = open("video");
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 1);
    assert!(doc.layers[0].video.is_some() && doc.timeline.is_some());
    let (s, _) = open("firstFrame");
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 1);
    assert!(doc.frame_animation.is_none());
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
    assert_eq!(import_for(AnimatedOpen::FirstFrame), photocraft_io::AnimationImport::FirstFrame);
    assert_eq!(import_for(AnimatedOpen::Video), photocraft_io::AnimationImport::Frames);
    // First Frame Only on a document read as one frame changes nothing.
    let mut still = photocraft_io::import("a.gif", &gif(&[([255, 0, 0], 5), ([0, 0, 255], 5)])).unwrap().document;
    let before = still.layers[0].name.clone();
    apply(&mut still, AnimatedOpen::FirstFrame, None).unwrap();
    assert_eq!(still.layers[0].name, before);
}

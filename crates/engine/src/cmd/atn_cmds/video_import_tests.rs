//! #10723: File › Import › Video Frames to Layers, recorded as `importVideoToLayers`
//! (`docs/atn.md`, "Recorded with source files").

use super::*;

const IMPORT: &str = "file.import.videoFramesToLayers";

/// `Usng` as the reference application writes a path (`txtu`, little-endian lengths, NUL),
/// built here independently of the converter.
fn txtu(path: &str) -> D {
    let units: Vec<u16> = path.encode_utf16().chain([0]).collect();
    let mut b = b"txtu".to_vec();
    b.extend_from_slice(&u32::try_from(12 + units.len() * 2).unwrap().to_le_bytes());
    b.extend_from_slice(&u32::try_from(units.len()).unwrap().to_le_bytes());
    b.extend(units.iter().flat_map(|u| u.to_le_bytes()));
    D::Path(b)
}

/// The measured step: the video, then Make Frame Animation.
fn recorded(path: &str, animation: bool) -> Descriptor {
    Descriptor::new("null").with("Usng", txtu(path)).with("makeAnimation", D::Boolean(animation))
}

/// Load the `importVideoToLayers` step holding `d` as action "A" through `actions.load`.
fn load(d: Descriptor) -> (Session, Value) {
    let item = AtnItem { event: AtnEvent::name("importVideoToLayers"), descriptor: Some(d), ..Default::default() };
    let file = AtnFile {
        name: UnicodeString::new_nul("Import"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items: vec![item], ..Default::default() }],
        ..Default::default()
    };
    let mut s = Session::new();
    let r = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&atn::write(&file).unwrap())})).unwrap();
    (s, r)
}

/// The first step of action "A" as `actions.save` writes it: its event and descriptor bytes.
fn saved(s: &mut Session) -> (String, Option<Vec<u8>>) {
    let r = s.execute("actions.save", json!({"action": "A"})).unwrap();
    assert_eq!(r["skipped"], json!([]), "{r}");
    let item = atn::parse(&photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap().actions.remove(0).items.remove(0);
    (item.event.id().to_string(), item.descriptor.as_ref().map(Descriptor::to_bytes))
}

/// A recorded step with Make Frame Animation off loads with its path and saves back byte for
/// byte, under its own event.
#[test]
fn a_recorded_step_loads_and_saves_back_unchanged() {
    for path in ["C:\\Footage\\clip.mov", "/Users/me/Movies/Clip 2.mp4", "D:/Ünïcode/vidéo.avi"] {
        let d = recorded(path, false);
        let (mut s, r) = load(d.clone());
        assert_eq!(r["unsupported"], json!([]), "{r}");
        assert_eq!(s.actions.list[0].steps[0], (IMPORT.to_string(), json!({"path": path})));
        assert_eq!(saved(&mut s), ("importVideoToLayers".to_string(), Some(d.to_bytes())));
    }
}

/// A loaded step imports every frame of the footage as a layer of its own, in order, as one
/// undoable edit.
#[test]
fn a_loaded_step_imports_the_frames_and_undoes() {
    let dir = std::env::temp_dir().join(format!("pc-atn-video-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // A folder of stills is footage to the command, one frame per file in name order.
    for (name, w, h, color) in [("f01.png", 12, 8, "#ff0000"), ("f02.png", 6, 10, "#0000ff")] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": w, "height": h, "background": color})).unwrap();
        let bytes = photocraft_io::export(&s.active().unwrap().doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes;
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    let (mut s, _) = load(recorded(&dir.to_string_lossy(), false));
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    let before = s.active().unwrap().doc.layers.len();
    let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
    assert_eq!(played["ran"], 1, "{played}");
    let frames: Vec<(String, u32, u32, f32)> = s.active().unwrap().doc.layers[before..]
        .iter()
        .map(|l| {
            let surf = l.surface().unwrap();
            let b = surf.content_bounds();
            let px = surf.read_region(photocraft_geom::Rect::new(b.x0, b.y0, b.x0 + 1, b.y0 + 1));
            (l.name.clone(), b.width(), b.height(), px[0])
        })
        .collect();
    assert_eq!(frames, vec![("Frame 1".to_string(), 12, 8, 1.0), ("Frame 2".to_string(), 6, 10, 0.0)]);
    s.execute("history.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), before);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A step recorded in PhotoCraft writes the measured keys, and loads back to the same step.
#[test]
fn a_native_step_saves_the_measured_form() {
    let p = json!({"path": "C:/Footage/clip.mov"});
    let d = export_item(IMPORT, &p, "S").and_then(|item| item.descriptor).unwrap();
    assert_eq!(d.items, recorded("C:/Footage/clip.mov", false).items);
    let (s, _) = load(d);
    assert_eq!(s.actions.list[0].steps[0], (IMPORT.to_string(), p));
}

/// A native step the event can't hold is refused: no file, an empty path or one with a NUL, a
/// path that isn't text, or another setting.
#[test]
fn settings_the_event_cannot_hold_are_refused() {
    for p in [
        json!({}),
        json!({"path": ""}),
        json!({"path": "C:/a\u{0}b.mov"}),
        json!({"path": 3}),
        json!({"path": "C:/Footage/clip.mov", "makeAnimation": true}),
        json!({"path": "C:/Footage/clip.mov", "frameSkip": 2}),
    ] {
        assert!(export_item(IMPORT, &p, "S").is_none(), "{p}");
    }
}

/// Any other form keeps the reference step: Make Frame Animation on (PhotoCraft has no frame
/// animation), a frame limit (PhotoCraft imports every frame), a missing, repeated or extra key,
/// another key order, or values of another type.
#[test]
fn other_forms_keep_the_photoshop_step() {
    let path = "C:\\Footage\\clip.mov";
    let mut reordered = recorded(path, false);
    reordered.items.reverse();
    let mut no_animation_key = recorded(path, false);
    no_animation_key.items.pop();
    let forms = [
        recorded(path, true),
        recorded(path, false).with("frameSkip", D::Integer(2)),
        recorded(path, true).with("frameSkip", D::Integer(3)),
        no_animation_key,
        Descriptor::new("null").with("makeAnimation", D::Boolean(false)),
        reordered,
        recorded(path, false).with("makeAnimation", D::Boolean(false)),
        recorded(path, false).with("Usng", txtu("C:\\Other.mov")),
        Descriptor::new("null").with("Usng", D::Text(UnicodeString::new_nul(path))).with("makeAnimation", D::Boolean(false)),
        Descriptor::new("null").with("Usng", txtu(path)).with("makeAnimation", D::Integer(0)),
        Descriptor::new("null").with("Usng", txtu("")).with("makeAnimation", D::Boolean(false)),
    ];
    for d in forms {
        let (mut s, r) = load(d.clone());
        assert_eq!(r["unsupported"].as_array().map(Vec::len), Some(1), "{d:?}");
        assert_eq!(s.actions.list[0].steps[0].0, PHOTOSHOP_STEP, "{d:?}");
        assert_eq!(saved(&mut s).1, Some(d.to_bytes()));
    }
}

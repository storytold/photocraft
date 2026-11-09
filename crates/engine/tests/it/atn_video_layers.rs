//! #10723: the remaining Video Layers events `docs/atn.md` measures. Restore Frame, Restore All
//! Frames, Reload Frame and Show / Hide Altered Video take no settings; New Video Layer from File
//! (`addVideoLayer`) and Replace Footage (`replaceFootage`) hold the video file in `null`.
use super::atn_video_frames::{file, fixture, item, load, pixels, saved};
use photocraft_doc::{LayerContent, LayerId};
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use photocraft_psd::{
    atn::AtnEvent,
    descriptor::{Descriptor, Value as D},
};
use serde_json::{Value, json};

const SHOW_ALTERED: &str = "layer.videoLayers.showAlteredVideo";

/// Each settings-free event with the command and params it loads as.
fn keyless() -> [(&'static str, &'static str, Value); 5] {
    [
        ("restoreFrame", "layer.videoLayers.restoreFrame", json!({})),
        ("restoreAllFrames", "layer.videoLayers.restoreAllFrames", json!({})),
        ("reloadFrame", "layer.videoLayers.reloadFrame", json!({})),
        ("showAlteredVideo", SHOW_ALTERED, json!({"show": true})),
        ("hideAlteredVideo", SHOW_ALTERED, json!({"show": false})),
    ]
}

/// A `Pth ` value as the reference writes it: `txtu`, total length, UTF-16 unit count, the
/// NUL-terminated units, all little-endian. Built here independently of the converter.
fn path(text: &str) -> D {
    let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let mut b = b"txtu".to_vec();
    b.extend_from_slice(&(12 + 2 * units.len() as u32).to_le_bytes());
    b.extend_from_slice(&(units.len() as u32).to_le_bytes());
    units.iter().for_each(|u| b.extend_from_slice(&u.to_le_bytes()));
    D::Path(b)
}

fn footage(text: &str) -> Descriptor {
    Descriptor::new("null").with("null", path(text))
}

/// Run action 0 and require every step to have played.
fn play(s: &mut Session) {
    let report = s.execute("actions.play", json!({"action": 0})).unwrap();
    assert!(report.get("failed").is_none(), "{report}");
}

fn show_altered(s: &Session, id: LayerId) -> bool {
    s.active().unwrap().doc.layer(id).unwrap().video.as_ref().unwrap().show_altered
}

fn display(s: &Session, id: LayerId) -> Vec<f32> {
    let LayerContent::Raster(display) = &s.active().unwrap().doc.layer(id).unwrap().content else { panic!("video display") };
    display.pixel(0, 0)
}

#[test]
/// The step loads as its command, keeps its flags, and saves back with or without a descriptor,
/// as it was recorded.
fn keyless_events_round_trip_including_an_absent_descriptor() {
    for (event, command, params) in keyless() {
        for descriptor in [None, Some(Descriptor::default())] {
            let mut source = item(event, descriptor.clone());
            source.enabled = false;
            source.with_dialog = true;
            let imported = atn_cmds::import(&file(source)).unwrap();
            assert!(imported.unsupported.is_empty(), "{event}");
            let action = &imported.actions[0];
            assert_eq!(action.steps[0].0, command);
            let mut expected = params.clone();
            if descriptor.is_none() {
                expected["_atnVideoFrameNoDescriptor"] = json!(true);
            }
            assert_eq!(action.steps[0].1, expected, "{event}");
            let back = saved(action);
            assert_eq!((back.event, back.descriptor, back.enabled, back.with_dialog), (AtnEvent::name(event), descriptor, false, true));
        }
    }
}

#[test]
/// Restore Frame puts the current frame's original back, Restore All Frames every frame's; both
/// are one undoable edit, at each depth.
fn restore_frame_and_restore_all_frames_play_with_undo() {
    for depth in [8, 16, 32] {
        for event in ["restoreFrame", "restoreAllFrames"] {
            let (mut s, id) = fixture(depth);
            let (before, originals) = (pixels(&s, id, false), pixels(&s, id, true));
            let expected = if event == "restoreFrame" { vec![before[0].clone(), originals[1].clone(), before[2].clone()] } else { originals.clone() };
            load(&mut s, item(event, Some(Descriptor::default())));
            play(&mut s);
            assert_eq!(pixels(&s, id, false), expected, "{depth}/{event}");
            assert_eq!(pixels(&s, id, true), originals, "the original footage is untouched: {depth}/{event}");
            assert_eq!(display(&s, id), expected[1], "{depth}/{event}");
            s.execute("history.undo", json!({})).unwrap();
            assert_eq!(pixels(&s, id, false), before, "{depth}/{event}");
        }
    }
}

#[test]
/// Show and Hide Altered Video set the state the event names, whatever the layer showed before:
/// a second Hide keeps the original footage, where the menu's toggle would show the edits again.
fn show_and_hide_altered_video_set_the_state() {
    let (mut s, id) = fixture(8);
    let (frames, originals) = (pixels(&s, id, false), pixels(&s, id, true));
    for (event, shown) in [("hideAlteredVideo", false), ("hideAlteredVideo", false), ("showAlteredVideo", true), ("showAlteredVideo", true)] {
        load(&mut s, item(event, None));
        play(&mut s);
        assert_eq!(show_altered(&s, id), shown, "{event}");
        assert_eq!(display(&s, id), if shown { frames[1].clone() } else { originals[1].clone() }, "{event}");
    }
}

/// The menu item without `show` still toggles; `show` sets; anything but a boolean is refused.
#[test]
fn show_altered_video_toggles_without_show_and_refuses_other_values() {
    let (mut s, id) = fixture(8);
    s.execute(SHOW_ALTERED, json!({})).unwrap();
    assert!(!show_altered(&s, id));
    s.execute(SHOW_ALTERED, json!({"show": null})).unwrap();
    assert!(show_altered(&s, id));
    s.execute(SHOW_ALTERED, json!({"show": true})).unwrap();
    assert!(show_altered(&s, id));
    for bad in [json!("false"), json!(0)] {
        assert!(s.execute(SHOW_ALTERED, json!({"show": bad})).is_err(), "{bad}");
        assert!(show_altered(&s, id), "a refused value changes nothing");
    }
}

#[test]
/// A native toggle can't be either event, and nothing beside `show` is written.
fn a_toggle_or_extra_settings_refuse_export() {
    for params in [json!({}), json!({"show": null}), json!({"show": "true"}), json!({"show": true, "layer": 3}), json!({"show": false, "frame": 1})] {
        let action = Action::new("Invalid", vec![(SHOW_ALTERED.into(), params.clone())]);
        assert!(atn_cmds::export("Video", &[&action], &[], false).is_err(), "{params}");
    }
    for (command, params) in [
        ("layer.videoLayers.restoreFrame", json!({"frame": 2})),
        ("layer.videoLayers.restoreAllFrames", json!({"layer": 3})),
        ("layer.videoLayers.reloadFrame", json!({"_atnVideoFrameNoDescriptor": false})),
        ("layer.videoLayers.newVideoLayerFromFile", json!({"path": "/v/clip", "fps": 24})),
        ("layer.videoLayers.newVideoLayerFromFile", json!({"path": ""})),
        ("layer.videoLayers.replaceFootage", json!({})),
    ] {
        let action = Action::new("Invalid", vec![(command.into(), params.clone())]);
        assert!(atn_cmds::export("Video", &[&action], &[], false).is_err(), "{command} {params}");
    }
}

#[test]
/// The footage events load as their commands with the path, and save back byte for byte.
fn footage_events_round_trip() {
    for (event, command) in [("addVideoLayer", "layer.videoLayers.newVideoLayerFromFile"), ("replaceFootage", "layer.videoLayers.replaceFootage")] {
        let d = footage("C:\\Footage\\clip ö.mp4");
        let imported = atn_cmds::import(&file(item(event, Some(d.clone())))).unwrap();
        assert!(imported.unsupported.is_empty(), "{event}");
        assert_eq!(imported.actions[0].steps[0], (command.to_string(), json!({"path": "C:\\Footage\\clip ö.mp4"})));
        let back = saved(&imported.actions[0]);
        assert_eq!((back.event, back.descriptor.map(|d| d.to_bytes())), (AtnEvent::name(event), Some(d.to_bytes())));
    }
}

#[test]
/// Alias bytes, a path that wouldn't save back the same, another class or an extra key keep the
/// step as recorded.
fn other_footage_forms_stay_kept_steps() {
    let D::Path(mut unterminated) = path("/v/clip") else { panic!("a path value") };
    unterminated.truncate(unterminated.len() - 2);
    for event in ["addVideoLayer", "replaceFootage"] {
        for d in [
            Descriptor::new("null").with("null", D::Alias(b"alis-bytes".to_vec())),
            Descriptor::new("null").with("null", D::Path(unterminated.clone())),
            Descriptor::new("null").with("null", path("")),
            Descriptor::new("other").with("null", path("/v/clip")),
            footage("/v/clip").with("frameSkip", D::Integer(2)),
            Descriptor::default(),
        ] {
            let imported = atn_cmds::import(&file(item(event, Some(d.clone())))).unwrap();
            assert_eq!(imported.unsupported.len(), 1, "{event} {d:?}");
            assert_eq!(imported.actions[0].steps[0].0, "actions.photoshopStep");
            assert_eq!(saved(&imported.actions[0]).descriptor, Some(d));
        }
    }
}

/// A folder of one-pixel PNG frames, one per colour, removed with the returned guard.
fn frames(colours: &[&str]) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    for (i, colour) in colours.iter().enumerate() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 1, "height": 1, "background": colour})).unwrap();
        let bytes = photocraft_io::export(&s.active().unwrap().doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes;
        std::fs::write(dir.path().join(format!("f{i}.png")), bytes).unwrap();
    }
    let path = dir.path().to_string_lossy().into_owned();
    (dir, path)
}

#[test]
/// Recorded footage steps play: a new video layer from the recorded folder, its footage replaced
/// from another, and Reload Frame bringing back the file's frame after an edit.
fn footage_and_reload_steps_play() {
    let (_first, first) = frames(&["#ff0000", "#00ff00"]);
    let (_second, second) = frames(&["#0000ff", "#ffffff"]);
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1, "height": 1})).unwrap();
    load(&mut s, item("addVideoLayer", Some(footage(&first))));
    play(&mut s);
    let id = s.active().unwrap().active_layer.unwrap();
    assert_eq!(pixels(&s, id, false), [vec![1.0, 0.0, 0.0, 1.0], vec![0.0, 1.0, 0.0, 1.0]]);

    load(&mut s, item("replaceFootage", Some(footage(&second))));
    play(&mut s);
    assert_eq!(pixels(&s, id, false), [vec![0.0, 0.0, 1.0, 1.0], vec![1.0, 1.0, 1.0, 1.0]]);

    // The fill lands on the displayed frame, which the next video edit stores into the stack.
    s.execute("edit.fill", json!({"color": "#ffff00"})).unwrap();
    assert_eq!(display(&s, id), [1.0, 1.0, 0.0, 1.0]);
    load(&mut s, item("reloadFrame", Some(Descriptor::default())));
    play(&mut s);
    assert_eq!(pixels(&s, id, false), [vec![0.0, 0.0, 1.0, 1.0], vec![1.0, 1.0, 1.0, 1.0]], "the frames come back from the file");
    assert_eq!(display(&s, id), [0.0, 0.0, 1.0, 1.0]);
}

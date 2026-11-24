//! Movie files (MP4, MOV, MKV, WebM) as video-layer footage (#9040, #5494).
// Movie files are read only by the desktop build.
#![cfg(not(target_arch = "wasm32"))]

use super::*;
use photocraft_doc::LayerId;
use photocraft_io::movie::testgen::{Container, h264, h264_from, level};
use serde_json::json;

const W: u32 = 40;
const H: u32 = 24;

/// A synthetic H.264 movie of `frames` frames at 25 fps, written to a fresh temp file.
fn movie(name: &str, container: Container, frames: usize) -> String {
    let ext = if container == Container::Mp4 { "mp4" } else { "mkv" };
    let p = std::env::temp_dir().join(format!("pc-movie-cmds-{}-{name}.{ext}", std::process::id()));
    std::fs::write(&p, h264(container, W, H, frames, 25).unwrap()).unwrap();
    p.to_string_lossy().into_owned()
}

fn session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": W, "height": H})).unwrap();
    s
}

fn grey_of(s: &Session, id: LayerId) -> f32 {
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Raster(r) => r.pixel((W / 2) as i32, (H / 2) as i32)[0],
        _ => panic!("a video layer shows raster content"),
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 4.0 / 255.0
}

#[test]
fn a_movie_becomes_a_video_layer_that_decodes_as_it_plays() {
    for container in [Container::Mp4, Container::Mkv] {
        let path = movie(&format!("layer-{container:?}"), container, 20);
        let mut s = session();
        let r = s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": path})).unwrap();
        let id = LayerId(r["layer"].as_u64().unwrap());
        assert_eq!(r["frames"], 20);
        let t = s.active().unwrap().doc.timeline.clone().unwrap();
        assert_eq!(t.duration, 20, "{container:?}: the timeline grows to the movie");
        assert!((t.fps - 25.0).abs() < 0.01, "{container:?}: the timeline takes the movie's rate");
        let v = s.active().unwrap().doc.layer(id).unwrap().video.clone().unwrap();
        assert!(v.frames.iter().all(|f| f.tile_count() == 0), "{container:?}: no frame is held");
        for frame in [0, 13, 7, 19] {
            s.execute("timeline.setFrame", json!({"frame": frame})).unwrap();
            assert!(close(grey_of(&s, id), level(frame)), "{container:?}: frame {frame}: {} vs {}", grey_of(&s, id), level(frame));
        }
        let _ = std::fs::remove_file(&path);
    }
}

#[test]
fn video_frames_to_layers_takes_a_range_and_every_nth_frame() {
    let path = movie("frames-to-layers", Container::Mp4, 30);
    let mut s = session();
    let before = s.active().unwrap().doc.layers.len();
    let r = s.execute("file.import.videoFramesToLayers", json!({"path": path, "from": 4, "to": 16, "every": 4})).unwrap();
    assert_eq!(r["layers"], 4, "frames 4, 8, 12 and 16");
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.layers.len(), before + 4);
    let top = doc.layers.last().unwrap();
    match &top.content {
        LayerContent::Raster(r) => assert!(close(r.pixel(5, 5)[0], level(16)), "the last layer is frame 16"),
        _ => panic!("frames become pixel layers"),
    }
    for bad in [json!({"from": 30}), json!({"from": 9, "to": 3}), json!({"every": 0}), json!({"to": -1})] {
        let mut p = bad.clone();
        p["path"] = json!(path);
        assert!(s.execute("file.import.videoFramesToLayers", p).is_err(), "{bad} is refused");
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn file_open_makes_a_video_document_the_size_of_the_movie() {
    let path = movie("open", Container::Mkv, 12);
    let mut s = crate::session();
    match s.start_open(&path, crate::jobs::OpenSource::Path(path.clone())).unwrap() {
        crate::jobs::Started::Job(id) => {
            s.wait_job(id).unwrap();
        }
        crate::jobs::Started::Done(_) => {}
    }
    let d = s.active().unwrap();
    assert_eq!((d.doc.size.width, d.doc.size.height), (W, H));
    assert_eq!(d.doc.timeline.as_ref().unwrap().duration, 12);
    assert!(d.source_read_only, "Save doesn't write over the movie");
    let id = d.doc.layers.last().unwrap().id;
    assert!(close(grey_of(&s, id), level(0)), "the first frame shows");
    s.execute("timeline.setFrame", json!({"frame": 11})).unwrap();
    assert!(close(grey_of(&s, id), level(11)));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn replace_footage_and_reload_frame_read_the_new_file() {
    let a = movie("replace-a", Container::Mp4, 10);
    let b = movie("replace-b", Container::Mp4, 16);
    let mut s = session();
    let r = s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": a})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    s.execute("layer.videoLayers.replaceFootage", json!({"path": b})).unwrap();
    let v = s.active().unwrap().doc.layer(id).unwrap().video.clone().unwrap();
    assert_eq!(v.footage.as_ref().unwrap().link.path, b);
    assert_eq!(v.len(), 16);

    // The file changes on disk: Reload Frame shows the new pictures.
    s.execute("timeline.setFrame", json!({"frame": 2})).unwrap();
    assert!(close(grey_of(&s, id), level(2)));
    std::fs::write(&b, h264_from(Container::Mp4, W, H, 16, 25, 5).unwrap()).unwrap();
    s.execute("layer.videoLayers.reloadFrame", json!({})).unwrap();
    assert!(close(grey_of(&s, id), level(7)), "frame 2 of the new file");

    // A file that isn't a movie is refused, naming it, and the layer keeps its footage.
    let junk = std::env::temp_dir().join(format!("pc-movie-cmds-{}-junk.mp4", std::process::id()));
    std::fs::write(&junk, b"no movie here").unwrap();
    let junk = junk.to_string_lossy().into_owned();
    let err = s.execute("layer.videoLayers.replaceFootage", json!({"path": junk})).unwrap_err().to_string();
    assert!(err.contains("junk"), "{err}");
    assert_eq!(s.active().unwrap().doc.layer(id).unwrap().video.as_ref().unwrap().footage.as_ref().unwrap().link.path, b);
    for p in [a, b, junk] {
        let _ = std::fs::remove_file(p);
    }
}

/// Taking frames out of a movie isn't held to the length a video layer may have: a movie longer
/// than a timeline may be still gives its first frame as a layer, while a video layer of it is
/// refused.
#[test]
fn frames_can_be_taken_from_a_movie_longer_than_a_video_layer_may_be() {
    let path = std::env::temp_dir().join(format!("pc-movie-cmds-{}-endless.mp4", std::process::id())).to_string_lossy().into_owned();
    let frames = crate::timeline_cmds::MAX_DURATION + 1;
    crate::footage::register(&path, Box::new(crate::footage::testing::Synthetic::new(W, H, frames, 25.0)));
    let mut s = session();
    let r = s.execute("file.import.videoFramesToLayers", json!({"path": path, "from": 0, "to": 0})).unwrap();
    assert_eq!(r["layers"], 1);
    let err = s.execute("layer.videoLayers.newVideoLayerFromFile", json!({"path": path})).unwrap_err().to_string();
    assert!(err.contains("limited to"), "{err}");
    crate::footage::forget(&path);
}

/// A movie whose first frame can't be decoded is refused by File › Open with the reason, not
/// opened as an empty layer.
#[test]
fn a_movie_whose_first_frame_fails_does_not_open() {
    let path = std::env::temp_dir().join(format!("pc-movie-cmds-{}-first-damaged.mp4", std::process::id())).to_string_lossy().into_owned();
    crate::footage::register(
        &path,
        Box::new(crate::footage::testing::Synthetic { fail_at: Some(0), ..crate::footage::testing::Synthetic::new(W, H, 3, 25.0) }),
    );
    let err = crate::footage::movie_document(&path).map(|_| ()).unwrap_err();
    assert!(err.contains("damaged"), "{err}");
    crate::footage::forget(&path);
}
